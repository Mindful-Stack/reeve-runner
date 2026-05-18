# Domain-Driven Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure `crates/reeved/` into six bounded-context crates with compiler-enforced isolation, while preserving behaviour and keeping `cargo test --workspace` green at every commit.

**Architecture:** Vertical-slice folder layout per context (`card/`, `workspace/`, `household/`); inverted-dependency ports defined by the consumer (`reeve-card`) and implemented by suppliers (`reeve-workspace`, `reeve-household`); `pub(crate)` adapters exposed only via factory functions returning `Arc<dyn Trait>`. Cards stay in `protocol::Card` until Stage 5, when the γ pattern (twin domain+wire types with `From`/`TryFrom`) lands.

**Tech Stack:** Rust 2024, tokio, rusqlite, async-trait, anyhow, thiserror, serde, tracing.

**Spec:** `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md`

**Important deviation from spec stage order:** The spec lists "Stage 2: reeve-household." But `HouseholdLookup` is defined by `reeve-card` (inverted-dep), so `reeve-household` cannot be carved before `reeve-card` exists. **This plan carves in dependency order: card → household → workspace.** Spec design unchanged; just the execution sequence.

---

## File-structure overview

After refactor, the workspace layout is:

```
crates/
├── reeve-protocol/         (unchanged structure; one field rename in Stage 6)
├── reeve-card/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          (facade: pub use + factory functions)
│       ├── domain.rs       (Card; Stage 5 promotes to typed entity)
│       ├── repo.rs         (CardRepo trait + pub(crate) SqliteCardRepo + factory + init_schema)
│       ├── transitions.rs  (on_column_change + WorkspaceLifecycle + HouseholdLookup traits)
│       └── handler.rs      (handle_ls / new / mv / mv_self / status_self)
├── reeve-workspace/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          (facade)
│       ├── domain.rs       (Workspace + Container entities)
│       ├── repo.rs         (WorkspaceRepo + pub(crate) SqliteWorkspaceRepo + factory + init_schema)
│       ├── lifecycle.rs    (LiveWorkspaceLifecycle — impls reeve_card::WorkspaceLifecycle)
│       ├── git.rs          (GitOps + pub(crate) RealGitOps + factory)
│       ├── clones.rs       (clone_all_for_card workflow)
│       ├── container.rs    (ContainerOps + pub(crate) DockerOps + factory + reconcile)
│       ├── fetch.rs        (background worker)
│       └── handler.rs      (handle_attach / handle_push)
├── reeve-household/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          (facade)
│       ├── domain.rs       (Household entity)
│       ├── manifest.rs     (Manifest + RepoEntry + validation)
│       ├── repo.rs         (HouseholdRepo + pub(crate) SqliteHouseholdRepo + impl HouseholdLookup + factory + init_schema)
│       ├── scaffold.rs     (HouseholdScaffold trait + pub(crate) RealHouseholdScaffold + factory)
│       └── handler.rs      (handle_household_add/new/ls/rm/show)
├── reeved/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         (composition root — thin)
│       ├── dispatch.rs     (RPC method → context handler routing)
│       ├── app/
│       │   ├── mod.rs
│       │   ├── state.rs    (DaemonState struct)
│       │   └── lifecycle.rs (paths, db_path, migrate orchestrator)
│       └── transport/
│           ├── mod.rs
│           ├── server.rs   (was reeved/src/server.rs)
│           └── listener.rs (was reeved/src/listeners.rs)
└── reeve/                  (unchanged)
```

The `reeved/src/` files that disappear: `server.rs`, `listeners.rs`, `handlers.rs`, `db.rs`, `transitions.rs`, `effects.rs`, `household.rs`, `manifest.rs`, `clones.rs`, `container.rs`, `fetch.rs`, `lifecycle.rs` (relocated to `app/lifecycle.rs`).

---

## Stage 0 — Crate skeletons

Goal: scaffolding only. New crates exist, compile, and are members of the workspace. No behaviour moved yet.

### Task 0.1: Create empty context crates

**Files:**
- Create: `crates/reeve-card/Cargo.toml`
- Create: `crates/reeve-card/src/lib.rs`
- Create: `crates/reeve-workspace/Cargo.toml`
- Create: `crates/reeve-workspace/src/lib.rs`
- Create: `crates/reeve-household/Cargo.toml`
- Create: `crates/reeve-household/src/lib.rs`
- Modify: `Cargo.toml` (root, add to `workspace.members`)

- [ ] **Step 1: Create `crates/reeve-card/Cargo.toml`**

```toml
[package]
name = "reeve-card"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

[dependencies]
reeve-protocol = { path = "../reeve-protocol" }
anyhow = { workspace = true }
async-trait = "0.1"
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
rusqlite = { version = "0.32", features = ["bundled"] }
uuid = { version = "1.10", features = ["v4"] }
```

- [ ] **Step 2: Create `crates/reeve-card/src/lib.rs`**

```rust
//! Card aggregate: kanban metadata + column-transition policy.
//!
//! Scaffolding only at Stage 0; populated in Stage 2.
```

- [ ] **Step 3: Create `crates/reeve-workspace/Cargo.toml`**

```toml
[package]
name = "reeve-workspace"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

[dependencies]
reeve-protocol = { path = "../reeve-protocol" }
reeve-card = { path = "../reeve-card" }
anyhow = { workspace = true }
async-trait = "0.1"
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
rusqlite = { version = "0.32", features = ["bundled"] }
```

- [ ] **Step 4: Create `crates/reeve-workspace/src/lib.rs`**

```rust
//! Workspace aggregate: per-card materialization (clones + container + listener).
//!
//! Scaffolding only at Stage 0; populated in Stage 4.
```

- [ ] **Step 5: Create `crates/reeve-household/Cargo.toml`**

```toml
[package]
name = "reeve-household"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

[dependencies]
reeve-protocol = { path = "../reeve-protocol" }
reeve-card = { path = "../reeve-card" }
anyhow = { workspace = true }
async-trait = "0.1"
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
rusqlite = { version = "0.32", features = ["bundled"] }
```

- [ ] **Step 6: Create `crates/reeve-household/src/lib.rs`**

```rust
//! Household aggregate: registered project bundle.
//!
//! Scaffolding only at Stage 0; populated in Stage 3.
```

- [ ] **Step 7: Add the three crates to root `Cargo.toml`**

Open `Cargo.toml` (root). Find the `[workspace]` section:

```toml
[workspace]
resolver = "2"
members = ["crates/reeve-protocol", "crates/reeved", "crates/reeve", "tests"]
```

Replace with:

```toml
[workspace]
resolver = "2"
members = [
    "crates/reeve-protocol",
    "crates/reeve-card",
    "crates/reeve-workspace",
    "crates/reeve-household",
    "crates/reeved",
    "crates/reeve",
    "tests",
]
```

- [ ] **Step 8: Build the workspace**

Run: `cargo build --workspace`
Expected: build succeeds; new crates produce small dylib outputs.

- [ ] **Step 9: Run all tests**

Run: `cargo test --workspace`
Expected: same pass count as before (new crates have no tests yet).

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml crates/reeve-card crates/reeve-workspace crates/reeve-household
git commit -m "$(cat <<'EOF'
refactor(0.1): scaffold reeve-card, reeve-workspace, reeve-household crates

Stage 0 of the domain-driven refactor: empty crate skeletons added
to the workspace. No behaviour moved yet.
EOF
)"
```

### Task 0.2: Create reeved/src/app/ and reeved/src/transport/ sub-folders

**Files:**
- Create: `crates/reeved/src/app/mod.rs`
- Create: `crates/reeved/src/transport/mod.rs`
- Modify: `crates/reeved/src/main.rs:1-11` (add module declarations)

- [ ] **Step 1: Create `crates/reeved/src/app/mod.rs`**

```rust
//! Application-level wiring (composition root + cross-cutting helpers).
//!
//! Populated in Stage 7.
```

- [ ] **Step 2: Create `crates/reeved/src/transport/mod.rs`**

```rust
//! RPC transport: Unix-socket server + per-card listener registry.
//!
//! Populated in Stage 1.
```

- [ ] **Step 3: Add `mod app;` and `mod transport;` to reeved/src/main.rs**

Open `crates/reeved/src/main.rs`. After the existing module declarations (around lines 1–11), add:

```rust
mod app;
mod transport;
```

- [ ] **Step 4: Build**

Run: `cargo build --workspace`
Expected: succeeds with two "unused module" warnings — that's OK; they go away as we populate.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/app crates/reeved/src/transport crates/reeved/src/main.rs
git commit -m "$(cat <<'EOF'
refactor(0.2): add reeved/src/app/ and reeved/src/transport/ sub-folders

Empty sub-modules ready for Stage 1 (transport relocation) and
Stage 7 (composition root rewrite).
EOF
)"
```

---

## Stage 1 — Move transport into `reeved/src/transport/`

Goal: relocate the two transport files into the new sub-folder. Pure file move + import updates. No logic changes.

### Task 1.1: Move `server.rs` to `transport/server.rs`

**Files:**
- Move: `crates/reeved/src/server.rs` → `crates/reeved/src/transport/server.rs`
- Modify: `crates/reeved/src/main.rs` (remove `mod server;`)
- Modify: `crates/reeved/src/transport/mod.rs` (add `pub mod server;`)
- Modify: any file with `use crate::server::*` (likely `main.rs`, `handlers.rs`, `effects.rs`, `listeners.rs`)

- [ ] **Step 1: Move the file**

```bash
git mv crates/reeved/src/server.rs crates/reeved/src/transport/server.rs
```

- [ ] **Step 2: Update `crates/reeved/src/transport/mod.rs`**

Replace contents with:

```rust
//! RPC transport: Unix-socket server + per-card listener registry.

pub mod server;
```

- [ ] **Step 3: Remove `mod server;` from `crates/reeved/src/main.rs`**

Delete the line `mod server;` near the top of `main.rs`.

- [ ] **Step 4: Update `use crate::server::*` imports across reeved**

Run: `git grep -n 'crate::server' crates/reeved/src/`

For each match, replace `crate::server` with `crate::transport::server`.

- [ ] **Step 5: Build**

Run: `cargo build --workspace`
Expected: succeeds (or fails with import errors you fix and re-run).

- [ ] **Step 6: Test**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(1.1): move server.rs into reeved/src/transport/

File relocation only; no logic changes. Imports updated across reeved.
EOF
)"
```

### Task 1.2: Move `listeners.rs` to `transport/listener.rs` (note rename: plural → singular)

**Files:**
- Move: `crates/reeved/src/listeners.rs` → `crates/reeved/src/transport/listener.rs`
- Modify: `crates/reeved/src/main.rs`
- Modify: `crates/reeved/src/transport/mod.rs`
- Modify: any file with `use crate::listeners::*`

- [ ] **Step 1: Move the file**

```bash
git mv crates/reeved/src/listeners.rs crates/reeved/src/transport/listener.rs
```

- [ ] **Step 2: Update `crates/reeved/src/transport/mod.rs`**

Replace contents with:

```rust
//! RPC transport: Unix-socket server + per-card listener registry.

pub mod server;
pub mod listener;
```

- [ ] **Step 3: Remove `mod listeners;` from `crates/reeved/src/main.rs`**

Delete the line `mod listeners;`.

- [ ] **Step 4: Update imports**

Run: `git grep -n 'crate::listeners' crates/reeved/src/`

For each match, replace `crate::listeners` with `crate::transport::listener`.

- [ ] **Step 5: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(1.2): move listeners.rs into reeved/src/transport/listener.rs

File relocation + plural→singular rename (one file, one listener type).
No logic changes; imports updated across reeved.
EOF
)"
```

---

## Stage 2 — Carve `reeve-card`

Goal: extract Card aggregate (kanban policy, repo, handlers) into its own crate. `reeve-card` defines the two cross-aggregate ports (`WorkspaceLifecycle`, `HouseholdLookup`). Stub impls in `reeved` keep the daemon functional; real impls land in Stages 3 and 4.

Approach: build up `reeve-card` alongside the old code first (additive), then cut over by deleting the old handlers/db code and wiring the stubs. Tests stay green at every commit.

### Task 2.1: Define `transitions.rs` with the two ports + on_column_change

**Files:**
- Create: `crates/reeve-card/src/transitions.rs`
- Modify: `crates/reeve-card/src/lib.rs`

- [ ] **Step 1: Create `crates/reeve-card/src/transitions.rs`**

```rust
//! Kanban transition policy + ports the Card aggregate needs from
//! other aggregates. Pure decision logic — no infra deps.

use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{Card, Column};

/// Port the Card aggregate uses to drive workspace state.
/// Implemented in `reeve-workspace`.
#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

/// Port the Card aggregate uses to check household existence before
/// a transition that needs one. Implemented in `reeve-household`.
#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
}

/// The kanban transition matrix. Decides which `WorkspaceLifecycle`
/// method (if any) to invoke when a card's column changes.
pub async fn on_column_change(
    old: Column,
    new: Column,
    card: &Card,
    workspace: &dyn WorkspaceLifecycle,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    match (old, new) {
        (_, Column::Implementing) => workspace.ensure_running(card).await,
        (_, Column::Testing | Column::Review) => Ok(()),
        (_, Column::Done) => workspace.ensure_stopped(&card.id).await,
        (
            Column::Implementing | Column::Testing | Column::Review,
            Column::Backlog | Column::Planning,
        ) => workspace.ensure_stopped(&card.id).await,
        (_, Column::Backlog | Column::Planning) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockFx {
        running: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl WorkspaceLifecycle for MockFx {
        async fn ensure_running(&self, card: &Card) -> Result<()> {
            self.running.lock().await.push(card.id.clone());
            Ok(())
        }
        async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
            self.stopped.lock().await.push(card_id.into());
            Ok(())
        }
    }

    fn sample(id: &str, col: Column) -> Card {
        Card {
            id: id.into(),
            household: "h".into(),
            brief: "b".into(),
            column: col,
            created_at: 0,
            updated_at: 0,
            needs_heavy: false,
            parent_id: None,
        }
    }

    async fn run_one(old: Column, new: Column) -> Arc<MockFx> {
        let fx = Arc::new(MockFx::default());
        let card = sample("c1", new);
        on_column_change(old, new, &card, &*fx).await.unwrap();
        fx
    }

    #[tokio::test]
    async fn backlog_to_implementing_spawns() {
        let fx = run_one(Column::Backlog, Column::Implementing).await;
        assert_eq!(*fx.running.lock().await, vec!["c1".to_string()]);
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_done_tears_down() {
        let fx = run_one(Column::Implementing, Column::Done).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn implementing_to_review_is_noop() {
        let fx = run_one(Column::Implementing, Column::Review).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn same_column_is_noop() {
        let fx = run_one(Column::Implementing, Column::Implementing).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_backlog_defensively_tears_down() {
        let fx = run_one(Column::Implementing, Column::Backlog).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn backlog_to_planning_is_noop() {
        let fx = run_one(Column::Backlog, Column::Planning).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }
}
```

- [ ] **Step 2: Update `crates/reeve-card/src/lib.rs`**

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod transitions;

pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
```

- [ ] **Step 3: Build and test**

```bash
cargo build --workspace
cargo test -p reeve-card
```

Expected: 6 tests pass in `reeve-card`.

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-card
git commit -m "$(cat <<'EOF'
refactor(2.1): reeve-card defines transitions policy + cross-aggregate ports

WorkspaceLifecycle and HouseholdLookup traits live here (inverted-dep:
the consumer defines what it needs). on_column_change is pure; tests
verify the kanban matrix with a MockFx.
EOF
)"
```

### Task 2.2: Define `CardRepo` trait, adapter, factory, and `init_schema`

