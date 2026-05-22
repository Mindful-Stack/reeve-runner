---
title: Visibility modifiers — pub, pub(crate), pub(super), pub(in path)
description: How Reeve uses Rust's visibility modifiers to enforce architectural rules at compile time. pub for crate API; pub(crate) for adapters that should be unnameable outside the crate; pub(super) and pub(in path) for tighter scoping inside a crate.
tags: [languages, rust, visibility, encapsulation, standards]
---

# Visibility modifiers

Rust's visibility system is a load-bearing piece of Reeve's architecture, not a styling choice. The `pub(crate)` discipline on adapters is what compile-enforces "use the port, not the impl" — without it, the architecture would be a code-review convention. This document is the spec for which modifier to use when.

## The hierarchy (from most to least restrictive)

| Modifier | Means | Use case in Reeve |
|---|---|---|
| (default, no modifier) | Private to the module | Helper fns, internal types, struct fields under encapsulation |
| `pub(in path)` | Visible to the specified path | Rare — when you want to scope to a sub-tree but not the whole crate |
| `pub(super)` | Visible to the parent module | When a sub-module wants to expose something only to its sibling sub-modules |
| `pub(crate)` | Visible within the crate | **Adapters** (`Sqlite*Repo`, `DockerOps`, `RealGitOps`, etc.) — invisible outside the crate boundary |
| `pub` | Public — visible to any consumer | Trait definitions, factory functions, domain entities, error types |

## The two big rules

### Adapters are `pub(crate)`

Every concrete implementation of a port trait — every `Sqlite*Repo`, `Real*`, `Docker*` — is `pub(crate)`. The factory function (`make_*_repo`, `make_*_ops`) returns `Arc<dyn Trait>`, which is the only handle external callers can hold.

```rust
// reeve-card/src/repo.rs
pub trait CardRepo: Send + Sync { ... }        // ← pub: the contract
pub(crate) struct SqliteCardRepo { ... }       // ← pub(crate): the impl
pub fn make_card_repo(conn) -> Arc<dyn CardRepo> { ... }  // ← pub: the factory
pub fn init_schema(conn: &Connection) -> Result<()> { ... }   // ← pub: the schema setup
```

`SqliteCardRepo::new` is also `pub(crate)` — only the factory can call it. This means:

- External crates **cannot name** `SqliteCardRepo`. The type doesn't exist in their namespace.
- External crates **cannot construct** a `SqliteCardRepo` even if they could name it.
- The only way to obtain a working `CardRepo` from outside `reeve-card` is `make_card_repo(...)`. The compiler enforces this.

Why this matters: see [[frameworks/rust/repo-port-adapter]] §"Why pub(crate) on the struct".

### Domain entities have private fields

Fields on domain entities (`reeve_card::Card`, `reeve_workspace::Workspace`, `reeve_household::Household`) are private. Public access is through getters and factory constructors.

```rust
// reeve-card/src/domain.rs
pub struct Card {
    id: String,            // private
    household: String,     // private
    brief: String,         // private
    column: Column,        // private
    // ...
}

impl Card {
    pub fn new(...) -> Result<Self, CardError> { ... }      // factory constructor
    pub fn from_persisted(...) -> Result<Self, CardError> { ... }
    pub fn id(&self) -> &str { &self.id }                   // getter
    pub fn column(&self) -> Column { self.column }
    // ...
}
```

This is what makes invariants enforceable. A caller can't construct a `Card` with `brief = ""`; the only constructors run validation. A caller can't mutate `brief` post-construction (no `set_brief` method exists). The Card aggregate's contract is total.

Wire DTOs are the opposite: `reeve_protocol::Card`'s fields are all `pub`, no constructor enforces invariants. This is deliberate — serde needs the fields public to deserialize, and validation happens at the wire/domain boundary via `TryFrom`. See [[frameworks/rust/from-tryfrom-boundary-conversion]].

## When to use `pub(super)`

Rare. The use case is: a sub-module wants to expose something to its parent module (and possibly the sibling sub-modules underneath the same parent), without making it available to the whole crate.

In Reeve, this comes up when a context crate has a sub-tree like:

```
reeve-workspace/src/
├── lib.rs
├── domain.rs
├── repo.rs
├── container/         (hypothetical multi-file submodule)
│   ├── mod.rs
│   ├── ops.rs
│   └── reconcile.rs
```

