---
title: Repository pattern — port + pub(crate) adapter + factory function
description: How Reeve structures persistence (and other "side effects") within a bounded-context crate. A public trait defines the port; a pub(crate) struct implements it for SQLite; a pub factory returns an Arc<dyn Trait>. Callers never name the concrete type.
tags: [frameworks, rust, ddd, hexagonal, repository, port-adapter]
---

# Repository pattern (port + pub(crate) adapter + factory)

Every Reeve context that needs persistence (and many that need other side effects: git, docker, scaffolding) uses the same three-piece pattern: a public trait, a `pub(crate)` struct, and a `pub` factory function. This document is the canonical reference for that pattern.

## The pattern in one file

`reeve-card/src/repo.rs` is the cleanest example:

```rust
use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{CardReference, Column};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::domain::Card;

// 1. The port — public trait, async, Send + Sync.
#[async_trait]
pub trait CardRepo: Send + Sync {
    async fn list(&self) -> Result<Vec<Card>>;
    async fn get(&self, id: &str) -> Result<Option<Card>>;
    async fn insert(&self, card: &Card) -> Result<()>;
    async fn update_column(&self, id: &str, col: Column, now: i64) -> Result<Option<Card>>;
    async fn for_household(&self, name: &str) -> Result<Vec<CardReference>>;
}

// 2. The adapter — pub(crate), so external crates literally cannot name it.
pub(crate) struct SqliteCardRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCardRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl CardRepo for SqliteCardRepo {
    async fn list(&self) -> Result<Vec<Card>> {
        // ... spawn_blocking + blocking_lock pattern — see frameworks/rust/async-over-sync-rusqlite
    }
    // ...
}

// 3. The factory — the only public path to obtain a working adapter.
pub fn make_card_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardRepo> {
    Arc::new(SqliteCardRepo::new(conn))
}

// 4. Schema init lives next to the adapter — same file, same concerns.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cards (...);
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // In-memory SQLite tests against the trait through the factory.
}
```

## Why the three pieces

### Why a trait (port)?

- **Testability** — handlers and downstream consumers depend on `&dyn CardRepo`, not on `SqliteCardRepo`. Tests pass a mock. Production passes the real adapter via the factory.
- **Substitutability** — swap SQLite for Postgres or for a no-op in tests. The trait is the contract.
- **Discoverability** — the trait is the public API surface for the aggregate's persistence. Reading the trait tells you what the aggregate can read and write. The adapter is implementation detail.

### Why `pub(crate)` on the struct?

This is the critical move. `pub(crate)` makes the type **unnameable outside its defining crate**. `reeve-workspace` literally cannot write `let r: reeve_card::SqliteCardRepo = ...;` — the type doesn't exist in its namespace.

The only thing other crates can do is obtain a `Arc<dyn CardRepo>` from the factory. That means callers MUST program against the interface, not the implementation. The compiler enforces this; no code review needed.

Why not `pub`? Because `pub` would mean external code could:

- Hold the concrete type and skip the trait — couples consumers to the SQL impl.
- Construct one bypassing `make_card_repo` — bypasses any future "validation at construction" logic the factory might add.
- Match on it in downcast scenarios — leaks the impl into consumer logic.

`pub(crate)` is the strongest available enforcement of "use the trait, not the impl" in Rust.

### Why a factory function returning `Arc<dyn Trait>`?

- **Hides the concrete type from consumers** entirely — they don't even see the name.
- **Provides a future hook** — if construction grows (validation, config, pooling, layering), it goes inside the factory without changing call sites.
- **`Arc<dyn Trait>` is the canonical shareable handle** — works across `tokio::spawn`, across `clone`, across `&dyn` borrows. No lifetime gymnastics.

### Why schema init in the same file?

The aggregate owns its tables. Putting `init_schema` next to the adapter makes the SQL DDL traceable from the trait, and keeps the per-context schema-init function discoverable from `reeved/src/app/lifecycle.rs::migrate`.