**Files:**
- Create: `crates/reeve-card/src/repo.rs`
- Modify: `crates/reeve-card/src/lib.rs`

- [ ] **Step 1: Create `crates/reeve-card/src/repo.rs`**

```rust
//! Persistence port + SQLite adapter for the Card aggregate.

use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::{Card, CardReference, Column};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait CardRepo: Send + Sync {
    async fn list(&self) -> Result<Vec<Card>>;
    async fn get(&self, id: &str) -> Result<Option<Card>>;
    async fn insert(&self, card: &Card) -> Result<()>;
    async fn update_column(
        &self,
        id: &str,
        col: Column,
        now: i64,
    ) -> Result<Option<Card>>;
    async fn for_household(&self, name: &str) -> Result<Vec<CardReference>>;
}

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
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<Card>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([], row_to_card)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?
    }

    async fn get(&self, id: &str) -> Result<Option<Card>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards WHERE id = ?1",
            )?;
            let card = stmt.query_row(params![id], row_to_card).optional()?;
            Ok(card)
        })
        .await?
    }

    async fn insert(&self, card: &Card) -> Result<()> {
        let conn = self.conn.clone();
        let card = card.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO cards (id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    card.id, card.household, card.brief,
                    card.column.as_wire_str(), card.created_at, card.updated_at,
                    card.needs_heavy as i64, card.parent_id,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    async fn update_column(
        &self,
        id: &str,
        col: Column,
        now: i64,
    ) -> Result<Option<Card>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
            let conn = conn.blocking_lock();
            let updated = conn.execute(
                "UPDATE cards SET column_ = ?1, updated_at = ?2 WHERE id = ?3",
                params![col.as_wire_str(), now, id],
            )?;
            if updated == 0 {
                return Ok(None);
            }
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards WHERE id = ?1",
            )?;
            Ok(stmt.query_row(params![id], row_to_card).optional()?)
        })
        .await?
    }

    async fn for_household(&self, name: &str) -> Result<Vec<CardReference>> {
        let conn = self.conn.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<CardReference>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, brief, column_ FROM cards WHERE household = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map([name], |r| {
                Ok(CardReference {
                    id: r.get::<_, String>(0)?,
                    brief: r.get::<_, String>(1)?,
                    column: r.get::<_, String>(2)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?
    }
}

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let column_str: String = row.get(3)?;
    let column = Column::from_wire_str(&column_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(InvalidColumn(column_str.clone())),
        )
    })?;
    Ok(Card {
        id: row.get(0)?,
        household: row.get(1)?,
        brief: row.get(2)?,
        column,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        needs_heavy: row.get::<_, i64>(6)? != 0,
        parent_id: row.get(7)?,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("invalid column value: {0}")]
struct InvalidColumn(String);

/// Construct the SQLite-backed CardRepo. Only `reeved` should call this.
pub fn make_card_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardRepo> {
    Arc::new(SqliteCardRepo::new(conn))
}

/// Initialise the cards table. Called by `reeved/src/app/lifecycle.rs::migrate`.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cards (
            id          TEXT PRIMARY KEY,
            household   TEXT NOT NULL,
            brief       TEXT NOT NULL,
            column_     TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            needs_heavy INTEGER NOT NULL DEFAULT 0,
            parent_id   TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);",
    )
    .context("init_schema for cards")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> Card {
        Card {
            id: id.into(),
            household: "h".into(),
            brief: "test".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        }
    }

    fn fresh_repo() -> Arc<dyn CardRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_card_repo(Arc::new(Mutex::new(conn)))
    }

    #[tokio::test]
    async fn insert_then_list() {
        let repo = fresh_repo();
        repo.insert(&sample("a")).await.unwrap();
        let cards = repo.list().await.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "a");
    }

    #[tokio::test]
    async fn update_column_returns_updated() {
        let repo = fresh_repo();
        repo.insert(&sample("a")).await.unwrap();
        let updated = repo
            .update_column("a", Column::Implementing, 2000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.column, Column::Implementing);
        assert_eq!(updated.updated_at, 2000);
    }

    #[tokio::test]
    async fn update_missing_returns_none() {
        let repo = fresh_repo();
        let result = repo
            .update_column("missing", Column::Done, 2000)
            .await
            .unwrap();
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Update `crates/reeve-card/src/lib.rs`**

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod repo;
mod transitions;

pub use repo::{init_schema, make_card_repo, CardRepo};
pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
```

- [ ] **Step 3: Build and test**

```bash
cargo build --workspace
cargo test -p reeve-card
```

Expected: 9 tests pass (6 transitions + 3 repo).

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-card
git commit -m "$(cat <<'EOF'
refactor(2.2): reeve-card defines CardRepo trait + SqliteCardRepo + factory

SqliteCardRepo is pub(crate); only obtainable via make_card_repo().
init_schema exported for reeved's migrate orchestrator. Tests use
in-memory SQLite to exercise the adapter end-to-end.
EOF
)"
```

### Task 2.3: Create `domain.rs` (re-export only, until Stage 5)

**Files:**
- Create: `crates/reeve-card/src/domain.rs`
- Modify: `crates/reeve-card/src/lib.rs`

- [ ] **Step 1: Create `crates/reeve-card/src/domain.rs`**

```rust
//! Card domain entity.
//!
//! Stage 2: just re-exports the protocol DTO. Stage 5 introduces the
//! typed entity with private fields + invariants and converts at
//! handler/repo boundaries.

pub use reeve_protocol::Card;
```

- [ ] **Step 2: Add `mod domain; pub use domain::Card;` to `crates/reeve-card/src/lib.rs`**

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod repo;
mod transitions;

pub use domain::Card;
pub use repo::{init_schema, make_card_repo, CardRepo};
pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
```

- [ ] **Step 3: Build**

```bash
cargo build --workspace
```

Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-card
git commit -m "$(cat <<'EOF'
refactor(2.3): reeve-card exposes Card from domain.rs (re-export for now)

Stage 5 will replace the re-export with a typed domain entity.
EOF
)"
```

### Task 2.4: Move card handlers into `reeve-card/src/handler.rs`

**Files:**
- Create: `crates/reeve-card/src/handler.rs`
- Modify: `crates/reeve-card/src/lib.rs`

The handlers depend on `CardRepo`, `WorkspaceLifecycle`, `HouseholdLookup` traits — all in this crate. Bodies are adapted from today's `crates/reeved/src/handlers.rs`.

- [ ] **Step 1: Create `crates/reeve-card/src/handler.rs`**

```rust
//! RPC handlers for the Card aggregate.

use crate::repo::CardRepo;
use crate::transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
use anyhow::Result;
use reeve_protocol::{
    Card, Column, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult,
    NewParams, NewResult, RpcError, StatusSelfResult,
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

fn not_found(id: &str) -> RpcError {
    RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {id}"),
    }
}

pub async fn handle_ls(cards: &dyn CardRepo) -> std::result::Result<Value, RpcError> {
    let cards = cards.list().await.map_err(internal)?;
    serde_json::to_value(LsResult { cards }).map_err(internal)
}

pub async fn handle_new(
    cards: &dyn CardRepo,
    params: NewParams,
) -> std::result::Result<Value, RpcError> {
    let now = now_secs();
    let card = Card {
        id: uuid::Uuid::new_v4().to_string(),
        household: params.household,
        brief: params.brief,
        column: Column::Backlog,
        created_at: now,
        updated_at: now,
        needs_heavy: params.needs_heavy,
        parent_id: None,
    };
    cards.insert(&card).await.map_err(internal)?;
    serde_json::to_value(NewResult { card }).map_err(internal)
}

pub async fn handle_mv(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    workspace: &dyn WorkspaceLifecycle,
    params: MvParams,
) -> std::result::Result<Value, RpcError> {
    let existing = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;
    let old_column = existing.column;

    if params.column == Column::Implementing
        && !households.exists(&existing.household).await.map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }

    let now = now_secs();
    let card = cards
        .update_column(&params.id, params.column, now)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;

    on_column_change(old_column, params.column, &card, workspace)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;

    serde_json::to_value(MvResult { card }).map_err(internal)
}

pub async fn handle_mv_self(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    workspace: &dyn WorkspaceLifecycle,
    card_id: String,
    params: MvSelfParams,
) -> std::result::Result<Value, RpcError> {
    let existing = cards
        .get(&card_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;
    let old_column = existing.column;

    if params.column == Column::Implementing
        && !households.exists(&existing.household).await.map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }

    let now = now_secs();
    let card = cards
        .update_column(&card_id, params.column, now)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;

    on_column_change(old_column, params.column, &card, workspace)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;

    serde_json::to_value(MvSelfResult { card }).map_err(internal)
}

pub async fn handle_status_self(
    cards: &dyn CardRepo,
    workspace_status: Option<reeve_protocol::ContainerStatus>,
    card_id: String,
) -> std::result::Result<Value, RpcError> {
    let card = cards
        .get(&card_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;
    serde_json::to_value(StatusSelfResult {
        card,
        container_status: workspace_status,
    })
    .map_err(internal)
}
```

Note: `handle_status_self` takes a pre-fetched `Option<ContainerStatus>` (from the Workspace aggregate) rather than reaching into a `WorkspaceRepo` from card-side code. The dispatch layer in `reeved` will resolve it before calling here. This keeps Card free of any WorkspaceRepo dependency.

- [ ] **Step 2: Update `crates/reeve-card/src/lib.rs`**

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod handler;
mod repo;
mod transitions;

pub use domain::Card;
pub use handler::{
    handle_ls, handle_mv, handle_mv_self, handle_new, handle_status_self,
};
pub use repo::{init_schema, make_card_repo, CardRepo};
pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
```

- [ ] **Step 3: Build**

```bash
cargo build --workspace
```

Expected: succeeds. (No tests for handlers yet — covered by reeved's integration tests after cutover.)

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-card
git commit -m "$(cat <<'EOF'
refactor(2.4): card handlers moved to reeve-card/src/handler.rs

handle_ls / new / mv / mv_self / status_self take port references as
arguments. handle_status_self receives a pre-resolved container status
from the dispatch layer (avoids card-side WorkspaceRepo dep).
EOF
)"
```

### Task 2.5: Cut over reeved to use reeve-card with stub port impls

**Files:**
- Modify: `crates/reeved/Cargo.toml` (add reeve-card dep)
- Create: `crates/reeved/src/stubs.rs` (temporary stub impls of WorkspaceLifecycle + HouseholdLookup)
- Modify: `crates/reeved/src/main.rs` (wire CardRepo + stubs + add reeve-card dispatch)
- Modify: `crates/reeved/src/handlers.rs` (remove card handlers; keep household + workspace handlers)
- Modify: `crates/reeved/src/db.rs` (remove card-specific functions; keep household + container)

- [ ] **Step 1: Add `reeve-card` dep to `crates/reeved/Cargo.toml`**

In the `[dependencies]` section, add:

```toml
reeve-card = { path = "../reeve-card" }
```

- [ ] **Step 2: Create `crates/reeved/src/stubs.rs` (temporary)**

```rust
//! Temporary port impls for reeve-card during the staged refactor.
//!
//! - `StubWorkspaceLifecycle` delegates to today's `effects::LiveEffects`.
//!   Replaced by `reeve-workspace`'s real impl in Stage 4.
//! - `StubHouseholdLookup` queries the households table directly.
//!   Replaced by `reeve-household`'s real impl in Stage 3.
//!
//! Delete this file after Stage 4.

use crate::db;
use crate::effects::{DaemonState, LiveEffects};
use crate::transitions::TransitionEffects;
use anyhow::Result;
use async_trait::async_trait;
use reeve_card::{HouseholdLookup, WorkspaceLifecycle};
use reeve_protocol::Card;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct StubWorkspaceLifecycle {
    pub state: DaemonState,
}

#[async_trait]
impl WorkspaceLifecycle for StubWorkspaceLifecycle {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        let fx = LiveEffects::new(self.state.clone());
        TransitionEffects::ensure_running(&fx, card).await
    }

    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        let fx = LiveEffects::new(self.state.clone());
        TransitionEffects::ensure_stopped(&fx, card_id).await
    }
}

pub struct StubHouseholdLookup {
    pub db: Arc<Mutex<Connection>>,
}

#[async_trait]
impl HouseholdLookup for StubHouseholdLookup {
    async fn exists(&self, name: &str) -> Result<bool> {
        let db = self.db.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = db.blocking_lock();
            Ok(db::get_household(&conn, &name)?.is_some())
        })
        .await?
    }
}
```

- [ ] **Step 3: Add `mod stubs;` to `crates/reeved/src/main.rs`**

After other `mod` declarations:

```rust
mod stubs;
```

- [ ] **Step 4: Update the dispatcher in `crates/reeved/src/main.rs`**

Find the existing `MainDispatcher`/`dispatch` setup (currently delegates to `handlers::dispatch`). We need to route card methods through `reeve-card`. Easiest: route inside `handlers::dispatch` (which we'll empty later). Open `crates/reeved/src/handlers.rs`, find the top-level `pub async fn dispatch(...)` function. Replace the match arms for `"ls"`, `"new"`, `"mv"`, `"mv-self"`, `"status-self"` with delegations.

In `crates/reeved/src/handlers.rs`, replace the match body so the card-method arms call `reeve_card::handle_*`:

```rust
// Inside dispatch():
match req.method.as_str() {
    "ls" => {
        let cards = make_card_repo(state.db.clone());
        reeve_card::handle_ls(&*cards).await
    }
    "new" => {
        let cards = make_card_repo(state.db.clone());
        let params = parse_params::<NewParams>(req.params)?;
        reeve_card::handle_new(&*cards, params).await
    }
    "mv" => {
        let cards = make_card_repo(state.db.clone());
        let households = stub_household_lookup(state.db.clone());
        let workspace = stub_workspace_lifecycle(state.clone());
        let params = parse_params::<MvParams>(req.params)?;
        reeve_card::handle_mv(&*cards, &*households, &*workspace, params).await
    }
    "mv-self" => {
        let card_id = require_card(&caller)?;
        let cards = make_card_repo(state.db.clone());
        let households = stub_household_lookup(state.db.clone());
        let workspace = stub_workspace_lifecycle(state.clone());
        let params = parse_params::<MvSelfParams>(req.params)?;
        reeve_card::handle_mv_self(&*cards, &*households, &*workspace, card_id, params).await
    }
    "status-self" => {
        let card_id = require_card(&caller)?;
        let cards = make_card_repo(state.db.clone());
        let container_status = fetch_container_status(&state, &card_id).await?;
        reeve_card::handle_status_self(&*cards, container_status, card_id).await
    }
    // ...household.* and workspace.* arms unchanged for now
}
```

Add these helper functions to the same file:

```rust
use crate::stubs::{StubHouseholdLookup, StubWorkspaceLifecycle};
use reeve_card::{make_card_repo, CardRepo, HouseholdLookup, WorkspaceLifecycle};
use std::sync::Arc;

fn stub_household_lookup(
    db: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
) -> Arc<dyn HouseholdLookup> {
    Arc::new(StubHouseholdLookup { db })
}

fn stub_workspace_lifecycle(
    state: crate::effects::DaemonState,
) -> Arc<dyn WorkspaceLifecycle> {
    Arc::new(StubWorkspaceLifecycle { state })
}