If `reconcile.rs` needs a helper from `ops.rs`, and the helper shouldn't be visible to the rest of `reeve-workspace`, use `pub(super)` in `ops.rs`. The helper is visible from `container::*` siblings but not from `repo.rs` etc.

Currently Reeve's modules are mostly single-file, so `pub(super)` doesn't appear much. If you find yourself wanting to expose something to one sibling and only that sibling, use `pub(in crate::container)` (the `pub(in path)` form) for explicit scoping.

## When to use `pub(in path)`

Even rarer. Use when you want a type visible within a specific module path but not anywhere else in the crate. Example (not currently in Reeve):

```rust
// reeve-workspace/src/container/internals.rs
pub(in crate::container) struct InternalThing { ... }
```

`InternalThing` is visible to anything inside `crate::container::*` but not to `reeve-workspace`'s other modules. Useful when a sub-tree has its own internal abstractions that shouldn't leak to siblings.

If you're reaching for `pub(in path)`, double-check that splitting into a different module wouldn't be cleaner. Excessive `pub(in path)` is a code smell — usually the right answer is to refactor the module hierarchy so default-private and `pub(crate)` are sufficient.

## What gets `pub` (and only that)

In a context crate's `lib.rs`, the public surface is:

- **Domain types** — `pub use domain::{Card, CardError};`
- **Port traits** — `pub use repo::CardRepo;` (the trait, not the struct)
- **Cross-aggregate ports defined here** — `pub use transitions::{HouseholdLookup, WorkspaceLifecycle};`
- **Factory functions** — `pub use repo::make_card_repo;`
- **Schema init** — `pub use repo::init_schema;`
- **Handler functions** — `pub use handler::{handle_ls, handle_mv, ...};`
- **The cross-aggregate policy fn** — `pub use transitions::on_column_change;`

Everything else (adapter structs, helpers, error mapping fns, the private modules themselves) stays out of `pub use`. The `mod` declarations are unqualified (private), so the modules can't be reached from outside via `crate::reeve_card::repo::*` either.

## Common mistakes

- **`pub` everywhere by default.** Tempting in early-stage code ("might need this later"). Resist. Use `pub(crate)` until an external caller actually needs the item; promote when needed. Demoting later (after some consumer started using `pub` types) is harder.
- **`pub` fields on domain entities.** Defeats encapsulation. Use getters; the invariant guarantees become observable.
- **`pub(crate)` on what should be `pub`.** A trait you want consumers to implement must be `pub` — `pub(crate)` makes it unnameable outside, which prevents impls. (Reeve's port traits are all `pub` for this reason.)
- **`pub use` of an adapter struct.** Re-exporting `pub use repo::SqliteCardRepo;` from `lib.rs` undoes the `pub(crate)` hiding. Only re-export traits + factories + schema init.
- **Forgetting that `mod` (without `pub`) is private.** `mod foo;` makes `foo` reachable as `crate::foo` from within the crate but unreachable from outside. `pub mod foo;` would expose it externally — usually undesirable for context-crate internals.

## Auditing visibility

Quick checks before merging:

1. **`grep -n 'pub struct' src/repo.rs src/git.rs src/container.rs src/scaffold.rs`** — adapter files. Every concrete struct should be `pub(crate)`, not bare `pub`.
2. **`grep -n 'pub use ' src/lib.rs`** — should re-export only traits, factories, schema-init, handlers, and domain entities. No `pub use repo::SqliteCardRepo` etc.
3. **`grep -n 'pub [a-z_]\+:' src/domain.rs`** — domain entity fields should not be `pub` (except wire DTOs in `reeve-protocol`, where serde needs them).
4. **`scripts/check-context-isolation.sh`** — catches `use reeve_X::submodule::*` from outside the defining crate (i.e., consumers reaching past the lib.rs facade).

## See also

- [[frameworks/rust/repo-port-adapter]] — the canonical use of `pub(crate)` on adapters.
- [[frameworks/rust/inverted-dependency-ports]] — pub traits defined in consumer crates; pub(crate) impls in supplier crates.
- [[general/architecture]] §"Enforcement" — the three layers (Cargo.toml + pub(crate) + CI lint).