## How the lib.rs facade exposes this

`reeve-card/src/lib.rs`:

```rust
mod domain;
mod handler;
mod repo;
mod transitions;

pub use domain::{Card, CardError};
pub use handler::{handle_ls, handle_mv, handle_mv_self, handle_new, handle_status_self};
pub use repo::{CardRepo, init_schema, make_card_repo};   // ← trait + init + factory; NOT the struct
pub use transitions::{HouseholdLookup, WorkspaceLifecycle, on_column_change};
```

`SqliteCardRepo` is not re-exported. The only way for `reeved`'s `main.rs` to construct a `CardRepo` is:

```rust
let cards: Arc<dyn reeve_card::CardRepo> = reeve_card::make_card_repo(conn.clone());
```

And the only way for `reeve-workspace`'s handler to use one is:

```rust
pub async fn handle_attach(cards: &dyn reeve_card::CardRepo, ...) -> Result<...> { ... }
```

## Variations

### Single-impl, two-trait-views

`reeve-household/src/repo.rs` implements both `HouseholdRepo` (its own port) and `reeve_card::HouseholdLookup` (a cross-aggregate port) on the same `pub(crate) SqliteHouseholdRepo`. The factory returns both Arcs over one concrete:

```rust
pub fn make_household_components(
    conn: Arc<Mutex<Connection>>,
) -> (Arc<dyn HouseholdRepo>, Arc<dyn reeve_card::HouseholdLookup>) {
    let concrete = Arc::new(SqliteHouseholdRepo::new(conn));
    (concrete.clone(), concrete)
}
```

Both pointers share the same backing data; the daemon stores both in DaemonState.

### Non-repo adapters (Git, Docker, Scaffold)

The same pattern applies to other side-effecting components:

- `reeve-workspace::git::GitOps` (trait) + `pub(crate) RealGitOps` (struct) + `make_git_ops()` (factory)
- `reeve-workspace::container::ContainerOps` + `pub(crate) DockerOps` + `make_container_ops()`
- `reeve-household::scaffold::HouseholdScaffold` + `pub(crate) RealHouseholdScaffold` + `make_scaffold()`

In each case, the consumer (a handler, the lifecycle impl, the fetch worker) takes `&dyn Trait`, never the concrete.

### Testing the trait directly

Tests in the same file construct the adapter directly via `pub(crate) fn new(...)`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<dyn CardRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_card_repo(Arc::new(Mutex::new(conn)))
    }

    #[tokio::test]
    async fn insert_then_list() {
        let repo = fresh();
        repo.insert(&sample("a")).await.unwrap();
        let cards = repo.list().await.unwrap();
        assert_eq!(cards.len(), 1);
    }
}
```

The test pattern: `Connection::open_in_memory()` → `init_schema` → `make_card_repo(...)` → exercise via the trait. Each test owns its own connection so they parallelise cleanly.

## When to deviate

The pattern is rigid by design, but two situations warrant deviation:

1. **The aggregate has no persistence.** If the trait would have zero implementors and zero methods, don't write one. (We haven't hit this case yet.)
2. **A second non-SQLite implementation actually exists.** When we eventually add a Postgres backend, in-memory cache, or test fixtures crate, we'll need multiple structs implementing the same trait. The factory function will gain a config parameter selecting between them. Today we ship one impl per port; the multi-impl case is hypothetical.

## See also

- [[general/architecture]] for the broader rule this pattern serves ("adapters are pub(crate); access via factory").
- [[frameworks/rust/inverted-dependency-ports]] for the *cross-crate* port pattern (where the trait lives in a different crate than the impl).
- [[frameworks/rust/async-over-sync-rusqlite]] for the `spawn_blocking + blocking_lock` pattern that lives inside every Sqlite*Repo adapter.
- [[general/testing]] for how this pattern interacts with the test layers.