async fn fetch_container_status(
    state: &crate::effects::DaemonState,
    card_id: &str,
) -> std::result::Result<Option<reeve_protocol::ContainerStatus>, reeve_protocol::RpcError> {
    let db = state.db.clone();
    let id = card_id.to_string();
    let row = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<crate::db::ContainerRow>> {
        let conn = db.blocking_lock();
        crate::db::get_container(&conn, &id)
    })
    .await
    .map_err(|e| reeve_protocol::RpcError {
        code: reeve_protocol::RpcError::INTERNAL,
        message: e.to_string(),
    })?
    .map_err(|e| reeve_protocol::RpcError {
        code: reeve_protocol::RpcError::INTERNAL,
        message: e.to_string(),
    })?;
    Ok(row.map(|r| r.status))
}
```

- [ ] **Step 5: Delete the old card-handler bodies from `crates/reeved/src/handlers.rs`**

Find and delete: `async fn handle_ls`, `handle_new`, `handle_mv`, `handle_mv_self`, `handle_status_self`. Their dispatch-arm bodies above replaced them; the legacy bodies are now dead code.

Also delete the old `precheck_household_for_implementing` helper if no remaining handler uses it (household + workspace handlers don't).

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass. The card-handler tests in `crates/reeved/src/handlers.rs::tests` may need port-arg style updates, but the simplest fix is to delete them and rely on reeve-card's tests + e2e. (We covered the policy + repo separately.)

If tests in `handlers.rs::tests` fail because they referenced deleted functions, delete those specific tests; the same behaviour is covered by `reeve-card`'s tests and `tests/e2e.rs`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(2.5): cut over card RPCs to reeve-card with stub port impls

handlers::dispatch routes "ls"/"new"/"mv"/"mv-self"/"status-self" to
reeve_card::handle_*. WorkspaceLifecycle and HouseholdLookup are
temporarily implemented in reeved/src/stubs.rs (StubWorkspaceLifecycle
delegates to LiveEffects; StubHouseholdLookup queries the households
table directly). Real impls land in Stages 3 and 4.

Card-handler bodies removed from reeved/src/handlers.rs. Card-handler
tests removed from handlers.rs::tests (covered by reeve-card unit tests
+ tests/e2e.rs).
EOF
)"
```

---

## Stage 3 — Carve `reeve-household`

Goal: extract Household aggregate (registration, Manifest, scaffold, handlers) into its own crate. Replace `StubHouseholdLookup` with the real impl.

### Task 3.1: Move `manifest.rs` into `reeve-household`

**Files:**
- Move: `crates/reeved/src/manifest.rs` → `crates/reeve-household/src/manifest.rs`
- Modify: `crates/reeve-household/src/lib.rs`
- Modify: `crates/reeved/src/main.rs` (remove `mod manifest;`)
- Modify: any reeved file with `use crate::manifest::*` (likely `handlers.rs`, `effects.rs`)

- [ ] **Step 1: Move the file**

```bash
git mv crates/reeved/src/manifest.rs crates/reeve-household/src/manifest.rs
```

- [ ] **Step 2: Update `crates/reeve-household/src/lib.rs`**

```rust
//! Household aggregate: registered project bundle.

pub mod manifest;
```

- [ ] **Step 3: Remove `mod manifest;` from `crates/reeved/src/main.rs`**

Delete the line `mod manifest;`.

- [ ] **Step 4: Add `reeve-household` dep to `crates/reeved/Cargo.toml`**

In `[dependencies]`:

```toml
reeve-household = { path = "../reeve-household" }
```

- [ ] **Step 5: Replace reeved's `use crate::manifest::*` with `use reeve_household::manifest::*`**

Run: `git grep -n 'crate::manifest' crates/reeved/src/`

For each match, change `crate::manifest` to `reeve_household::manifest`.

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass; `manifest`'s tests now run under `reeve-household`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(3.1): move manifest.rs into reeve-household

Pure relocation; module path updated to reeve_household::manifest.
Manifest tests now run under reeve-household.
EOF
)"
```

### Task 3.2: Create `domain.rs` (Household entity moved from `reeved/src/household.rs`)

**Files:**
- Create: `crates/reeve-household/src/domain.rs`
- Modify: `crates/reeve-household/src/lib.rs`
- Modify: `crates/reeved/src/household.rs` (will be deleted in Task 3.7; keep it functional for now)

- [ ] **Step 1: Create `crates/reeve-household/src/domain.rs`**

Contents (adapt from `crates/reeved/src/household.rs`):

```rust
//! Household domain entity.

use crate::manifest::{self, Manifest};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub name: String,
    pub meta_repo_path: PathBuf,
}

/// Load and validate the meta-repo's manifest.
pub fn load_household_manifest(household: &Household) -> Result<Manifest> {
    manifest::load_manifest(&household.meta_repo_path)
        .with_context(|| format!("load manifest for household '{}'", household.name))
}
```

(Stage 5 promotes this to a richer entity with private fields + invariants — for now, keep it field-compatible with what's currently in reeved.)

- [ ] **Step 2: Update `crates/reeve-household/src/lib.rs`**

```rust
//! Household aggregate: registered project bundle.

mod domain;
pub mod manifest;

pub use domain::{load_household_manifest, Household};
```

- [ ] **Step 3: Build**

```bash
cargo build --workspace
```

Expected: succeeds. (reeved/src/household.rs still exists; both coexist briefly.)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(3.2): reeve-household exposes Household + load_household_manifest

Adapted from reeved/src/household.rs. Stage 7 deletes the old file.
EOF
)"
```

### Task 3.3: Create `repo.rs` — HouseholdRepo trait, SqliteHouseholdRepo, HouseholdLookup impl, factory, init_schema

**Files:**
- Create: `crates/reeve-household/src/repo.rs`
- Modify: `crates/reeve-household/src/lib.rs`

- [ ] **Step 1: Create `crates/reeve-household/src/repo.rs`**

```rust
//! Persistence port + SQLite adapter for the Household aggregate.
//! Also implements `reeve_card::HouseholdLookup`.

use crate::domain::Household;
use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Persistence port for Household. Owned by `reeve-household`.
#[async_trait]
pub trait HouseholdRepo: Send + Sync {
    async fn insert(&self, row: HouseholdRow) -> Result<()>;
    async fn remove(&self, name: &str) -> Result<bool>;
    async fn list(&self) -> Result<Vec<HouseholdRow>>;
    async fn get(&self, name: &str) -> Result<Option<HouseholdRow>>;
}

#[derive(Debug, Clone)]
pub struct HouseholdRow {
    pub name: String,
    pub meta_repo_path: PathBuf,
    pub added_at: i64,
}

impl From<&HouseholdRow> for Household {
    fn from(r: &HouseholdRow) -> Self {
        Household {
            name: r.name.clone(),
            meta_repo_path: r.meta_repo_path.clone(),
        }
    }
}

pub(crate) struct SqliteHouseholdRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteHouseholdRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl HouseholdRepo for SqliteHouseholdRepo {
    async fn insert(&self, row: HouseholdRow) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO households (name, meta_repo_path, added_at) VALUES (?1, ?2, ?3)",
                params![row.name, row.meta_repo_path.to_string_lossy(), row.added_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove(&self, name: &str) -> Result<bool> {
        let conn = self.conn.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = conn.blocking_lock();
            let n = conn.execute("DELETE FROM households WHERE name = ?1", [name])?;
            Ok(n > 0)
        })
        .await?
    }

    async fn list(&self) -> Result<Vec<HouseholdRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<HouseholdRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT name, meta_repo_path, added_at FROM households ORDER BY name",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(HouseholdRow {
                        name: r.get(0)?,
                        meta_repo_path: PathBuf::from(r.get::<_, String>(1)?),
                        added_at: r.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?
    }

    async fn get(&self, name: &str) -> Result<Option<HouseholdRow>> {
        let conn = self.conn.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<HouseholdRow>> {
            let conn = conn.blocking_lock();
            let row = conn
                .query_row(
                    "SELECT name, meta_repo_path, added_at FROM households WHERE name = ?1",
                    [name],
                    |r| {
                        Ok(HouseholdRow {
                            name: r.get(0)?,
                            meta_repo_path: PathBuf::from(r.get::<_, String>(1)?),
                            added_at: r.get(2)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
        .await?
    }
}

/// The same SQLite repo also implements reeve_card's HouseholdLookup port.
/// Two trait views, one backing struct.
#[async_trait]
impl reeve_card::HouseholdLookup for SqliteHouseholdRepo {
    async fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.get(name).await?.is_some())
    }
}

/// Factory returning both trait views from one underlying connection.
/// Reeved holds both Arcs; they point at the same SqliteHouseholdRepo instance.
pub fn make_household_components(
    conn: Arc<Mutex<Connection>>,
) -> (
    Arc<dyn HouseholdRepo>,
    Arc<dyn reeve_card::HouseholdLookup>,
) {
    let concrete = Arc::new(SqliteHouseholdRepo::new(conn));
    (concrete.clone(), concrete)
}

/// Initialise the households table. Called by reeved's migrate orchestrator.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS households (
            name           TEXT PRIMARY KEY,
            meta_repo_path TEXT NOT NULL,
            added_at       INTEGER NOT NULL
         );",
    )
    .context("init_schema for households")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<dyn HouseholdRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (repo, _) = make_household_components(Arc::new(Mutex::new(conn)));
        repo
    }

    #[tokio::test]
    async fn insert_then_get() {
        let repo = fresh();
        repo.insert(HouseholdRow {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/x"),
            added_at: 1,
        })
        .await
        .unwrap();
        let got = repo.get("test").await.unwrap().unwrap();
        assert_eq!(got.name, "test");
    }

    #[tokio::test]
    async fn remove_returns_true_if_existed() {
        let repo = fresh();
        repo.insert(HouseholdRow {
            name: "x".into(),
            meta_repo_path: PathBuf::from("/x"),
            added_at: 1,
        })
        .await
        .unwrap();
        assert!(repo.remove("x").await.unwrap());
        assert!(!repo.remove("x").await.unwrap());
    }

    #[tokio::test]
    async fn lookup_impl_returns_existence() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (repo, lookup) = make_household_components(Arc::new(Mutex::new(conn)));
        repo.insert(HouseholdRow {
            name: "a".into(),
            meta_repo_path: PathBuf::from("/a"),
            added_at: 0,
        })
        .await
        .unwrap();
        assert!(lookup.exists("a").await.unwrap());
        assert!(!lookup.exists("nope").await.unwrap());
    }
}
```

- [ ] **Step 2: Update `crates/reeve-household/src/lib.rs`**

```rust
//! Household aggregate: registered project bundle.

mod domain;
pub mod manifest;
mod repo;

pub use domain::{load_household_manifest, Household};
pub use repo::{init_schema, make_household_components, HouseholdRepo, HouseholdRow};
```

- [ ] **Step 3: Build and test**

```bash
cargo build --workspace
cargo test -p reeve-household
```

Expected: 3 tests pass in `reeve-household`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(3.3): reeve-household defines HouseholdRepo + impl HouseholdLookup

SqliteHouseholdRepo is pub(crate); only obtainable via
make_household_components(), which returns both
(Arc<dyn HouseholdRepo>, Arc<dyn HouseholdLookup>) pointing at the
same concrete instance. init_schema exported for reeved's orchestrator.
EOF
)"
```

### Task 3.4: Create `scaffold.rs` — extract the inline git workflow from `handle_household_new`

**Files:**
- Create: `crates/reeve-household/src/scaffold.rs`
- Modify: `crates/reeve-household/src/lib.rs`

The current `handle_household_new` (in `crates/reeved/src/handlers.rs`) inlines: clone template → rm .git → git init → manifest rewrite → git add + commit. Extract into a port with a real impl.

- [ ] **Step 1: Create `crates/reeve-household/src/scaffold.rs`**

```rust
//! Household scaffolding port: clone-template → re-init → manifest-rewrite → commit.
//! Used by `handle_household_new`.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;

/// Port for the git operations household scaffolding needs.
#[async_trait]
pub trait HouseholdScaffold: Send + Sync {
    async fn clone_template(&self, template_url: &str, dest: &Path) -> Result<()>;
    async fn init_repo(&self, dest: &Path) -> Result<()>;
    async fn commit_all(&self, dest: &Path, message: &str) -> Result<()>;
}

pub(crate) struct RealHouseholdScaffold;

