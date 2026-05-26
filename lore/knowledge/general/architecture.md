---
title: Architecture — vertical-slice bounded contexts with inverted-dependency ports
description: The high-level rules every Reeve change must follow. Six crates, one bounded context per crate (except reeved which composes them), inverted-dependency ports between contexts, pub(crate) adapters with factory functions, schema independence, CI lint to enforce facade discipline.
tags: [general, architecture, ddd, hexagonal, rust, standards]
---

# Architecture

This is the source of truth for how Reeve is organised and *why*. Every change should respect these rules; the CLAUDE.md at the repo root and the per-crate `lib.rs` re-exports are the day-to-day operationalisation, this document is the explanation.

## The six crates

| Crate | Role |
|---|---|
| `reeve-protocol` | Wire DTOs (`Card`, `Column`, `ContainerStatus`, etc.), `paths::`, lifecycle helpers. No daemon-side logic. Depended on by every other crate. |
| `reeve-card` | The [[domain/card]] aggregate. Kanban metadata + column-transition policy. *Defines* the `WorkspaceLifecycle` and `HouseholdLookup` ports (consumed by Card, supplied by others). |
| `reeve-workspace` | The [[domain/workspace]] aggregate. Per-card materialization: clones, container, fetch worker. *Implements* `WorkspaceLifecycle`. Depends on `reeve-card`. |
| `reeve-household` | The [[domain/household]] aggregate. Manifest, scaffold, repos. *Implements* `HouseholdLookup`. Depends on `reeve-card`. |
| `reeved` | The daemon binary. Composition root (`app/state.rs`, `app/lifecycle.rs`), transport sub-module (`transport/server.rs`, `transport/listener.rs`), dispatch table (`dispatch.rs`), startup reconcile (`reconcile.rs`). Depends on all four crates above. |
| `reeve` | CLI + TUI client. Depends *only* on `reeve-protocol`. |

The dependency graph at the crate level forms a DAG with `reeve-card` at the centre of the "domain" layer:

```
reeve-protocol
   ↑           ↑           ↑
reeve-card  ←─ reeve-workspace
   ↑       ←─ reeve-household
   |
reeved (composition root)
```

Adding a forbidden dep (e.g. `reeve-card → reeve-workspace`) does not compile until Cargo.toml is updated, which is itself a deliberate, reviewable act.

## The four-file pattern (per context crate)

Each of `reeve-card`, `reeve-workspace`, `reeve-household` follows the same internal shape:

```
src/
├── lib.rs        ← facade: pub re-exports + factory functions
├── domain.rs     ← entity + invariants + factory constructors (no rusqlite/tokio)
├── repo.rs       ← trait XxxRepo + pub(crate) SqliteXxxRepo + factory + init_schema
└── handler.rs    ← thin RPC handlers taking port references as args
```

Workspace adds `git.rs`, `clones.rs`, `container.rs`, `fetch.rs`, `lifecycle.rs` (adapter files).
Household adds `manifest.rs`, `scaffold.rs`.
Card adds `transitions.rs` (the kanban policy + cross-aggregate port traits).

See [[frameworks/rust/repo-port-adapter]] for the pattern's mechanics.

## The five rules

### 1. Cross-context coupling only through ports

A port (trait) lives with its *consumer*. Its implementations live with the *supplier*. So:

- `WorkspaceLifecycle` is defined in `reeve-card/src/transitions.rs` and implemented by `LiveWorkspaceLifecycle` in `reeve-workspace/src/lifecycle.rs`. **`reeve-workspace` depends on `reeve-card`**, not the other way around.
- `HouseholdLookup` is defined in `reeve-card/src/transitions.rs` and implemented by `SqliteHouseholdRepo` in `reeve-household/src/repo.rs`. **`reeve-household` depends on `reeve-card`**.

This is the **inverted-dependency** pattern, deliberately encoded in the crate graph. See [[frameworks/rust/inverted-dependency-ports]] for the reasoning.

### 2. Adapters are `pub(crate)`; access via factory functions

Every `Sqlite*Repo`, `Real*`, `Docker*` struct is `pub(crate)` — invisible outside its defining crate. The only way to obtain a working adapter is a factory function returning `Arc<dyn Trait>`:

```rust
// reeve-card/src/repo.rs
pub trait CardRepo: Send + Sync { ... }
pub(crate) struct SqliteCardRepo { ... }

pub fn make_card_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardRepo> {
    Arc::new(SqliteCardRepo::new(conn))
}
```

