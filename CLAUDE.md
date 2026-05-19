# Reeve — Architecture & Conventions

This file is the entry point for agents (and humans) reading the Reeve daemon codebase.
Spec lives at `docs/superpowers/specs/2026-04-29-reeve-design.md`; per-milestone designs
and the recent architecture refactor are alongside it.

## Architecture

Reeve uses **vertical-slice per-context crates** with **inverted-dependency ports**.
Each bounded context owns its domain, persistence, and RPC entry points.

### Crates

- `reeve-protocol` — wire DTOs (`Card`, `Column`, `ContainerStatus`, etc.), `paths::`,
  and lifecycle helpers. No daemon-side logic. Both `reeved` and the `reeve` CLI/TUI
  depend on it.
- `reeve-card` — **Card aggregate**. Kanban metadata + column-transition policy.
  Defines the two cross-aggregate port traits consumed by Card (`WorkspaceLifecycle`,
  `HouseholdLookup`).
- `reeve-workspace` — **Workspace aggregate**. Per-card materialization: git clones,
  docker/devcontainer ops, fetch worker, the `LiveWorkspaceLifecycle` impl of
  `reeve_card::WorkspaceLifecycle`.
- `reeve-household` — **Household aggregate**. Registered project bundle: manifest,
  scaffold, repos. Impls `reeve_card::HouseholdLookup`.
- `reeved` — daemon binary. Composition root (`app/state.rs`, `app/lifecycle.rs`),
  transport sub-module (`transport/server.rs`, `transport/listener.rs`), dispatch table
  (`dispatch.rs`), startup reconcile pass (`reconcile.rs`).
- `reeve` — CLI + TUI client. Depends only on `reeve-protocol`.

### Aggregate roots

| Aggregate | Identity | Lifecycle |
|---|---|---|
| **Card** | `id: String` (UUID) | Created in Backlog; mutates through columns; persists indefinitely |
| **Workspace** | `card_id` (1:1 with Card) | Created on `→ Implementing`; torn down on `→ Done` |
| **Household** | `name: String` (PK) | Registered via `household add/new`; persists until `household rm` |

Cross-aggregate references are by ID only. The cross-cutting policy
`reeve_card::transitions::on_column_change(old, new, card, &dyn WorkspaceLifecycle)`
is the *only* code that knows how a Card column change maps to Workspace actions.

### Per-context interior (the four-file pattern)

Each context crate follows:

```
reeve-<context>/
├── Cargo.toml
└── src/
    ├── lib.rs        ← facade: pub re-exports + factory functions
    ├── domain.rs     ← pure entity + invariants + constructors (no rusqlite/tokio)
    ├── repo.rs       ← trait XxxRepo + pub(crate) SqliteXxxRepo + factory + init_schema
    └── handler.rs    ← thin RPC handlers taking port references as args
```

Workspace adds `git.rs`, `clones.rs`, `container.rs`, `fetch.rs`, `lifecycle.rs`.
Household adds `manifest.rs`, `scaffold.rs`.
Card adds `transitions.rs` (the kanban policy + cross-aggregate port traits).

### Architectural rules

1. **Cross-context coupling only through ports.** A port trait lives with its
   *consumer*. Its implementations live with the *supplier*. So:
   `reeve-workspace` depends on `reeve-card` (to impl `WorkspaceLifecycle`).
   `reeve-household` depends on `reeve-card` (to impl `HouseholdLookup`).
   `reeve-card` depends on no other context crate.
2. **Adapters are `pub(crate)`.** Every `Sqlite*Repo`, `Real*`, `Docker*` struct is
   crate-private. Cross-crate consumers obtain trait objects only via factory
   functions (`make_card_repo`, `make_workspace_repo`,
   `make_household_components`, `make_scaffold`, `make_git_ops`,
   `make_container_ops`).
3. **Domain has no infra deps** (convention within each crate). `domain.rs` and
   `transitions.rs` must not `use rusqlite`, `tokio::process`, or `docker`-related
   types.
4. **One bounded context per crate.** Don't put cross-cutting "shared" code into
   one of the context crates. Genuinely-shared infra lives in `reeve-protocol`
   (e.g., `paths::`, `lifecycle::`) or — only after the rule-of-three is hit —
   a new `reeve-shared` crate.
5. **Type strategy.** Card uses the γ pattern (domain entity in `reeve-card`
   with private fields + `From`/`TryFrom` to the wire DTO `reeve_protocol::Card`).
   Workspace and Household are domain-only — they're never transmitted whole;
   wire-side they appear as read-model projections (`HouseholdSummary`,
   `StatusSelfResult.container_status`).

### Enforcement

- **Cargo.toml dependencies** are the crate-level firewall. Adding a forbidden
  dep does not compile until the Cargo.toml is updated.
- **`pub(crate)` on adapters** compile-enforces "use the port, not the impl".
- **`scripts/check-context-isolation.sh`** is a CI grep lint catching
  cross-context internal-path imports (`use reeve_X::submodule::Type`). Run
  before pushing.

## Adding a new RPC method

1. Decide which aggregate owns the operation. Add the handler in that crate's
   `handler.rs`.
2. Re-export it from that crate's `lib.rs`.
3. Add a match arm in `crates/reeved/src/dispatch.rs` routing the method name
   to the handler.

## Schema independence

Each context owns its tables. No cross-context FKs. Schema-init is delegated
to per-crate `init_schema` functions, orchestrated by
`reeved/src/app/lifecycle.rs::migrate`:

```rust
pub fn migrate(conn: &Connection) -> Result<()> {
    reeve_card::init_schema(conn)?;       // cards table
    reeve_household::init_schema(conn)?;  // households table
    reeve_workspace::init_schema(conn)?;  // containers table (no FK to cards)
    Ok(())
}
```

This lets each context's `cargo test -p reeve-X` against a real SQLite stand
on its own — no cross-crate fixture wiring.

## Manifest schema migration

`Manifest.workspace` was renamed to `Manifest.meta_repo` in 2026-05. The
serde field has `#[serde(rename = "meta_repo", alias = "workspace")]` so
existing `household.json` files still parse; a `tracing::warn!` fires when
the old name is encountered. The witan-household template repo writes the
new name unconditionally. Alias support will be removed in a future release.

## Refactor history

- `docs/superpowers/specs/2026-04-29-reeve-design.md` — parent design
- `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md` —
  the spec that produced this structure
- `docs/superpowers/plans/2026-05-18-domain-driven-refactor.md` —
  the implementation plan executed on the `feat/domain-refactor` branch