#[async_trait]
impl HouseholdScaffold for RealHouseholdScaffold {
    async fn clone_template(&self, template_url: &str, dest: &Path) -> Result<()> {
        let output = Command::new("git")
            .args([
                "clone",
                template_url,
                dest.to_str().context("dest path not utf-8")?,
            ])
            .output()
            .await
            .context("exec git clone (template)")?;
        if !output.status.success() {
            anyhow::bail!(
                "git clone {template_url} failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn init_repo(&self, dest: &Path) -> Result<()> {
        std::fs::remove_dir_all(dest.join(".git"))
            .with_context(|| format!("remove {}/.git", dest.display()))?;
        git_in(dest, &["init", "-b", "main", "--quiet"]).await
    }

    async fn commit_all(&self, dest: &Path, message: &str) -> Result<()> {
        git_in(dest, &["add", "-A"]).await?;
        git_in(dest, &["commit", "-m", message, "--quiet"]).await
    }
}

async fn git_in(dir: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "git {args:?} failed in {} (exit {}): {}",
            dir.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

pub fn make_scaffold() -> Arc<dyn HouseholdScaffold> {
    Arc::new(RealHouseholdScaffold)
}
```

- [ ] **Step 2: Update `crates/reeve-household/src/lib.rs`**

```rust
//! Household aggregate: registered project bundle.

mod domain;
pub mod manifest;
mod repo;
mod scaffold;

pub use domain::{load_household_manifest, Household};
pub use repo::{init_schema, make_household_components, HouseholdRepo, HouseholdRow};
pub use scaffold::{make_scaffold, HouseholdScaffold};
```

- [ ] **Step 3: Build**

```bash
cargo build --workspace
```

Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(3.4): extract HouseholdScaffold trait + RealHouseholdScaffold

Encapsulates the inline git workflow (clone template, re-init, commit)
currently embedded in handle_household_new. The handler will be
rewritten in Task 3.5 to delegate here.
EOF
)"
```

### Task 3.5: Move household handlers into `reeve-household/src/handler.rs`

**Files:**
- Create: `crates/reeve-household/src/handler.rs`
- Modify: `crates/reeve-household/src/lib.rs`
- Modify: `crates/reeve-household/Cargo.toml` (add uuid + serde dep if not already present — already there)

The handlers reproduce `handle_household_add`, `handle_household_new`, `handle_household_ls`, `handle_household_rm`, `handle_household_show` from today's `crates/reeved/src/handlers.rs`, refactored to take ports as args.

- [ ] **Step 1: Add `git2` or stay with subprocess git for `clone_url`**

We need a `clone_url(url, dest)` operation for `household add` (URL form) and `household new`. Define it in `reeve-household/src/scaffold.rs` if it isn't already there:

In `scaffold.rs`, add a method to the trait:

```rust
async fn clone_url(&self, url: &str, dest: &Path) -> Result<()>;
```

And impl:

```rust
async fn clone_url(&self, url: &str, dest: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["clone", url, dest.to_str().context("dest path not utf-8")?])
        .output()
        .await
        .context("exec git clone (url)")?;
    if !output.status.success() {
        anyhow::bail!(
            "git clone {url} failed (exit {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
```

(This duplicates `RealGitOps::clone_url` in `reeve-workspace`; per spec, that's accepted under rule-of-three.)

- [ ] **Step 2: Create `crates/reeve-household/src/handler.rs`**

```rust
//! RPC handlers for the Household aggregate.

use crate::domain::Household;
use crate::manifest::{self, Manifest};
use crate::repo::{HouseholdRepo, HouseholdRow};
use crate::scaffold::HouseholdScaffold;
use anyhow::Result;
use reeve_protocol::{
    CardReference, HouseholdAddParams, HouseholdDetail, HouseholdNewParams, HouseholdSource,
    HouseholdSummary, RepoDetail, RepoOnDisk, RpcError,
};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_TEMPLATE: &str = "https://github.com/Mindful-Stack/witan-household";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

fn default_source_dir() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("Source"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn summary_from(row: &HouseholdRow, m: &Manifest) -> HouseholdSummary {
    HouseholdSummary {
        name: row.name.clone(),
        meta_repo_path: row.meta_repo_path.clone(),
        workspace_name: m.workspace.clone(),
        knowledge_base: m.knowledge_base.clone(),
        repo_count: m.repos.len(),
    }
}

#[derive(Deserialize)]
pub struct NameParam {
    pub name: String,
}

pub async fn handle_household_add(
    repo: &dyn HouseholdRepo,
    scaffold: &dyn HouseholdScaffold,
    params: HouseholdAddParams,
) -> std::result::Result<Value, RpcError> {
    let meta_repo_path = match &params.source {
        HouseholdSource::Path { path } => {
            if params.clone_into.is_some() {
                return Err(RpcError {
                    code: RpcError::INVALID_PARAMS,
                    message: "--clone-into is only valid for URL sources".into(),
                });
            }
            if !path.exists() {
                return Err(RpcError {
                    code: RpcError::NOT_FOUND,
                    message: format!("path {} does not exist", path.display()),
                });
            }
            if !path.join(".git").exists() {
                return Err(RpcError {
                    code: RpcError::INVALID_ARGUMENT,
                    message: format!("path {} is not a git repository", path.display()),
                });
            }
            path.clone()
        }
        HouseholdSource::Url { url } => {
            let dest = params
                .clone_into
                .clone()
                .unwrap_or_else(|| default_source_dir().join(&params.name));
            if dest.exists() {
                return Err(RpcError {
                    code: RpcError::CONFLICT,
                    message: format!("target directory {} already exists", dest.display()),
                });
            }
            scaffold
                .clone_url(url, &dest)
                .await
                .map_err(|e| RpcError {
                    code: RpcError::INTERNAL,
                    message: format!("clone {url}: {e:#}"),
                })?;
            dest
        }
    };

    let m = manifest::load_manifest(&meta_repo_path).map_err(|e| RpcError {
        code: RpcError::INVALID_ARGUMENT,
        message: format!("manifest invalid: {e:#}"),
    })?;

    let row = HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: meta_repo_path.clone(),
        added_at: now_secs(),
    };
    repo.insert(row.clone()).await.map_err(|e| RpcError {
        code: RpcError::CONFLICT,
        message: format!("insert household: {e:#}"),
    })?;

    serde_json::to_value(summary_from(&row, &m)).map_err(internal)
}

pub async fn handle_household_new(
    repo: &dyn HouseholdRepo,
    scaffold: &dyn HouseholdScaffold,
    params: HouseholdNewParams,
) -> std::result::Result<Value, RpcError> {
    let dest = params
        .path
        .clone()
        .unwrap_or_else(|| default_source_dir().join(&params.name));

    if dest.exists()
        && dest
            .read_dir()
            .map(|mut i| i.next().is_some())
            .unwrap_or(true)
    {
        return Err(RpcError {
            code: RpcError::CONFLICT,
            message: format!(
                "target directory {} exists and is non-empty",
                dest.display()
            ),
        });
    }

    let template = params
        .template
        .clone()
        .unwrap_or_else(|| DEFAULT_TEMPLATE.into());

    scaffold
        .clone_template(&template, &dest)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("clone template {template}: {e:#}"),
        })?;

    scaffold
        .init_repo(&dest)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("init repo: {e:#}"),
        })?;

    let manifest_path = dest.join("household.json");
    let body = std::fs::read_to_string(&manifest_path).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("read manifest: {e:#}"),
    })?;
    let mut m: Manifest = serde_json::from_str(&body).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("parse template manifest: {e:#}"),
    })?;
    let old_workspace = m.workspace.clone();
    if let Some(entry) = m.repos.iter_mut().find(|r| r.name == old_workspace) {
        entry.name = params.name.clone();
    } else {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "template manifest's workspace = '{old_workspace}' has no matching repos[] entry"
            ),
        });
    }
    m.workspace = params.name.clone();
    let body = serde_json::to_string_pretty(&m).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("re-serialize manifest: {e:#}"),
    })?;
    std::fs::write(&manifest_path, body).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("write manifest: {e:#}"),
    })?;

    scaffold
        .commit_all(&dest, "Initial workspace")
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("commit: {e:#}"),
        })?;

    let row = HouseholdRow {
        name: params.name.clone(),
        meta_repo_path: dest.clone(),
        added_at: now_secs(),
    };
    repo.insert(row.clone()).await.map_err(|e| RpcError {
        code: RpcError::CONFLICT,
        message: format!("insert household: {e:#}"),
    })?;

    serde_json::to_value(summary_from(&row, &m)).map_err(internal)
}

pub async fn handle_household_ls(
    repo: &dyn HouseholdRepo,
) -> std::result::Result<Value, RpcError> {
    let rows = repo.list().await.map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("list households: {e:#}"),
    })?;
    let mut out: Vec<HouseholdSummary> = Vec::with_capacity(rows.len());
    for row in rows {
        let m = manifest::load_manifest(&row.meta_repo_path).unwrap_or_else(|_| Manifest {
            workspace: row.name.clone(),
            knowledge_base: None,
            repos: vec![],
        });
        out.push(summary_from(&row, &m));
    }
    serde_json::to_value(out).map_err(internal)
}

pub async fn handle_household_rm(
    repo: &dyn HouseholdRepo,
    params: NameParam,
) -> std::result::Result<Value, RpcError> {
    let removed = repo.remove(&params.name).await.map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("remove household: {e:#}"),
    })?;
    if !removed {
        return Err(RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("household '{}' not found", params.name),
        });
    }
    serde_json::to_value(()).map_err(internal)
}

pub async fn handle_household_show(
    repo: &dyn HouseholdRepo,
    cards_for_household: Vec<CardReference>,
    params: NameParam,
) -> std::result::Result<Value, RpcError> {
    let row = repo
        .get(&params.name)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("get household: {e:#}"),
        })?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("household '{}' not found", params.name),
        })?;

    let m = manifest::load_manifest(&row.meta_repo_path).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("load manifest: {e:#}"),
    })?;

    let workspace_url = m
        .workspace_entry()
        .url
        .clone()
        .or_else(|| git_remote_url(&row.meta_repo_path));

    let repos: Vec<RepoDetail> = m
        .repos
        .iter()
        .map(|entry| {
            let is_workspace = entry.name == m.workspace;
            let is_knowledge_base = m.knowledge_base.as_deref() == Some(entry.name.as_str());
            let on_disk = if is_workspace {
                RepoOnDisk::Workspace
            } else {
                let dir = row.meta_repo_path.join(&entry.name);
                if dir.join(".git").exists() {
                    RepoOnDisk::Sibling
                } else if dir.exists() {
                    RepoOnDisk::Inline
                } else {
                    RepoOnDisk::Missing
                }
            };
            RepoDetail {
                name: entry.name.clone(),
                url: entry.url.clone(),
                tags: entry.tags.clone(),
                description: entry.description.clone(),
                is_workspace,
                is_knowledge_base,
                on_disk,
            }
        })
        .collect();

    let detail = HouseholdDetail {
        summary: summary_from(&row, &m),
        workspace_url,
        repos,
        cards: cards_for_household,
    };
    serde_json::to_value(detail).map_err(internal)
}

fn git_remote_url(meta_repo: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(meta_repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
```

Note: `handle_household_show` takes `cards_for_household: Vec<CardReference>` as a pre-resolved argument. The dispatch layer in `reeved` looks them up via `CardRepo::for_household` before calling. This keeps reeve-household free of CardRepo deps.

- [ ] **Step 3: Update `crates/reeve-household/src/lib.rs`**

```rust
//! Household aggregate: registered project bundle.

mod domain;
mod handler;
pub mod manifest;
mod repo;
mod scaffold;

pub use domain::{load_household_manifest, Household};
pub use handler::{
    handle_household_add, handle_household_ls, handle_household_new,
    handle_household_rm, handle_household_show, NameParam,
};
pub use repo::{init_schema, make_household_components, HouseholdRepo, HouseholdRow};
pub use scaffold::{make_scaffold, HouseholdScaffold};
```

- [ ] **Step 4: Build**

```bash
cargo build --workspace
```

Expected: succeeds. (No new tests in handler.rs — covered by reeved's integration tests after cutover, plus the existing e2e tests.)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(3.5): household handlers moved to reeve-household/src/handler.rs

handle_household_add/new/ls/rm/show take port references as arguments.
handle_household_show receives pre-resolved CardReferences from the
dispatch layer (avoids household-side CardRepo dep).
EOF
)"
```

### Task 3.6: Cut over reeved to use reeve-household; delete the stub HouseholdLookup

**Files:**
- Modify: `crates/reeved/src/handlers.rs` (route household.* to reeve-household; remove old handler bodies)
- Modify: `crates/reeved/src/main.rs` / `crates/reeved/src/stubs.rs` (replace StubHouseholdLookup with reeve_household::make_household_components)
- Delete: `crates/reeved/src/household.rs` (now in reeve-household/src/domain.rs)
- Modify: `crates/reeved/src/db.rs` (remove `HouseholdRow`, `insert_household`, `remove_household`, `list_households`, `get_household`; cards_for_household stays)

- [ ] **Step 1: Update the dispatch table in `crates/reeved/src/handlers.rs` to route household methods to reeve-household**

In the `match req.method.as_str()` block, replace the existing arms for `household.add`, `household.new`, `household.ls`, `household.rm`, `household.show` with:

```rust
"household.add" => {
    let scaffold = reeve_household::make_scaffold();
    let params = parse_params::<reeve_protocol::HouseholdAddParams>(req.params)?;
    reeve_household::handle_household_add(
        &*state.household_repo,
        &*scaffold,
        params,
    ).await
}
"household.new" => {
    let scaffold = reeve_household::make_scaffold();
    let params = parse_params::<reeve_protocol::HouseholdNewParams>(req.params)?;
    reeve_household::handle_household_new(
        &*state.household_repo,
        &*scaffold,
        params,
    ).await
}
"household.ls" => {
    reeve_household::handle_household_ls(&*state.household_repo).await
}
"household.rm" => {
    let params = parse_params::<reeve_household::NameParam>(req.params)?;
    reeve_household::handle_household_rm(&*state.household_repo, params).await
}
"household.show" => {
    let params = parse_params::<reeve_household::NameParam>(req.params)?;
    // pre-resolve referencing cards via CardRepo
    let cards_repo = reeve_card::make_card_repo(state.db.clone());
    let cards = cards_repo.for_household(&params.name).await.map_err(internal)?;
    reeve_household::handle_household_show(&*state.household_repo, cards, params).await
}
```

- [ ] **Step 2: Update `DaemonState` in `crates/reeved/src/effects.rs`**

Add fields:

```rust
pub household_repo: Arc<dyn reeve_household::HouseholdRepo>,
pub household_lookup: Arc<dyn reeve_card::HouseholdLookup>,
```

- [ ] **Step 3: Update `crates/reeved/src/main.rs` composition**

Where `DaemonState` is constructed, before constructing it:

```rust
let (household_repo, household_lookup) =
    reeve_household::make_household_components(db.clone());
```

Pass both into `DaemonState`.

- [ ] **Step 4: Delete `crates/reeved/src/stubs.rs::StubHouseholdLookup` and its uses**

In `stubs.rs`, delete `StubHouseholdLookup` and the impl. Keep `StubWorkspaceLifecycle` (Stage 4 replaces it).

In `handlers.rs`, replace `stub_household_lookup(state.db.clone())` with `state.household_lookup.clone()`.

- [ ] **Step 5: Delete `crates/reeved/src/household.rs`**

```bash
git rm crates/reeved/src/household.rs
```

Remove `mod household;` from `crates/reeved/src/main.rs`. Replace any `use crate::household::*` with `use reeve_household::*`.

- [ ] **Step 6: Remove household-related code from `crates/reeved/src/db.rs`**

Delete: `HouseholdRow` struct, `insert_household`, `remove_household`, `list_households`, `get_household`, and any tests for them (they're now in `reeve-household/src/repo.rs`).

Keep `cards_for_household` for now (workspace will need it during Stage 4). Actually — `cards_for_household` is a card-side query, not a household-side query. The card repo (in `reeve-card`) already has `for_household`. So we can delete `db::cards_for_household` from `reeved/src/db.rs` too. Verify by searching: `git grep 'db::cards_for_household' crates/reeved/src/` — if the only remaining caller was the old household_show handler (which is now gone), delete it.

- [ ] **Step 7: Update schema-init in reeved**

`crates/reeved/src/db.rs::migrate` currently creates all tables. Update it to:

```rust
fn migrate(conn: &Connection) -> Result<()> {
    reeve_card::init_schema(conn)?;
    reeve_household::init_schema(conn)?;
    // workspace schema-init joins in Stage 4
    // (containers table init still inline in db.rs until then)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS containers (
            card_id        TEXT PRIMARY KEY,
            container_name TEXT NOT NULL,
            status         TEXT NOT NULL CHECK (status IN ('spawning','running','stopped','failed')),
            started_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_containers_status ON containers(status);
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}
```

(Note: the containers-table FK to cards is **dropped here**, ahead of Stage 4. This is a small concession to keep schema-init consistent during the stage.)

- [ ] **Step 8: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass. If `crates/reeved/src/handlers.rs::tests` has tests for household handlers, delete them (covered by reeve-household tests + e2e). Otherwise tests pass green.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(3.6): cut over household RPCs to reeve-household; FK preview-drop

dispatch routes household.add/new/ls/rm/show to reeve_household::*.
DaemonState gains household_repo + household_lookup Arcs from
make_household_components. StubHouseholdLookup deleted (real impl now
in reeve-household).

reeved/src/household.rs deleted. Household-related DB code deleted from
reeved/src/db.rs (lives in reeve-household/src/repo.rs).

Containers FK to cards dropped preemptively in migrate() to keep
schema-init consistent during the staged refactor; Stage 4 confirms
this when workspace takes over containers schema-init.
EOF
)"
```

---

## Stage 4 — Carve `reeve-workspace` + drop FK definitively

Goal: extract Workspace aggregate (provision/teardown, git, container, fetch, attach, push). Replace `StubWorkspaceLifecycle` with the real impl from `LiveEffects`. Take ownership of the containers schema.

### Task 4.1: Move infrastructure files into reeve-workspace as-is

This is a series of file relocations. Each moves one file, updates imports, builds, commits.

**Files (in order):**

a. `crates/reeved/src/container.rs` → `crates/reeve-workspace/src/container.rs`
b. `crates/reeved/src/fetch.rs` → `crates/reeve-workspace/src/fetch.rs`
c. `crates/reeved/src/clones.rs` → split into `crates/reeve-workspace/src/git.rs` (the `GitOps` trait + `RealGitOps`) and `crates/reeve-workspace/src/clones.rs` (the `clone_all_for_card` workflow)

- [ ] **Step 1: Move container.rs**

```bash
git mv crates/reeved/src/container.rs crates/reeve-workspace/src/container.rs
```

Add to `crates/reeve-workspace/src/lib.rs`:

```rust
pub mod container;
```

Remove `mod container;` from `crates/reeved/src/main.rs`. Update `use crate::container::*` to `use reeve_workspace::container::*` everywhere in reeved.

- [ ] **Step 2: Within container.rs, make `DockerOps` pub(crate) and add `make_container_ops` factory**

In `crates/reeve-workspace/src/container.rs`, change `pub struct DockerOps` to `pub(crate) struct DockerOps`. Add at the end of the file:

```rust
pub fn make_container_ops() -> Arc<dyn ContainerOps> {
    Arc::new(DockerOps)
}
```

(Update lib.rs exports to expose `ContainerOps` and `make_container_ops`, not the struct.)

- [ ] **Step 3: Build + test**

```bash
cargo build --workspace
cargo test --workspace
```

Fix any imports that break. Update `effects.rs` (still in reeved) to import `ContainerOps` from `reeve_workspace::container`. Update `clones.rs` similarly if it needs container types.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(4.1a): move container.rs into reeve-workspace

DockerOps demoted to pub(crate); only obtainable via make_container_ops().
Imports updated across reeved (effects, clones, etc.).
EOF
)"
```

- [ ] **Step 5: Move fetch.rs**

```bash
git mv crates/reeved/src/fetch.rs crates/reeve-workspace/src/fetch.rs
```

Add to `crates/reeve-workspace/src/lib.rs`:

```rust
pub mod fetch;
```

Remove `mod fetch;` from reeved/main.rs. Update imports.

- [ ] **Step 6: Build + test + commit**

```bash
cargo build --workspace
cargo test --workspace
```

```bash
git add -A
git commit -m "refactor(4.1b): move fetch.rs into reeve-workspace"
```

- [ ] **Step 7: Split clones.rs into git.rs + clones.rs**

```bash
git mv crates/reeved/src/clones.rs crates/reeve-workspace/src/clones.rs
```

Open `crates/reeve-workspace/src/clones.rs`. Cut the `GitOps` trait + `RealGitOps` struct + their impls into a new file `crates/reeve-workspace/src/git.rs`:

```rust
//! Git port + RealGitOps adapter.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;

#[async_trait]
pub trait GitOps: Send + Sync {
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()>;
    async fn clone_url(&self, url: &str, dest: &Path) -> Result<()>;
    async fn fetch(&self, repo: &Path, source: &Path, refspec: &str) -> Result<()>;
    async fn push(&self, repo: &Path, branch: &str) -> Result<()>;
    // (Add any other GitOps methods that exist today — copy verbatim from clones.rs.)
}

pub(crate) struct RealGitOps;

#[async_trait]
impl GitOps for RealGitOps {
    // (Move method bodies from clones.rs verbatim.)
}

pub fn make_git_ops() -> Arc<dyn GitOps> {
    Arc::new(RealGitOps)
}
```

In `crates/reeve-workspace/src/clones.rs`, keep only `clone_all_for_card` and helpers. It should import `crate::git::GitOps` at the top.

Update `crates/reeve-workspace/src/lib.rs`:

```rust
pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;

pub use git::{make_git_ops, GitOps};
```

- [ ] **Step 8: Build + test + commit**

```bash
cargo build --workspace
cargo test --workspace
```

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(4.1c): split clones.rs into git.rs (port+adapter) + clones.rs (workflow)

GitOps + RealGitOps live in git.rs; RealGitOps is pub(crate) with
make_git_ops factory. clones.rs keeps the clone_all_for_card workflow.
EOF
)"
```

### Task 4.2: Create `domain.rs` — Workspace + Container entities

**Files:**
- Create: `crates/reeve-workspace/src/domain.rs`
- Modify: `crates/reeve-workspace/src/lib.rs`

- [ ] **Step 1: Create `crates/reeve-workspace/src/domain.rs`**

```rust
//! Workspace + Container domain entities.

use reeve_protocol::ContainerStatus;
use std::path::PathBuf;

pub struct Workspace {
    card_id: String,
    container: Option<Container>,
    socket_path: PathBuf,
}

pub struct Container {
    name: String,
    status: ContainerStatus,
    started_at: i64,
}

impl Workspace {
    pub fn for_card(card_id: String, socket_path: PathBuf) -> Self {
        Self {
            card_id,
            container: None,
            socket_path,
        }
    }

    pub fn card_id(&self) -> &str {
        &self.card_id
    }
    pub fn container(&self) -> Option<&Container> {
        self.container.as_ref()
    }
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }
    pub fn is_running(&self) -> bool {
        matches!(
            self.container.as_ref().map(|c| c.status),
            Some(ContainerStatus::Running)
        )
    }

    pub fn with_container(self, container: Container) -> Self {
        Self {
            container: Some(container),
            ..self
        }
    }
    pub fn without_container(self) -> Self {
        Self {
            container: None,
            ..self
        }
    }
}

impl Container {
    pub fn new(name: String, status: ContainerStatus, started_at: i64) -> Self {
        Self {
            name,
            status,
            started_at,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn status(&self) -> ContainerStatus {
        self.status
    }
    pub fn started_at(&self) -> i64 {
        self.started_at
    }
    pub fn with_status(self, status: ContainerStatus) -> Self {
        Self { status, ..self }
    }
}
```

- [ ] **Step 2: Update `crates/reeve-workspace/src/lib.rs`**

```rust
mod domain;

pub use domain::{Container, Workspace};

pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;

pub use git::{make_git_ops, GitOps};
```

- [ ] **Step 3: Build + commit**

```bash
cargo build --workspace
```

```bash
git add crates/reeve-workspace
git commit -m "$(cat <<'EOF'
refactor(4.2): Workspace + Container domain entities (typed, private fields)

Constructors enforce invariants; methods return new Self for state
transitions. No twin DTO — Workspace never transmitted whole.
EOF
)"
```

### Task 4.3: Create `repo.rs` — WorkspaceRepo, SqliteWorkspaceRepo, factory, init_schema (no FK)

**Files:**
- Create: `crates/reeve-workspace/src/repo.rs`
- Modify: `crates/reeve-workspace/src/lib.rs`

- [ ] **Step 1: Create `crates/reeve-workspace/src/repo.rs`**

```rust
//! Persistence port + SQLite adapter for the Workspace aggregate.
//! Owns the containers table (no FK to cards — see spec §"Schema independence").

use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::ContainerStatus;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRow {
    pub card_id: String,
    pub container_name: String,
    pub status: ContainerStatus,
    pub started_at: i64,
    pub updated_at: i64,
}

#[async_trait]
pub trait WorkspaceRepo: Send + Sync {
    async fn upsert_container(
        &self,
        card_id: &str,
        container_name: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()>;
    async fn update_container_status(
        &self,
        card_id: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()>;
    async fn get_container(&self, card_id: &str) -> Result<Option<ContainerRow>>;
    async fn list_by_status(&self, status: ContainerStatus) -> Result<Vec<ContainerRow>>;
    async fn remove_container(&self, card_id: &str) -> Result<()>;
}

pub(crate) struct SqliteWorkspaceRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteWorkspaceRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl WorkspaceRepo for SqliteWorkspaceRepo {
    async fn upsert_container(
        &self,
        card_id: &str,
        container_name: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        let container_name = container_name.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO containers (card_id, container_name, status, started_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(card_id) DO UPDATE SET
                    container_name = excluded.container_name,
                    status         = excluded.status,
                    updated_at     = excluded.updated_at",
                params![card_id, container_name, status.as_wire_str(), now],
            )?;
            Ok(())
        })
        .await?
    }

    async fn update_container_status(
        &self,
        card_id: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "UPDATE containers SET status = ?1, updated_at = ?2 WHERE card_id = ?3",
                params![status.as_wire_str(), now, card_id],
            )?;
            Ok(())
        })
        .await?
    }

    async fn get_container(&self, card_id: &str) -> Result<Option<ContainerRow>> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<ContainerRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT card_id, container_name, status, started_at, updated_at
                 FROM containers WHERE card_id = ?1",
            )?;
            let row = stmt
                .query_row(params![card_id], row_to_container)
                .optional()?;
            Ok(row)
        })
        .await?
    }

    async fn list_by_status(&self, status: ContainerStatus) -> Result<Vec<ContainerRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<ContainerRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT card_id, container_name, status, started_at, updated_at
                 FROM containers WHERE status = ?1",
            )?;
            let rows = stmt
                .query_map(params![status.as_wire_str()], row_to_container)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?
    }

    async fn remove_container(&self, card_id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute("DELETE FROM containers WHERE card_id = ?1", params![card_id])?;
            Ok(())
        })
        .await?
    }
}

fn row_to_container(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContainerRow> {
    let status_str: String = row.get(2)?;
    let status = ContainerStatus::from_wire_str(&status_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(InvalidContainerStatus(status_str.clone())),
        )
    })?;
    Ok(ContainerRow {
        card_id: row.get(0)?,
        container_name: row.get(1)?,
        status,
        started_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("invalid container status: {0}")]
struct InvalidContainerStatus(String);

pub fn make_workspace_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn WorkspaceRepo> {
    Arc::new(SqliteWorkspaceRepo::new(conn))
}

/// Initialise the containers table. NO foreign key to cards — schemas are
/// per-context independent (spec §"Schema independence").
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS containers (
            card_id        TEXT PRIMARY KEY,
            container_name TEXT NOT NULL,
            status         TEXT NOT NULL CHECK (status IN ('spawning','running','stopped','failed')),
            started_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_containers_status ON containers(status);",
    )
    .context("init_schema for containers")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<dyn WorkspaceRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_workspace_repo(Arc::new(Mutex::new(conn)))
    }

    #[tokio::test]
    async fn upsert_then_get() {
        let repo = fresh();
        repo.upsert_container("c1", "reeve-c1", ContainerStatus::Spawning, 1000)
            .await
            .unwrap();
        let row = repo.get_container("c1").await.unwrap().unwrap();
        assert_eq!(row.container_name, "reeve-c1");
        assert_eq!(row.status, ContainerStatus::Spawning);
    }

    #[tokio::test]
    async fn no_fk_no_cards_row_needed() {
        // Critical: dropping the FK means we can insert a containers row
        // without any cards row existing. Verifies the spec's stated benefit.
        let repo = fresh();
        repo.upsert_container("nonexistent-card", "x", ContainerStatus::Running, 0)
            .await
            .unwrap();
        let row = repo.get_container("nonexistent-card").await.unwrap();
        assert!(row.is_some());
    }
}
```

- [ ] **Step 2: Update `crates/reeve-workspace/src/lib.rs`**

```rust
mod domain;
mod repo;

pub use domain::{Container, Workspace};
pub use repo::{init_schema, make_workspace_repo, ContainerRow, WorkspaceRepo};

pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;

pub use git::{make_git_ops, GitOps};
```

- [ ] **Step 3: Build + test + commit**

```bash
cargo build --workspace
cargo test -p reeve-workspace
```

Expected: 2 tests pass (upsert+get, no_fk_no_cards_row_needed).

```bash
git add crates/reeve-workspace
git commit -m "$(cat <<'EOF'
refactor(4.3): reeve-workspace defines WorkspaceRepo + SqliteWorkspaceRepo

Owns containers table schema (no FK to cards). Test
no_fk_no_cards_row_needed pins that schemas are independent now —
critical for per-crate test isolation.
EOF
)"
```

### Task 4.4: Create `lifecycle.rs` — `LiveWorkspaceLifecycle` impl

**Files:**
- Create: `crates/reeve-workspace/src/lifecycle.rs`
- Modify: `crates/reeve-workspace/src/lib.rs`

Port the body of `crates/reeved/src/effects.rs::LiveEffects` into a struct that implements `reeve_card::WorkspaceLifecycle` (instead of the old local `TransitionEffects` trait).

- [ ] **Step 1: Create `crates/reeve-workspace/src/lifecycle.rs`**

```rust
//! LiveWorkspaceLifecycle — concrete impl of reeve_card::WorkspaceLifecycle.
//! Composes the workspace's git, container, and repo ports to provision and
//! tear down a card's workspace.

use crate::clones;
use crate::container::{self, ContainerOps};
use crate::git::GitOps;
use crate::repo::WorkspaceRepo;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_card::WorkspaceLifecycle;
use reeve_protocol::{Card, ContainerStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct LiveWorkspaceLifecycle {
    workspaces: Arc<dyn WorkspaceRepo>,
    households: Arc<dyn reeve_household::HouseholdRepo>,
    git: Arc<dyn GitOps>,
    containers: Arc<dyn ContainerOps>,
    listeners: crate::ListenerHandle,
    claude_dir: PathBuf,
    api_key: Option<String>,
}

/// Wrapper trait to abstract over reeved::transport::listener::ListenerRegistry
/// so reeve-workspace can hold a handle without depending on reeved.
/// Defined here for now; production type lives in reeved.
#[async_trait]
pub trait ListenerOps: Send + Sync {
    async fn spawn(&self, card_id: &str, socket: PathBuf);
    async fn stop(&self, card_id: &str) -> Result<()>;
}

pub type ListenerHandle = Arc<dyn ListenerOps>;

impl LiveWorkspaceLifecycle {
    pub fn new(
        workspaces: Arc<dyn WorkspaceRepo>,
        households: Arc<dyn reeve_household::HouseholdRepo>,
        git: Arc<dyn GitOps>,
        containers: Arc<dyn ContainerOps>,
        listeners: ListenerHandle,
        claude_dir: PathBuf,
        api_key: Option<String>,
    ) -> Self {
        Self {
            workspaces,
            households,
            git,
            containers,
            listeners,
            claude_dir,
            api_key,
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl WorkspaceLifecycle for LiveWorkspaceLifecycle {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        // Port the body of effects::LiveEffects::ensure_running here. Replace
        // direct db:: calls with self.workspaces.* and self.households.*.
        // Replace self.state.containers with self.containers, etc.
        //
        // The card-existence check that the FK previously implied: this
        // function reads the household from self.households; if it doesn't
        // exist we bail. The card-existence check is implicit — we wouldn't
        // have been called without a valid card.

        let household_name = card.household.clone();
        let row = self
            .households
            .get(&household_name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("household '{}' not configured", card.household))?;
        let household = reeve_household::Household {
            name: row.name,
            meta_repo_path: row.meta_repo_path,
        };

        let card_id = &card.id;
        let socket = reeve_protocol::paths::card_socket_path(card_id);
        let cards_socket_dir = reeve_protocol::paths::cards_socket_dir();
        let workspace_path = reeve_protocol::paths::card_workspace_path(card_id);

        std::fs::create_dir_all(&cards_socket_dir)
            .with_context(|| format!("create cards socket dir {}", cards_socket_dir.display()))?;

        // Adopt existing running container if any.
        if let Ok(Some(existing)) = self.containers.discover_container_name(card_id).await {
            if let Some(state) = self.containers.inspect_status(&existing).await? {
                if state == "running" {
                    self.workspaces
                        .upsert_container(card_id, &existing, ContainerStatus::Running, now_secs())
                        .await?;
                    self.listeners.spawn(card_id, socket).await;
                    return Ok(());
                }
            }
        }

        let manifest = reeve_household::load_household_manifest(&household)
            .with_context(|| format!("load manifest for '{}'", household.name))?;

        if let Some(parent) = workspace_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create cards root {}", parent.display()))?;
        }

        clones::clone_all_for_card(&*self.git, &household, &manifest, &workspace_path).await?;

        let devcontainer_path = workspace_path.join(".devcontainer").join("devcontainer.json");
        if !devcontainer_path.exists() {
            anyhow::bail!(
                "devcontainer.json missing at {} — every witan-household must ship one at the conventional location",
                devcontainer_path.display(),
            );
        }

        let argv = container::devcontainer_up_command(
            card_id,
            &workspace_path,
            &devcontainer_path,
            &cards_socket_dir,
            &self.claude_dir,
            self.api_key.as_deref(),
        );
        self.containers.spawn(argv).await?;

        let container_name = discover_with_deadline(&*self.containers, card_id).await?;
        self.workspaces
            .upsert_container(card_id, &container_name, ContainerStatus::Running, now_secs())
            .await?;
        self.listeners.spawn(card_id, socket).await;
        Ok(())
    }

    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        self.listeners.stop(card_id).await?;

        let row = match self.workspaces.get_container(card_id).await? {
            Some(r) => r,
            None => return Ok(()), // never had one
        };

        if self.containers.inspect_status(&row.container_name).await?.is_none() {
            self.workspaces.remove_container(card_id).await?;
            return Ok(());
        }

        self.containers.teardown(&row.container_name).await?;
        self.workspaces.remove_container(card_id).await?;
        Ok(())
    }
}

async fn discover_with_deadline(
    ops: &dyn ContainerOps,
    card_id: &str,
) -> Result<String> {
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(100);
    loop {
        if let Some(name) = ops.discover_container_name(card_id).await? {
            return Ok(name);
        }
        if start.elapsed() >= deadline {
            anyhow::bail!("container not discoverable after devcontainer up");
        }
        tokio::time::sleep(backoff).await;
        if backoff < std::time::Duration::from_millis(400) {
            backoff *= 2;
        }
    }
}
```

- [ ] **Step 2: Add `reeve-household` dep to `crates/reeve-workspace/Cargo.toml`**

(Needed for `reeve_household::HouseholdRepo` and `Household`.)

```toml
reeve-household = { path = "../reeve-household" }
```

- [ ] **Step 3: Update `crates/reeve-workspace/src/lib.rs`**

```rust
mod domain;
mod lifecycle;
mod repo;

pub use domain::{Container, Workspace};
pub use lifecycle::{ListenerHandle, ListenerOps, LiveWorkspaceLifecycle};
pub use repo::{init_schema, make_workspace_repo, ContainerRow, WorkspaceRepo};

pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;

pub use git::{make_git_ops, GitOps};
```

- [ ] **Step 4: Build**

```bash
cargo build --workspace
```

Expected: succeeds, but reeved's `effects.rs` still uses the old `TransitionEffects` trait. That's OK — we cut over in Task 4.5.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(4.4): LiveWorkspaceLifecycle in reeve-workspace impls WorkspaceLifecycle

Body ported from effects::LiveEffects, with direct DB calls replaced by
WorkspaceRepo/HouseholdRepo trait calls. ListenerOps trait abstracts
over reeved's per-card socket registry to avoid a workspace→reeved dep.
EOF
)"
```

### Task 4.5: Cut over reeved — wire LiveWorkspaceLifecycle, delete stubs + effects.rs

**Files:**
- Modify: `crates/reeved/src/main.rs` (build LiveWorkspaceLifecycle, pass into DaemonState)
- Modify: `crates/reeved/src/effects.rs` (delete; replace `DaemonState` definition with a thinner one in main.rs or app/state.rs)
- Modify: `crates/reeved/src/stubs.rs` (delete `StubWorkspaceLifecycle`; whole file empty → delete)
- Modify: `crates/reeved/src/transitions.rs` (delete; reeve-card now owns it)
- Modify: `crates/reeved/src/handlers.rs` (update card-handler arms to use real workspace_lifecycle; route attach/push to reeve-workspace handlers — see Task 4.6)
- Add: Impl `ListenerOps` on `transport::listener::ListenerRegistry` (in reeved)

- [ ] **Step 1: Impl `reeve_workspace::ListenerOps` for `ListenerRegistry`**

In `crates/reeved/src/transport/listener.rs`, add:

```rust
#[async_trait::async_trait]
impl reeve_workspace::ListenerOps for ListenerRegistry {
    async fn spawn(&self, card_id: &str, socket: std::path::PathBuf) {
        ListenerRegistry::spawn(self, card_id, socket).await;
    }
    async fn stop(&self, card_id: &str) -> anyhow::Result<()> {
        ListenerRegistry::stop(self, card_id).await
    }
}
```

Add `reeve-workspace` to `crates/reeved/Cargo.toml` dependencies (alongside the others).

- [ ] **Step 2: Update `crates/reeved/src/main.rs` to construct LiveWorkspaceLifecycle**

Where today we build `DaemonState`:

```rust
let workspaces = reeve_workspace::make_workspace_repo(db.clone());
let git = reeve_workspace::make_git_ops();
let containers = reeve_workspace::container::make_container_ops();
let scaffold = reeve_household::make_scaffold();
let listeners_arc: reeve_workspace::ListenerHandle = Arc::new(listeners.clone());

let workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle> = Arc::new(
    reeve_workspace::LiveWorkspaceLifecycle::new(
        workspaces.clone(),
        household_repo.clone(),
        git.clone(),
        containers.clone(),
        listeners_arc,
        claude_dir.clone(),
        api_key.clone(),
    )
);
```

Pass `workspace_lifecycle` into `DaemonState`.

- [ ] **Step 3: Update DaemonState in `crates/reeved/src/effects.rs`**

Replace its definition:

```rust
pub struct DaemonState {
    pub db: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    pub workspaces: Arc<dyn reeve_workspace::WorkspaceRepo>,
    pub household_repo: Arc<dyn reeve_household::HouseholdRepo>,
    pub household_lookup: Arc<dyn reeve_card::HouseholdLookup>,
    pub workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle>,
    pub git: Arc<dyn reeve_workspace::GitOps>,
    pub containers: Arc<dyn reeve_workspace::container::ContainerOps>,
    pub listeners: crate::transport::listener::ListenerRegistry,
    pub claude_dir: std::path::PathBuf,
    pub api_key: Option<String>,
}
```

Delete the old `LiveEffects` impl (it's now `LiveWorkspaceLifecycle` in reeve-workspace).

- [ ] **Step 4: Delete `crates/reeved/src/transitions.rs`**

```bash
git rm crates/reeved/src/transitions.rs
```

Remove `mod transitions;` from `crates/reeved/src/main.rs`. Replace any `use crate::transitions::*` with `use reeve_card::*` (the imports needed are `on_column_change`, `WorkspaceLifecycle`).

- [ ] **Step 5: Update `crates/reeved/src/handlers.rs` to use real workspace_lifecycle (no stub)**

In each card-handler dispatch arm that previously called `stub_workspace_lifecycle(state.clone())`, replace with `state.workspace_lifecycle.clone()`.

Delete the `stub_workspace_lifecycle` helper. Delete `crates/reeved/src/stubs.rs` (empty now):

```bash
git rm crates/reeved/src/stubs.rs
```

Remove `mod stubs;` from `crates/reeved/src/main.rs`.

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass. The old `effects::LiveEffects` tests may break — if so, delete them (the behaviour is covered by reeve-card transitions tests + reeve-workspace tests + e2e).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(4.5): wire LiveWorkspaceLifecycle; delete stubs + transitions + effects::LiveEffects

reeved/src/transitions.rs deleted (owned by reeve-card now).
reeved/src/stubs.rs deleted (real impls in reeve-workspace + reeve-household).
DaemonState gains workspace_lifecycle Arc.
ListenerRegistry impls reeve_workspace::ListenerOps for cross-crate use.
EOF
)"
```

### Task 4.6: Move workspace-side handlers (attach, push) into reeve-workspace

**Files:**
- Create: `crates/reeve-workspace/src/handler.rs`
- Modify: `crates/reeve-workspace/src/lib.rs`
- Modify: `crates/reeved/src/handlers.rs` (route attach/push; delete old bodies)

Port `handle_attach` and `handle_push` from today's `crates/reeved/src/handlers.rs` into `reeve-workspace/src/handler.rs`, taking ports as args.

- [ ] **Step 1: Create `crates/reeve-workspace/src/handler.rs`**

Adapt the bodies of `handle_attach` and `handle_push` from `crates/reeved/src/handlers.rs`. Signatures:

```rust
pub async fn handle_attach(
    workspaces: &dyn WorkspaceRepo,
    containers: &dyn ContainerOps,
    params: AttachParams,
) -> Result<Value, RpcError> { ... }

pub async fn handle_push(
    workspaces: &dyn WorkspaceRepo,
    households: &dyn reeve_household::HouseholdRepo,
    git: &dyn GitOps,
    cards: &dyn reeve_card::CardRepo,  // for looking up cards in implementing/testing/review
    params: PushParams,
) -> Result<Value, RpcError> { ... }
```

(Copy the existing handler bodies verbatim; substitute `state.X` accesses with the trait args. The exact line-by-line port depends on M4/M5 specifics — refer to today's handlers.rs for the source.)

- [ ] **Step 2: Update `crates/reeve-workspace/src/lib.rs`**

```rust
mod domain;
mod handler;
mod lifecycle;
mod repo;

pub use domain::{Container, Workspace};
pub use handler::{handle_attach, handle_push};
pub use lifecycle::{ListenerHandle, ListenerOps, LiveWorkspaceLifecycle};
pub use repo::{init_schema, make_workspace_repo, ContainerRow, WorkspaceRepo};

pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;

pub use git::{make_git_ops, GitOps};
```

- [ ] **Step 3: Update dispatch in `crates/reeved/src/handlers.rs` for `attach` and `push`**

In `dispatch()`, replace existing arms:

```rust
"attach" => {
    let params = parse_params::<reeve_protocol::AttachParams>(req.params)?;
    reeve_workspace::handle_attach(
        &*state.workspaces,
        &*state.containers,
        params,
    ).await
}
"push" => {
    let params = parse_params::<reeve_protocol::PushParams>(req.params)?;
    let cards = reeve_card::make_card_repo(state.db.clone());
    reeve_workspace::handle_push(
        &*state.workspaces,
        &*state.household_repo,
        &*state.git,
        &*cards,
        params,
    ).await
}
```

Delete the old `handle_attach` and `handle_push` function bodies from `crates/reeved/src/handlers.rs`.

- [ ] **Step 4: Replace inline migrate() containers-table init with reeve_workspace::init_schema**

In `crates/reeved/src/db.rs::migrate` (or wherever it lives now), replace the inline `CREATE TABLE containers ...` with a call to `reeve_workspace::init_schema(conn)?;`.

- [ ] **Step 5: Spawn fetch worker via reeve_workspace::fetch::run**

In `crates/reeved/src/main.rs`, replace any existing fetch-spawn call with:

```rust
let fetch_handle = tokio::spawn(reeve_workspace::fetch::run(
    state.workspaces.clone(),
    reeve_card::make_card_repo(db.clone()),
    state.git.clone(),
    fetch_shutdown_rx,
));
```

(Match the actual `fetch::run` signature; if today's fetch::run takes `state: &DaemonState`, adapt the function in `reeve-workspace/src/fetch.rs` to accept its specific ports instead.)

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass. The M4 e2e test (`tests/e2e.rs::fetch + push round-trip`) must pass. The M5 e2e test (`tests/e2e.rs::attach`) must pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(4.6): attach + push handlers + fetch worker now live in reeve-workspace

handle_attach and handle_push moved out of reeved/handlers.rs.
Fetch worker spawned via reeve_workspace::fetch::run with explicit
port arguments. Containers schema-init now via reeve_workspace::init_schema.
EOF
)"
```

### Task 4.7: Final cleanup — delete reeved/src/db.rs, reeved/src/effects.rs, reeved/src/lifecycle.rs

**Files:**
- Delete: `crates/reeved/src/db.rs`
- Delete: `crates/reeved/src/effects.rs`
- Modify: `crates/reeved/src/main.rs` (remove mod declarations)

- [ ] **Step 1: Verify reeved/src/db.rs is empty of useful exports**

```bash
git grep -l 'crate::db::' crates/reeved/src/
```

Expected: no matches (everything moved). If matches remain, fix them.

- [ ] **Step 2: Delete it**

```bash
git rm crates/reeved/src/db.rs
```

Remove `mod db;` from `crates/reeved/src/main.rs`.

- [ ] **Step 3: Repeat for effects.rs**

```bash
git grep -l 'crate::effects::' crates/reeved/src/
```

Should be no matches after Task 4.5. If `DaemonState` was kept in `effects.rs`, move it to `crates/reeved/src/app/state.rs` and update imports.

```bash
git rm crates/reeved/src/effects.rs
```

Remove `mod effects;` from `crates/reeved/src/main.rs`.

- [ ] **Step 4: Move lifecycle.rs → app/lifecycle.rs**

```bash
git mv crates/reeved/src/lifecycle.rs crates/reeved/src/app/lifecycle.rs
```

In `crates/reeved/src/app/mod.rs`:

```rust
pub mod lifecycle;
pub mod state;
```

Replace `use crate::lifecycle::*` with `use crate::app::lifecycle::*` across reeved.

- [ ] **Step 5: Move DaemonState → app/state.rs**

Create `crates/reeved/src/app/state.rs` with the `DaemonState` struct definition (last seen in `effects.rs`).

Update `use crate::effects::DaemonState` to `use crate::app::state::DaemonState`.

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(4.7): delete reeved/src/{db,effects}.rs; move lifecycle + state into app/

Per-context crates own their own DB code; effects is gone (was the
WorkspaceLifecycle impl, now in reeve-workspace). lifecycle.rs becomes
app/lifecycle.rs (paths, db_path, migrate orchestrator). DaemonState
lives in app/state.rs.
EOF
)"
```

---

## Stage 5 — Card γ type migration

Goal: introduce `reeve_card::domain::Card` as a typed entity with private fields, getters, and methods. Bridge to `reeve_protocol::Card` (the wire DTO) via `From`/`TryFrom`. Update `CardRepo` and handlers to use the domain type internally; convert at the wire boundary only.

### Task 5.1: Replace `pub use reeve_protocol::Card` with a typed domain entity

**Files:**
- Modify: `crates/reeve-card/src/domain.rs`

- [ ] **Step 1: Replace `crates/reeve-card/src/domain.rs` contents**

```rust
//! Card domain entity. Private fields + invariants + factory constructors.
//! Conversion to/from the wire DTO `reeve_protocol::Card` lives below.

use reeve_protocol::Column;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CardError {
    #[error("brief must be non-empty")]
    EmptyBrief,
    #[error("brief too long ({0} > 200 chars)")]
    BriefTooLong(usize),
    #[error("id must be non-empty")]
    EmptyId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    id: String,
    household: String,
    brief: String,
    column: Column,
    created_at: i64,
    updated_at: i64,
    needs_heavy: bool,
    parent_id: Option<String>,
}

impl Card {
    /// Create a fresh card in Backlog. Generates a UUID and sets timestamps.
    pub fn new(household: String, brief: String, needs_heavy: bool) -> Result<Self, CardError> {
        if brief.is_empty() {
            return Err(CardError::EmptyBrief);
        }
        let n = brief.chars().count();
        if n > 200 {
            return Err(CardError::BriefTooLong(n));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            household,
            brief,
            column: Column::Backlog,
            created_at: now,
            updated_at: now,
            needs_heavy,
            parent_id: None,
        })
    }

    // Getters
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn household(&self) -> &str {
        &self.household
    }
    pub fn brief(&self) -> &str {
        &self.brief
    }
    pub fn column(&self) -> Column {
        self.column
    }
    pub fn created_at(&self) -> i64 {
        self.created_at
    }
    pub fn updated_at(&self) -> i64 {
        self.updated_at
    }
    pub fn needs_heavy(&self) -> bool {
        self.needs_heavy
    }
    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    /// Construct a Card directly from validated fields. Used by repo when
    /// reading from persisted SQL rows.
    pub fn from_persisted(
        id: String,
        household: String,
        brief: String,
        column: Column,
        created_at: i64,
        updated_at: i64,
        needs_heavy: bool,
        parent_id: Option<String>,
    ) -> Result<Self, CardError> {
        if id.is_empty() {
            return Err(CardError::EmptyId);
        }
        if brief.is_empty() {
            return Err(CardError::EmptyBrief);
        }
        let n = brief.chars().count();
        if n > 200 {
            return Err(CardError::BriefTooLong(n));
        }
        Ok(Self {
            id,
            household,
            brief,
            column,
            created_at,
            updated_at,
            needs_heavy,
            parent_id,
        })
    }
}

// Domain → wire (always succeeds).
impl From<Card> for reeve_protocol::Card {
    fn from(c: Card) -> Self {
        reeve_protocol::Card {
            id: c.id,
            household: c.household,
            brief: c.brief,
            column: c.column,
            created_at: c.created_at,
            updated_at: c.updated_at,
            needs_heavy: c.needs_heavy,
            parent_id: c.parent_id,
        }
    }
}

// Wire → domain (re-runs invariants).
impl TryFrom<reeve_protocol::Card> for Card {
    type Error = CardError;
    fn try_from(d: reeve_protocol::Card) -> Result<Self, CardError> {
        Self::from_persisted(
            d.id,
            d.household,
            d.brief,
            d.column,
            d.created_at,
            d.updated_at,
            d.needs_heavy,
            d.parent_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire() -> reeve_protocol::Card {
        reeve_protocol::Card {
            id: "abc".into(),
            household: "h".into(),
            brief: "test".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        }
    }

    #[test]
    fn new_card_in_backlog() {
        let c = Card::new("h".into(), "brief".into(), false).unwrap();
        assert_eq!(c.column(), Column::Backlog);
        assert!(!c.id().is_empty());
        assert_eq!(c.brief(), "brief");
    }

    #[test]
    fn new_card_rejects_empty_brief() {
        let err = Card::new("h".into(), "".into(), false).unwrap_err();
        assert!(matches!(err, CardError::EmptyBrief));
    }

    #[test]
    fn new_card_rejects_overlong_brief() {
        let long = "x".repeat(201);
        let err = Card::new("h".into(), long, false).unwrap_err();
        assert!(matches!(err, CardError::BriefTooLong(201)));
    }

    #[test]
    fn protocol_to_domain_to_protocol_round_trips() {
        let dto = sample_wire();
        let entity = Card::try_from(dto.clone()).unwrap();
        let dto2: reeve_protocol::Card = entity.into();
        assert_eq!(dto, dto2);
    }

    #[test]
    fn try_from_rejects_invalid_persisted() {
        let mut bad = sample_wire();
        bad.brief = String::new();
        let err = Card::try_from(bad).unwrap_err();
        assert!(matches!(err, CardError::EmptyBrief));
    }
}
```

- [ ] **Step 2: Update `crates/reeve-card/src/lib.rs` to expose Card + CardError**

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod handler;
mod repo;
mod transitions;

pub use domain::{Card, CardError};
pub use handler::{
    handle_ls, handle_mv, handle_mv_self, handle_new, handle_status_self,
};
pub use repo::{init_schema, make_card_repo, CardRepo};
pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
```

- [ ] **Step 3: Build**

```bash
cargo build --workspace
```

Expected: **fails** — repo.rs and handler.rs still use `reeve_protocol::Card` directly. Stage continues in Task 5.2.

- [ ] **Step 4: Run reeve-card unit tests (excluding the broken downstream)**

```bash
cargo test -p reeve-card --lib domain
```

Expected: 5 tests pass (new_card_in_backlog, rejects_empty_brief, rejects_overlong_brief, round_trips, rejects_invalid_persisted).

- [ ] **Step 5: Commit (intermediate state — build is broken; fixed in 5.2)**

Important: this commit doesn't satisfy "build green every commit." Run task 5.1 and 5.2 back-to-back and commit only after 5.2 builds clean. So combine the two tasks into one commit.

If your workflow demands a checkpoint commit even when the workspace doesn't build, label it `[WIP]`:

```bash
git add crates/reeve-card
git commit -m "[WIP] refactor(5.1): introduce reeve_card::Card typed entity (build broken until 5.2)"
```

Otherwise skip this commit and proceed to 5.2; the combined commit happens there.

### Task 5.2: Update repo.rs and handlers to use domain::Card

**Files:**
- Modify: `crates/reeve-card/src/repo.rs`
- Modify: `crates/reeve-card/src/handler.rs`
- Modify: `crates/reeved/src/handlers.rs` (dispatch arms convert at the wire boundary)

- [ ] **Step 1: Update `CardRepo` trait signatures to use `crate::domain::Card`**

In `crates/reeve-card/src/repo.rs`, change every `Card` reference from `reeve_protocol::Card` to `crate::domain::Card`. Specifically:

```rust
use crate::domain::Card;  // replaces `use reeve_protocol::Card`
use reeve_protocol::{CardReference, Column};  // these stay
```

In every method signature: `Card` now refers to `crate::domain::Card`.

In `row_to_card`, replace the construction:

```rust
fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let column_str: String = row.get(3)?;
    let column = Column::from_wire_str(&column_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(InvalidColumn(column_str.clone())),
        )
    })?;
    Card::from_persisted(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        column,
        row.get(4)?,
        row.get(5)?,
        row.get::<_, i64>(6)? != 0,
        row.get(7)?,
    )
    .map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(InvalidPersistedCard(e.to_string())),
        )
    })
}