`reeve-workspace` literally cannot name `SqliteCardRepo`. The compiler enforces "use the trait, not the impl." Factories: `make_card_repo`, `make_workspace_repo`, `make_household_components` (returns the tuple `(HouseholdRepo, HouseholdLookup)` over one struct), `make_scaffold`, `make_git_ops`, `make_container_ops`.

### 3. Domain has no infra deps

Files named `domain.rs` and `transitions.rs` must not `use rusqlite`, `tokio::process`, or `docker`-related types. This is convention within a crate (the compiler doesn't enforce it; the CI isolation lint catches the cross-crate version — see the "Enforcement" section further down). If you find yourself wanting to add such an import, you're probably implementing a port and should move the work into `repo.rs`, `git.rs`, or wherever the side effects belong.

### 4. One bounded context per crate

Don't put cross-cutting "shared" code into one of the context crates. Genuinely-shared infra (path resolvers, lifecycle helpers) lives in `reeve-protocol` (`paths::`, `lifecycle::`). After hitting the rule of three (a third consumer of the same shape), consider creating a `reeve-shared` crate — until then, accept the modest duplication (e.g., `clone_url` exists in both `reeve-workspace::git::GitOps` and `reeve-household::scaffold::HouseholdScaffold`).

### 5. Type strategy

Card uses the **γ pattern**: a domain entity in `reeve-card` with private fields + `From`/`TryFrom` to the wire DTO `reeve_protocol::Card`. Workspace and Household are domain-only — they're never transmitted whole; wire-side they appear as read-model projections (`HouseholdSummary`, `StatusSelfResult.container_status`). See [[frameworks/rust/from-tryfrom-boundary-conversion]] for the mechanics.

## Schema independence

Each context owns its tables. No cross-context FKs. Schema-init is delegated to per-crate `init_schema` functions, orchestrated by `reeved/src/app/lifecycle.rs::migrate`:

```rust
pub fn migrate(conn: &Connection) -> Result<()> {
    reeve_card::init_schema(conn)?;       // cards table
    reeve_household::init_schema(conn)?;  // households table
    reeve_workspace::init_schema(conn)?;  // containers table (no FK to cards)
    Ok(())
}
```

The previous FK `containers.card_id REFERENCES cards.id ON DELETE CASCADE` was deliberately dropped during the refactor. Reasons:

1. `cargo test -p reeve-workspace` against a real (non-mocked) SQLite needs to insert a containers row without a corresponding cards row — the FK leaked cross-context coupling into test setup.
2. The CASCADE was theoretical — Reeve has no card-delete RPC that's blocked by the absence of the constraint (delete handles teardown explicitly).
3. The "does the card exist?" check naturally belongs to `LiveWorkspaceLifecycle::ensure_running`, which already reads the card from the DB before provisioning.

The `no_fk_no_cards_row_needed` test in `reeve-workspace/src/repo.rs::tests` pins the absence of the constraint.

## Concurrency invariants

Two cross-cutting concurrency rules every handler must honour:

### Per-card serialization for column-state-changing RPCs

All RPCs that change a card's column (or that delete a card) must acquire `state.lock_card(&card_id).await` *before* reading the card and *hold it until* both DB write and lifecycle side effects complete. Currently this applies to `mv`, `mv-self`, `delete`. Cross-card concurrency is unconstrained — two cards can mv simultaneously without coordination.

The lock is keyed by `card_id`; the registry is a `Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>` (or similar) on `DaemonState`. New mv arrives → lookup-or-create the per-card mutex → lock it. The mutex auto-releases on drop. Adding a new column-mutating RPC means adding the lock acquisition; forgetting it can produce orphan containers (e.g. two `mv → Implementing` calls racing into `ensure_running`). Introduced in commit `53dcb5d`.

### Lifecycle runs before column commit

For column-changing RPCs, the order is:

1. Acquire per-card lock.
2. Read existing card.
3. Run pre-checks (e.g. `HouseholdLookup::exists` for `mv → Implementing`).
4. **Run `WorkspaceLifecycle::on_column_change(old, new, card, ...)` — this is the side-effectful step.**
5. *Only on success*, `CardRepo::update_column(...)` commits the new column.
6. Release lock.

If step 4 fails, step 5 doesn't run. The card stays in its old column with the error propagated. This avoids the "card says Implementing but provisioning never happened" failure mode. Introduced in commit `fe52675`. See [[domain/card]] §"Invariants" for the matching invariant.

## Idempotency

Two idempotency invariants worth knowing about:

- **`clone_all_for_card` is idempotent.** It checks `<workspace>/.git` and short-circuits if the workspace is already cloned. The destructive `cleanup()` helper that used to wipe the workspace on retry was dropped (commit `9150ba3`) — it caused agent work to be lost.
- **Container teardown is idempotent.** `ensure_stopped` handles the "no container row" case (Card moved Backlog → Done with no prior Implementing) and the "container row exists but docker says it's gone" case (daemon crashed mid-spawn). Both return Ok cleanly.

## Manifest field deprecation policy

The Manifest field renamed from `workspace` to `meta_repo` in 2026-05. The original alias-and-deprecation-warning phase ran from 2026-05 through 2026-05-25; the alias was then dropped across all three Witan tools (Reeve, Lorekeeper, witan-household). Manifests using the old `workspace` field name no longer parse — they must be migrated with `sed -i 's/"workspace":/"meta_repo":/' household.json`. The pattern generalises: rename-with-alias for one release, then drop. See the 2026-05-25 spec for the cross-tool rollout details.

## Handler ownership of time

Repository methods that update timestamps take `now: i64` as an argument. They do NOT call `SystemTime::now()` themselves. The handler computes `now_secs()` once at the start of the operation and passes the same value through every write. Reasons:

1. **Deterministic tests** — a test can pass `now=1000` and assert on exact updated_at values.
2. **Single source of "now"** — atomic operations (mv + lifecycle + column update) all carry the same timestamp.
3. **Surfaces the time decision at the boundary** — handlers are the only callers of `now_secs()`.

Introduced via `update_brief(now: i64)` in commit `2a4169d`; pattern applied across all column-changing repo methods.

## Enforcement

Three layers of "do not violate these rules":

1. **Cargo.toml dependencies** are the crate-level firewall. Adding a forbidden dep does not compile.
2. **`pub(crate)` on adapters** compile-enforces "use the port, not the impl" at the crate boundary.
3. **`scripts/check-context-isolation.sh`** is a CI grep lint catching cross-context internal-path imports (`use reeve_X::submodule::Type`). Run via `make doctor` or before pushing. Even reeved (the composition root) should not reach past `mod.rs` of context crates — the factory functions are the only intended entry points.

Three rules, three layers: dep graph (compiler enforces crate boundaries), `pub(crate)` (compiler enforces type opacity), CI lint (catches the residual class of cross-context internal imports that compile but shouldn't exist).

## Adding a new RPC method

1. Decide which aggregate owns the operation. Add the handler in that crate's `handler.rs`.
2. Re-export it from that crate's `lib.rs`.
3. Add a match arm in `crates/reeved/src/dispatch.rs` routing the method name to the handler.
4. If the operation changes a card's column state, the dispatch arm must acquire `state.lock_card(&id).await` (per the per-card serialization rule above).

## Adding a new cross-aggregate port

1. Define the trait in the *consumer* crate (the one that calls it). For ports needed by Card, that's `reeve-card/src/transitions.rs`.
2. Add a `pub use` for the trait in the consumer's `lib.rs`.
3. Implement the trait in the *supplier* crate. The supplier already depends on the consumer (or adds the Cargo.toml dep deliberately).
4. Wire the implementation in `reeved/src/main.rs` (build the concrete via the supplier's factory, hand it to whatever consumer needs it).

## See also

- [[frameworks/rust/repo-port-adapter]] — the trait + pub(crate) struct + factory mechanics.
- [[frameworks/rust/inverted-dependency-ports]] — why the consumer defines the trait.
- [[frameworks/rust/from-tryfrom-boundary-conversion]] — the γ type pattern.
- [[frameworks/rust/async-over-sync-rusqlite]] — the `spawn_blocking` + `blocking_lock` pattern that lives inside every Sqlite*Repo.
- [[general/testing]] — how each crate's tests stay independent and what mocks live where.
- [[general/commit-conventions]] — how refactor steps surface in `git log` and how agent-coauthored commits are attributed.
- [[languages/rust/visibility]] — the `pub`/`pub(crate)`/`pub(super)` modifier rules these architectural choices depend on.
- [[domain/card]] / [[domain/workspace]] / [[domain/household]] — the three bounded contexts these rules organise.