#[derive(Debug, thiserror::Error)]
#[error("invalid persisted card: {0}")]
struct InvalidPersistedCard(String);
```

In `insert`, the field accesses change to getters:

```rust
async fn insert(&self, card: &Card) -> Result<()> {
    let conn = self.conn.clone();
    let card = card.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = conn.blocking_lock();
        conn.execute(
            "INSERT INTO cards (id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                card.id(), card.household(), card.brief(),
                card.column().as_wire_str(), card.created_at(), card.updated_at(),
                card.needs_heavy() as i64, card.parent_id(),
            ],
        )?;
        Ok(())
    })
    .await?
}
```

- [ ] **Step 2: Update `crates/reeve-card/src/handler.rs` to convert at wire boundary**

Replace `use reeve_protocol::{Card, ...}` with `use crate::domain::Card; use reeve_protocol::{...without Card}`.

In `handle_new`:

```rust
pub async fn handle_new(
    cards: &dyn CardRepo,
    params: NewParams,
) -> std::result::Result<Value, RpcError> {
    let card = Card::new(params.household, params.brief, params.needs_heavy)
        .map_err(|e| RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: e.to_string(),
        })?;
    cards.insert(&card).await.map_err(internal)?;
    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(NewResult { card: dto }).map_err(internal)
}
```

In `handle_ls`:

```rust
pub async fn handle_ls(cards: &dyn CardRepo) -> std::result::Result<Value, RpcError> {
    let entities = cards.list().await.map_err(internal)?;
    let dtos: Vec<reeve_protocol::Card> = entities.into_iter().map(Into::into).collect();
    serde_json::to_value(LsResult { cards: dtos }).map_err(internal)
}
```

In `handle_mv` and `handle_mv_self`: similar — work with domain Card internally; `.into()` at the response boundary. Field accesses become getters: `existing.column` → `existing.column()`, `existing.household` → `existing.household()`.

`on_column_change` signature in `transitions.rs` still uses `reeve_protocol::Card`. Update it to use `crate::domain::Card`:

In `crates/reeve-card/src/transitions.rs`:
```rust
use crate::domain::Card;  // replace use reeve_protocol::Card
use reeve_protocol::Column;  // Column stays
```

And update `WorkspaceLifecycle::ensure_running(&self, card: &Card)` — now domain Card. This means `LiveWorkspaceLifecycle::ensure_running` in `reeve-workspace/src/lifecycle.rs` also needs adjusting: `card: &reeve_card::Card` instead of `&reeve_protocol::Card`. Its body uses `card.id()`, `card.household()` etc. (getters).

- [ ] **Step 3: Update reeve-workspace/src/lifecycle.rs to match domain::Card**

In `crates/reeve-workspace/src/lifecycle.rs`:

Replace `use reeve_protocol::{Card, ContainerStatus}` with `use reeve_card::Card; use reeve_protocol::ContainerStatus;`.

In `ensure_running`, replace `card.household` with `card.household()`, `card.id` with `card.id()`.

- [ ] **Step 4: Update handler.rs in reeve-workspace if any handler uses Card**

`handle_push` may take a card id and look up the card via CardRepo; if it ever holds a Card, it's now domain Card. Update accordingly.

- [ ] **Step 5: Update reeved/src/handlers.rs dispatch arms**

The dispatch arms currently pass through `MvResult { card: <wire Card> }`. With Card now being domain in reeve-card's handlers, the conversion happens inside reeve_card::handle_*. So the dispatch arms in reeved don't change — they still receive a `Value` back.

If anywhere in reeved you hold a `reeve_protocol::Card`, that still works. If you hold a `reeve_card::Card`, convert via `.into()` before serializing.

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass, including the round-trip test in domain.rs.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(5.1+5.2): Card γ migration — domain entity + wire DTO + boundary conversion

reeve_card::Card is a typed entity with private fields, getters,
factory constructors (new, from_persisted), and invariants enforced
at construction. From<Card> for protocol::Card (domain → wire) and
TryFrom<protocol::Card> for Card (wire → domain) bridge the boundary.

CardRepo signatures use domain::Card; SqliteCardRepo converts at the
row boundary via from_persisted. Handlers convert at the wire boundary
via .into(). transitions::on_column_change takes &domain::Card;
reeve-workspace's LiveWorkspaceLifecycle updated accordingly.

Round-trip test pins protocol → domain → protocol equivalence.
EOF
)"
```

---

## Stage 6 — Rename `Manifest.workspace` → `meta_repo`

Goal: rename the field with a `serde` alias for one release of back-compat. Update internal uses; the JSON file format gets a deprecation warning if the old name is seen.

### Task 6.1: Add the rename + alias + deprecation warning

**Files:**
- Modify: `crates/reeve-household/src/manifest.rs`

- [ ] **Step 1: Update the struct in `crates/reeve-household/src/manifest.rs`**

Find:

```rust
pub struct Manifest {
    pub workspace: String,
    ...
}
```

Replace with:

```rust
pub struct Manifest {
    /// Name of the repos[] entry that IS the meta-repo.
    /// Renamed from `workspace` (2026-05); old name accepted via alias for
    /// one release, will be removed.
    #[serde(rename = "meta_repo", alias = "workspace")]
    pub meta_repo: String,

    #[serde(default)]
    pub knowledge_base: Option<String>,

    pub repos: Vec<RepoEntry>,
}
```

Also rename the field-access method:

```rust
pub fn workspace_entry(&self) -> &RepoEntry {
    self.repos
        .iter()
        .find(|r| r.name == self.meta_repo)
        .expect("workspace_entry: validation ensures meta_repo matches a repos[].name")
}

pub fn siblings(&self) -> impl Iterator<Item = &RepoEntry> {
    self.repos.iter().filter(|r| r.name != self.meta_repo)
}
```

(Method name `workspace_entry` stays — semantically still correct as "the meta-repo entry." Renaming the method is out of scope.)

- [ ] **Step 2: Add a deprecation warning at load time**

In `load_manifest`, after parsing:

```rust
let body = std::fs::read_to_string(&path)
    .with_context(|| format!("read manifest at {}", path.display()))?;

// Detect the deprecated `workspace` field name and warn.
if body.contains("\"workspace\"") && !body.contains("\"meta_repo\"") {
    tracing::warn!(
        "manifest at {} uses deprecated field `workspace`; rename to `meta_repo`. \
         Alias support will be removed in a future release.",
        path.display()
    );
}

let manifest: Manifest = serde_json::from_str(&body)
    .with_context(|| format!("parse manifest at {}", path.display()))?;
```

- [ ] **Step 3: Update validation messages**

`validate()` references `m.workspace`. Replace all such references with `m.meta_repo`.

- [ ] **Step 4: Update all internal callers within reeve-household**

Run `git grep -n 'm.workspace\|\.workspace[^_]' crates/reeve-household/src/`

For each match, change `.workspace` to `.meta_repo` if it refers to the Manifest field. Be careful — `workspace_name` in `HouseholdSummary` is different (it's the protocol DTO field) and stays as `workspace_name` (renaming protocol DTO fields is out of scope).

In `handler.rs::summary_from`:

```rust
fn summary_from(row: &HouseholdRow, m: &Manifest) -> HouseholdSummary {
    HouseholdSummary {
        name: row.name.clone(),
        meta_repo_path: row.meta_repo_path.clone(),
        workspace_name: m.meta_repo.clone(),   // protocol field name unchanged
        knowledge_base: m.knowledge_base.clone(),
        repo_count: m.repos.len(),
    }
}
```

In `handle_household_new`, where it rewrites the manifest:

```rust
let old_workspace = m.meta_repo.clone();
if let Some(entry) = m.repos.iter_mut().find(|r| r.name == old_workspace) {
    entry.name = params.name.clone();
} else {
    return Err(...);
}
m.meta_repo = params.name.clone();
```

- [ ] **Step 5: Update the existing manifest tests**

In `crates/reeve-household/src/manifest.rs::tests`, update any test JSON literals that use `workspace` field name. Add a new test that the alias works:

```rust
#[test]
fn old_workspace_field_name_still_parses_as_alias() {
    let dir = tempdir().unwrap();
    let root = write_manifest(
        dir.path(),
        r#"{ "workspace": "ws", "repos": [{ "name": "ws" }] }"#,
    );
    let m = load_manifest(&root).unwrap();
    assert_eq!(m.meta_repo, "ws");
}
```

(Add `use tempfile::tempdir;` if not already imported.)

- [ ] **Step 6: Update reeve-workspace if it reads `m.workspace`**

```bash
git grep -n '\.workspace[^_]\|m\.workspace' crates/reeve-workspace/src/
```

For each, change to `.meta_repo`. Likely affected: `clones.rs::clone_all_for_card` (checks `entry.name != m.workspace` in `siblings()` — but `siblings()` already uses the field internally, so callers don't need changes).

- [ ] **Step 7: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass; new alias test passes.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(6): rename Manifest.workspace → Manifest.meta_repo

#[serde(rename = "meta_repo", alias = "workspace")] keeps existing
household.json files parseable; a tracing::warn fires when the old name
is encountered. Witan-household template repo will be updated
separately to use the new name unconditionally.

HouseholdSummary.workspace_name (protocol DTO field) unchanged —
renaming protocol fields is out of scope.
EOF
)"
```

---

## Stage 7 — Wire migration + cleanup

Goal: finalise composition root. Add a dedicated `dispatch.rs` instead of routing through the (now-shrunken) `reeved/src/handlers.rs`. Remove the now-empty `reeved/src/handlers.rs`.

### Task 7.1: Move dispatch out of handlers.rs into dispatch.rs

**Files:**
- Create: `crates/reeved/src/dispatch.rs`
- Delete: `crates/reeved/src/handlers.rs`
- Modify: `crates/reeved/src/main.rs`

- [ ] **Step 1: Create `crates/reeved/src/dispatch.rs`**

```rust
//! RPC method dispatch table. The phone directory of the daemon — one place
//! to see every supported RPC method and where it routes.

use crate::app::state::DaemonState;
use crate::transport::server::CallerContext;
use anyhow::Result;
use reeve_protocol::{RpcError, RpcRequest};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn parse_params<T: DeserializeOwned>(value: Value) -> std::result::Result<T, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError {
        code: RpcError::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
    })
}

fn require_card(caller: &CallerContext) -> std::result::Result<String, RpcError> {
    caller.card_id.clone().ok_or_else(|| RpcError {
        code: RpcError::FORBIDDEN,
        message: "self-scoped methods require per-card socket".into(),
    })
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

async fn fetch_container_status(
    state: &DaemonState,
    card_id: &str,
) -> std::result::Result<Option<reeve_protocol::ContainerStatus>, RpcError> {
    let row = state
        .workspaces
        .get_container(card_id)
        .await
        .map_err(internal)?;
    Ok(row.map(|r| r.status))
}

pub async fn dispatch(
    req: RpcRequest,
    state: &DaemonState,
    caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        // --- card aggregate ---
        "ls" => reeve_card::handle_ls(&*state.cards).await,
        "new" => {
            let p = parse_params::<reeve_protocol::NewParams>(req.params)?;
            reeve_card::handle_new(&*state.cards, p).await
        }
        "mv" => {
            let p = parse_params::<reeve_protocol::MvParams>(req.params)?;
            reeve_card::handle_mv(
                &*state.cards,
                &*state.household_lookup,
                &*state.workspace_lifecycle,
                p,
            ).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let p = parse_params::<reeve_protocol::MvSelfParams>(req.params)?;
            reeve_card::handle_mv_self(
                &*state.cards,
                &*state.household_lookup,
                &*state.workspace_lifecycle,
                card_id,
                p,
            ).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            let status = fetch_container_status(state, &card_id).await?;
            reeve_card::handle_status_self(&*state.cards, status, card_id).await
        }

        // --- household aggregate ---
        "household.add" => {
            let scaffold = reeve_household::make_scaffold();
            let p = parse_params::<reeve_protocol::HouseholdAddParams>(req.params)?;
            reeve_household::handle_household_add(&*state.household_repo, &*scaffold, p).await
        }
        "household.new" => {
            let scaffold = reeve_household::make_scaffold();
            let p = parse_params::<reeve_protocol::HouseholdNewParams>(req.params)?;
            reeve_household::handle_household_new(&*state.household_repo, &*scaffold, p).await
        }
        "household.ls" => reeve_household::handle_household_ls(&*state.household_repo).await,
        "household.rm" => {
            let p = parse_params::<reeve_household::NameParam>(req.params)?;
            reeve_household::handle_household_rm(&*state.household_repo, p).await
        }
        "household.show" => {
            let p = parse_params::<reeve_household::NameParam>(req.params)?;
            let cards = state.cards.for_household(&p.name).await.map_err(internal)?;
            reeve_household::handle_household_show(&*state.household_repo, cards, p).await
        }

        // --- workspace aggregate ---
        "attach" => {
            let p = parse_params::<reeve_protocol::AttachParams>(req.params)?;
            reeve_workspace::handle_attach(&*state.workspaces, &*state.containers, p).await
        }
        "push" => {
            let p = parse_params::<reeve_protocol::PushParams>(req.params)?;
            reeve_workspace::handle_push(
                &*state.workspaces,
                &*state.household_repo,
                &*state.git,
                &*state.cards,
                p,
            ).await
        }

        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}
```

- [ ] **Step 2: Update `crates/reeved/src/main.rs` to use `dispatch::dispatch`**

Replace the dispatcher Pin/Box wrappers + `handlers::dispatch` with:

```rust
mod dispatch;

// In main(), where server::run is called:
let state_for_dispatch = state.clone();
let dispatch_fn = move |req: RpcRequest, _db: DbHandle, caller: CallerContext| {
    let state = state_for_dispatch.clone();
    async move { crate::dispatch::dispatch(req, &state, caller).await }
};

reeve_transport::server::run(...).await
```

(The exact closure shape depends on what `server::run` expects — match the existing signature.)

- [ ] **Step 3: Delete `crates/reeved/src/handlers.rs`**

After confirming all routing is now in `dispatch.rs`:

```bash
git rm crates/reeved/src/handlers.rs
```

Remove `mod handlers;` from main.rs.

- [ ] **Step 4: Update `crates/reeved/src/app/state.rs` (the final DaemonState)**

```rust
//! Composition of port handles. Constructed once in main.rs.

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct DaemonState {
    pub db: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    pub cards: Arc<dyn reeve_card::CardRepo>,
    pub workspaces: Arc<dyn reeve_workspace::WorkspaceRepo>,
    pub household_repo: Arc<dyn reeve_household::HouseholdRepo>,
    pub household_lookup: Arc<dyn reeve_card::HouseholdLookup>,
    pub workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle>,
    pub git: Arc<dyn reeve_workspace::GitOps>,
    pub containers: Arc<dyn reeve_workspace::container::ContainerOps>,
    pub listeners: crate::transport::listener::ListenerRegistry,
    pub claude_dir: PathBuf,
    pub api_key: Option<String>,
}
```

- [ ] **Step 5: Final `crates/reeved/src/main.rs` shape**

Confirm `main.rs` is thin — just composition. Roughly 80–120 lines. The body should follow the abbreviated example in the spec §"End-to-end composition."

- [ ] **Step 6: Build and test**

```bash
cargo build --workspace
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(7): dispatch.rs is the phone directory; handlers.rs deleted

All RPC routing lives in reeved/src/dispatch.rs (~80 lines, one match
block over all 12 methods). The old reeved/src/handlers.rs is deleted.
main.rs is now ~100 lines of composition.
EOF
)"
```

---

## Stage 8 — Quality pass + CI lint

Goal: add the cross-context isolation lint, run `fmt`/`clippy`/full test suite, update CLAUDE.md.

### Task 8.1: CI lint for cross-context internal-path imports

**Files:**
- Create: `scripts/check-context-isolation.sh`
- Modify: CI workflow (path depends on what reeve uses; check `.github/workflows/` if it exists; otherwise document the script for manual + hook use)

- [ ] **Step 1: Create `scripts/check-context-isolation.sh`**

```bash
#!/usr/bin/env bash
# Fail if any source file imports a path that reaches past another context
# crate's lib.rs re-exports.
#
# Acceptable:  use reeve_card::Card;
#              use reeve_card::WorkspaceLifecycle;
# Forbidden:   use reeve_card::repo::SqliteCardRepo;
#              use reeve_card::repo::*;
#              use reeve_workspace::lifecycle::LiveWorkspaceLifecycle;
#
# Reeved (the composition root) is exempt — it may reach deeper if needed,
# though factories should make this unnecessary.

set -euo pipefail

# Crates whose internals are off-limits to other crates.
contexts=(card workspace household)

violations=$(git grep -nE 'use reeve_('"$(IFS='|'; echo "${contexts[*]}")"')::[a-z_]+::' \
    -- 'crates/reeve-card/src/*' 'crates/reeve-workspace/src/*' 'crates/reeve-household/src/*' \
    || true)

if [[ -n "$violations" ]]; then
    echo "Context-isolation violations detected:" >&2
    echo "$violations" >&2
    echo >&2
    echo "Cross-context imports must go through the crate's lib.rs facade." >&2
    echo "Move the path past mod.rs or add the needed type to lib.rs's pub use." >&2
    exit 1
fi

echo "Context isolation OK."
```

Make it executable:

```bash
chmod +x scripts/check-context-isolation.sh
```

- [ ] **Step 2: Test the lint catches a violation**

Temporarily introduce a bad import:

```bash
echo 'use reeve_card::repo::SqliteCardRepo;' >> crates/reeve-workspace/src/handler.rs
./scripts/check-context-isolation.sh
```

Expected: exit code 1, prints the violation.

Revert:

```bash
git checkout crates/reeve-workspace/src/handler.rs
./scripts/check-context-isolation.sh
```

Expected: exit code 0, prints "Context isolation OK."

- [ ] **Step 3: Wire into CI (if `.github/workflows/` exists)**

Check: `ls .github/workflows/`

If a workflow file exists, add a step:

```yaml
      - name: Check context isolation
        run: scripts/check-context-isolation.sh
```

If no CI exists today, document the script in the project README or in the new CLAUDE.md (Task 8.3) so it gets run pre-commit or pre-PR.

- [ ] **Step 4: Commit**

```bash
git add scripts/check-context-isolation.sh .github/workflows/*.yml 2>/dev/null || true
git add scripts/check-context-isolation.sh
git commit -m "$(cat <<'EOF'
build(8.1): CI lint for cross-context internal-path imports

scripts/check-context-isolation.sh fails if any reeve-card /
reeve-workspace / reeve-household source file imports a deep path
(use reeve_X::submodule::Type) from another context crate. Belt and
braces alongside compile-time pub(crate) enforcement.
EOF
)"
```

### Task 8.2: Quality pass — fmt, clippy, full tests, e2e

- [ ] **Step 1: Format**

Run: `cargo fmt --all`

Commit if there are any formatting changes:

```bash
git add -A
git diff --cached --quiet || git commit -m "chore: cargo fmt residue from refactor"
```

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Fix any new lints (the refactor may have introduced some — e.g., unused imports, `dead_code` in old files we forgot to delete).

```bash
git add -A
git diff --cached --quiet || git commit -m "chore: clippy fixes from refactor"
```

- [ ] **Step 3: Full test suite**

Run: `cargo test --workspace`

Expected: all tests pass.

- [ ] **Step 4: Run e2e tests (require docker)**

Run: `cargo test --test e2e -- --include-ignored` (or whatever the project's convention is for docker-gated tests)

Verify these tests pass:
- M3 e2e (clones + devcontainer)
- M4 e2e (fetch + push round-trip)
- M5 e2e (attach — tmux session present in container after mv → implementing)

- [ ] **Step 5: Run context-isolation lint**

```bash
./scripts/check-context-isolation.sh
```

Expected: "Context isolation OK."

- [ ] **Step 6: Commit (if anything pending)**

If steps 1–5 produced any deltas:

```bash
git add -A
git commit -m "chore(8.2): quality pass after refactor — fmt/clippy/tests/lint"
```

### Task 8.3: Update CLAUDE.md with new architectural rules

**Files:**
- Modify: `CLAUDE.md` (project root, if it exists; otherwise create it)

- [ ] **Step 1: Add a section "Architecture" to CLAUDE.md**

Append (or create) `CLAUDE.md` with:

```markdown
## Architecture

Reeve uses **vertical-slice per-context crates** with **inverted-dependency ports**.

### Crates

- `reeve-protocol` — wire DTOs, paths::, lifecycle helpers. No daemon logic.
- `reeve-card` — Card aggregate. Defines `WorkspaceLifecycle` and `HouseholdLookup` port traits (consumed by Card; supplied by workspace/household crates).
- `reeve-workspace` — Workspace aggregate. Impls `WorkspaceLifecycle`. Owns git/container/fetch adapters.
- `reeve-household` — Household aggregate. Impls `HouseholdLookup`. Owns Manifest + scaffold.
- `reeved` — daemon binary. Composition root, dispatch table, transport sub-module.
- `reeve` — CLI + TUI client. Depends only on `reeve-protocol`.

### Architectural rules

- **Cross-context coupling only through ports.** Defining a port lives with the *consumer*; implementing it lives with the *supplier*. Suppliers depend on consumers — never the other way.
- **Adapters are `pub(crate)`.** Every `Sqlite*Repo`, `Real*`, `Docker*` struct is crate-private. Cross-crate consumers get instances only via factory functions returning `Arc<dyn Trait>`.
- **Domain has no infra deps** (convention within each crate). Files named `domain.rs` and `transitions.rs` must not `use rusqlite`, `tokio::process`, or `docker`-related types.
- **One bounded context per crate.** Don't add cross-cutting "shared" code to one of the context crates; put genuinely-shared infra in `reeve-protocol` or a new `reeve-shared` (only after rule of three).

### Enforcement

- Cargo.toml dependencies are the crate-level firewall.
- `pub(crate)` on adapters is the compile-level enforcement.
- `scripts/check-context-isolation.sh` is a CI grep lint catching cross-context internal-path imports (`use reeve_X::submodule::*`).

### Adding a new RPC method

1. Decide which aggregate owns it. Add the handler in that crate's `handler.rs`.
2. Re-export from that crate's `lib.rs`.
3. Add a match arm in `crates/reeved/src/dispatch.rs` routing the method name to the handler.

### Refactor history

See `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md` for the
design that produced this structure.
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "$(cat <<'EOF'
docs(8.3): document architecture in CLAUDE.md

Per-context crates, inverted-dep ports, pub(crate) adapters, CI lint.
Adding a new RPC: choose aggregate → handler → re-export → dispatch arm.
EOF
)"
```

---

## Self-review (against the spec)

Quick sanity check before handoff.

**Spec coverage:**

- §"Six crates" → Stages 0–4 create exactly: reeve-card, reeve-workspace, reeve-household, plus the new sub-modules in reeved. ✓
- §"Dependency rules" → Cargo.toml deps in Tasks 0.1, 2.5, 3.6, 4.5 set up the exact arrows from spec. ✓
- §"Enforcement of 'use the port, not the impl'" → Factory functions in Tasks 2.2, 3.3, 3.4, 4.1, 4.3 implement the pattern. Adapter structs are `pub(crate)`. ✓
- §"CI lint" → Task 8.1. ✓
- §"Inverted-dependency ports" → `WorkspaceLifecycle` + `HouseholdLookup` in Task 2.1 (reeve-card); impls in Tasks 3.3 (HouseholdLookup) and 4.4 (WorkspaceLifecycle). ✓
- §"Workspace size: co-edit map" → Documented in spec, not a code change; nothing to plan-implement. ✓
- §"Type strategy applied to all three aggregates" → Card γ in Stage 5; Workspace + Household domain entities in Tasks 3.2 and 4.2. ✓
- §"Schema independence — no cross-context FKs" → FK previewed-dropped in Task 3.6; definitively dropped in Task 4.3 (init_schema explicitly omits it); pinned by test `no_fk_no_cards_row_needed`. ✓
- §"Manifest field migration" → Stage 6. ✓
- §"End-to-end composition" → Task 7.1 (dispatch.rs + final main.rs shape). ✓
- §"Eight stages" → Stages 0–8 (Stage 0 is the crate skeletons setup that precedes the spec's stage 1; spec's stages 1–8 → plan's stages 1–8). The plan reorders the *body* of stages 2 vs 3 (card before household) for execution correctness; design unchanged. ✓
- §"Testing strategy" → Each task adds tests for new code; old tests delete when their function is moved; e2e tests stay as the regression net (Task 8.2 runs them). ✓

**Placeholder scan:**

- No "TBD/TODO" tokens.
- "Adapt from today's `crates/reeved/src/effects.rs`" appears in Tasks 4.4 and 4.6 — this is intentional (the bodies are too long to copy verbatim and the source still exists in git at that point). Engineer can `git show` to read them.

**Type-consistency check:**

- `CardRepo` signatures consistent across Tasks 2.2, 5.2.
- `WorkspaceLifecycle`/`HouseholdLookup` signatures consistent across Tasks 2.1, 3.3, 4.4.
- `make_card_repo`/`make_workspace_repo`/`make_household_components`/`make_scaffold`/`make_git_ops`/`make_container_ops` factory names used consistently.
- `init_schema` exported by all three context crates; orchestrated in `reeved/src/app/lifecycle.rs::migrate`.

**Known sharp edges in execution:**

1. **Stage 5 (Card γ) breaks the build between Tasks 5.1 and 5.2.** Solution noted in Task 5.1, Step 5: combine into one logical commit, or use `[WIP]` if you really need an intermediate checkpoint.
2. **Stage 4 has the most file movement** (3 git mv operations in Task 4.1 alone). Each sub-step builds + commits to keep the diff readable. If any cargo build fails mid-step, fix imports before moving to the next.
3. **`handle_attach` and `handle_push` bodies (Task 4.6) reference M4/M5 specifics not detailed here.** The engineer should `git show` the relevant commits (`80ef6c4`, `02a5d10`, `2c29619`, `bf27d32`) for the exact bodies and adapt to take port args instead of `DaemonState`.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-18-domain-driven-refactor.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best for a plan this size (40+ tasks across 8 stages).

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints. Slower but everything stays in one conversation.

Which approach?


