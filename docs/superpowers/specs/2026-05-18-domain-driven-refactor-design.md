# Domain-driven refactor — bounded contexts, vertical slice, per-context crates

**Status:** Design (2026-05-18, revised after code review)
**Predecessors:** M1–M5 implementation plans (2026-04-30 through 2026-05-17). Architecture critique session 2026-05-17.

## Context

After M1–M5 shipped, the daemon has accreted into a horizontally-layered shape that doesn't match the user's stated need: enabling parallel agent work on different bounded contexts. Concretely, `crates/reeved/src/handlers.rs` is 1963 lines, `clones.rs` is 1079 lines, and changes to either Card lifecycle or Household concerns inevitably co-edit the same files. Two agents working on independent contexts collide on the choke point.

This document specifies a structural refactor — preserving behaviour — that reorganises the daemon along bounded-context lines, with one crate per context and physical isolation between them enforced by Cargo and Rust visibility.

## Motivation

**Primary:** enable parallel agent work. Each bounded context should live in its own crate, so agents working on independent concerns touch zero shared files. Cross-context coupling happens only through declared trait ports.

**Secondary:** establish a documentable architecture. Once the structure matches DDD bounded contexts, the lore can describe "how Reeve is organised" with a vocabulary that maps 1:1 to the code.

This refactor is purely structural. No feature changes, no behaviour changes, no schema-breaking changes beyond the deliberate `Manifest.workspace` → `Manifest.meta_repo` rename (handled with `#[serde(alias)]` for backward compatibility) and the dropping of the now-unnecessary cross-context FK on `containers.card_id`.

## Summary

Restructure the daemon into six crates, organised by bounded context rather than by technical layer:

```
crates/
├── reeve-protocol/      wire DTOs, paths::, lifecycle helpers   (already exists)
├── reeve-card/          Card aggregate + transitions policy + WorkspaceLifecycle PORT
├── reeve-workspace/     Workspace aggregate + git/container adapters + fetch worker
├── reeve-household/     Household aggregate + Manifest + scaffold
├── reeved/              binary: composition root, dispatch table, transport, main.rs
└── reeve/               CLI + TUI                                (already exists, unchanged)
```

Transport (Unix-socket server + per-card listener registry) lives inside `reeved` as a sub-module rather than a crate of its own; it has exactly one consumer today and a second transport is not on the roadmap. Extract when a real second consumer appears.

Within each context crate, follow a consistent four-file pattern (`lib.rs` facade, `domain.rs` pure types + invariants, `repo.rs` port + adapter, `handler.rs` thin RPC entry points). Workspace adds adapter files for its larger surface (`git.rs`, `clones.rs`, `container.rs`, `fetch.rs`, `lifecycle.rs`).

Cross-aggregate coordination uses **inverted-dependency ports**: the *consuming* aggregate defines the trait; the *supplying* aggregate implements it. So `reeve-workspace` and `reeve-household` both depend on `reeve-card` (not the other way around). This is the Dependency Inversion Principle made physical.

Each context's persistence adapter (`SqliteCardRepo`, `SqliteWorkspaceRepo`, `SqliteHouseholdRepo`) is `pub(crate)` — invisible outside its defining crate. The only way to obtain an adapter is a factory function like `reeve_card::make_card_repo(conn) -> Arc<dyn CardRepo>`. The Rust compiler thus enforces "consumers use the port, not the impl" — there is no social-rule annotation. A CI grep lint adds belt-and-braces against cross-context internal-path imports.

Type strategy follows the .NET Clean Architecture pattern: domain entities live in each context's `domain.rs` with private fields, getters, factory constructors, and methods. Card additionally has a twin wire DTO (`protocol::Card`) because the JSON-RPC protocol transmits Card whole; `From`/`TryFrom` bridge them at handler and repository boundaries. Workspace and Household are never transmitted whole — wire-side they appear only as read-model projections constructed in handlers (`HouseholdSummary`, `HouseholdDetail`, `StatusSelfResult.container_status`). All three aggregates have the same internal shape; the asymmetry on the wire reflects what's actually transmitted.

The refactor is executed as a **big-bang in scope, incremental in execution**: eight stages, each keeping `cargo test --workspace` green.

## Aggregate roster and vocabulary

### Three aggregate roots

| Aggregate | Identity | Owns | Lifecycle |
|---|---|---|---|
| **Card** | `id: String` (UUID) | column, brief, household reference, parent_id, timestamps, needs_heavy flag | Created in Backlog; mutates through columns; persists indefinitely |
| **Workspace** | `card_id: String` (1:1 with Card) | the running Container (name, status, started_at), the cloned repos on disk, the per-card socket | Created on `→ Implementing`; torn down on `→ Done` |
| **Household** | `name: String` (PK) | meta-repo path, parsed Manifest (`meta_repo`, `knowledge_base`, `repos[]`) | Registered once via `household add` or `household new`; persists until `household rm` |

**Cross-aggregate references are by ID only.** No live pointers between aggregates. `Card.household_id` is a string name; `Workspace.card_id` is a string UUID. The DDD rule is preserved by construction. Database tables likewise reference by string only — no cross-context FKs (see §"Schema independence" below).

### Cross-cutting policy

`transitions::on_column_change(old, new, card, &dyn WorkspaceLifecycle)` lives in `reeve-card`. It is the only code that knows how a Card column change maps to Workspace actions. It is a pure function — same shape as today's `transitions::on_column_change`.

### Value objects (pure, immutable, no identity)

- `Column` — kanban state enum (Backlog/Planning/Implementing/Testing/Review/Done)
- `ContainerStatus` — Spawning/Running/Stopped/Failed
- `RepoOnDisk` — Workspace/Sibling/Inline/Missing
- `Manifest` and `RepoEntry` — parsed `household.json`

### Ubiquitous language

| Term | Meaning |
|---|---|
| **Household** | Registered project bundle (meta-repo + declared repos + KB) |
| **Manifest** | Parsed `household.json` |
| **meta-repo** | The repo that holds the manifest |
| **sibling** | Non-meta repo with its own `.git/` |
| **inline** | Non-meta repo embedded in the meta-repo (no own `.git/`) |
| **knowledge_base** | Manifest field naming the KB repo entry |
| **Card** | Kanban metadata + column state |
| **Column** | Kanban column (Backlog … Done) |
| **Workspace** | Per-card on-disk + container materialization |
| **Container** | Docker container of a running workspace |
| **Listener** | Per-card RPC socket |
| **Fetch worker** | Background loop mirroring workspace refs into the host meta-repo (M4) |
| **Tmux session** | Per-card session inside the container running `claude` (M5) |

Two existing names are explicitly retired in favour of clearer ones:

- `Manifest.workspace` → `Manifest.meta_repo`. The field names the repos[] entry that IS the meta-repo. `meta_repo` aligns with existing `meta_repo_path` usage and reads accurately. Migration via `#[serde(rename = "meta_repo", alias = "workspace")]` for one release.
- The previously-implicit per-card materialization concept is now named **Workspace**. (No earlier code name existed; we go straight from path-arithmetic-only to a typed aggregate.)

## Crate layout and dependency graph

### Six crates

```
crates/
├── reeve-protocol/      ← wire DTOs, paths::, lifecycle helpers   (already exists)
├── reeve-card/          ← Card aggregate + transitions + WorkspaceLifecycle port + HouseholdLookup port
├── reeve-workspace/     ← Workspace aggregate + impls of WorkspaceLifecycle + git/container adapters + fetch worker
├── reeve-household/     ← Household aggregate + impl of HouseholdLookup + Manifest + scaffold
├── reeved/              ← binary: composition root, dispatch, app/lifecycle, transport sub-module
└── reeve/               ← CLI + TUI                                (already exists, unchanged)
```

### Dependency rules (Cargo.toml is the audit)

| Crate | Depends on | Forbidden imports |
|---|---|---|
| `reeve-protocol` | (nothing daemon-side) | — |
| `reeve-card` | `reeve-protocol` | workspace, household |
| `reeve-workspace` | `reeve-protocol`, `reeve-card` (port only) | household |
| `reeve-household` | `reeve-protocol`, `reeve-card` (port only) | workspace |
| `reeved` | all 4 above | — (composition root) |
| `reeve` | `reeve-protocol` | daemon-side crates |

**Two agents on independent contexts cannot collide on the same file.** The Cargo.toml is the architecture diagram. Adding a forbidden dependency does not compile until the Cargo.toml is updated, which is itself a deliberate, reviewable act.

### Enforcement of "use the port, not the impl"

Cargo.toml prevents *crate-level* coupling, but within a declared dependency, nothing stops `reeve-workspace` from `use reeve_card::SqliteCardRepo` (if exposed). To prevent this, every persistence adapter is **`pub(crate)`** and accessed only via a factory function:

```rust
// reeve-card/src/repo.rs
pub trait CardRepo: Send + Sync { ... }
pub(crate) struct SqliteCardRepo { conn: Arc<Mutex<Connection>> }

pub fn make_card_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardRepo> {
    Arc::new(SqliteCardRepo::new(conn))
}
```

`SqliteCardRepo` is unnameable outside `reeve-card`. The only obtainable handle is `Arc<dyn CardRepo>`. Compiler-enforced.

The same pattern applies to every adapter: `make_workspace_repo`, `make_household_repo`, `make_git_ops`, `make_container_ops`, `make_household_scaffold`. Trait objects are the public currency.

For `HouseholdLookup` (one of the inverted-dep ports) and `HouseholdRepo` (the persistence port), both pointing at the same backing data: a single factory returns both Arcs constructed from one concrete instance:

```rust
// reeve-household/src/lib.rs
pub fn make_household_components(conn: Arc<Mutex<Connection>>)
    -> (Arc<dyn HouseholdRepo>, Arc<dyn reeve_card::HouseholdLookup>)
{
    let concrete = Arc::new(SqliteHouseholdRepo::new(conn));
    (concrete.clone(), concrete)
}
```

### CI lint (belt and braces)

In addition to compile-time `pub(crate)` enforcement, add a small CI check that fails the build if any source file imports a path that reaches past another crate's `mod.rs` re-exports. Concretely: a shell script (or `xtask`) that runs:

```
git grep -nE 'use reeve_(card|workspace|household)::[a-z_]+::' crates/
```

If any match appears outside the defining crate, fail CI. This catches accidental `use reeve_card::repo::CardRepo` (which would work — the trait is `pub` — but reaches past the facade) and any future leakage. ~10 lines.

### Inverted-dependency ports

`reeve-card` is the central consumer; the other context crates supply what it needs:

```
reeve-card  ←─── reeve-workspace    (impls WorkspaceLifecycle)
     ▲
     └─────── reeve-household   (impls HouseholdLookup)
```

The card crate defines the trait describing what it needs. The supplying crate implements it. Reeved composes them at startup.

### File-to-crate mapping

| Today (`reeved/src/...`) | Lines | Tomorrow |
|---|---|---|
| `main.rs` | 199 | `reeved/src/main.rs` (thinner — pure composition) |
| `lifecycle.rs` | 36 | `reeved/src/app/lifecycle.rs` (paths, db_path, migrate orchestrator) |
| `server.rs` | 343 | `reeved/src/transport/server.rs` |
| `listeners.rs` | 230 | `reeved/src/transport/listener.rs` |
| `handlers.rs` | 1963 | **split** → `reeve-card/src/handler.rs`, `reeve-workspace/src/handler.rs`, `reeve-household/src/handler.rs`; dispatch glue in `reeved/src/dispatch.rs` |
| `transitions.rs` | 160 | `reeve-card/src/transitions.rs` |
| `effects.rs` | 247 | `reeve-workspace/src/lifecycle.rs` (the `WorkspaceLifecycle` impl) |
| `household.rs` | 116 | `reeve-household/src/domain.rs` |
| `manifest.rs` | 253 | `reeve-household/src/manifest.rs` |
| `db.rs` | 501 | **split** → per-context `repo.rs` files; schema-init coordinator in `reeved/src/app/lifecycle.rs::migrate` |
| `clones.rs` | 1079 | **split** → `reeve-workspace/src/git.rs` (`GitOps` + `RealGitOps`), `reeve-workspace/src/clones.rs` (`clone_all_for_card` workflow) |
| `container.rs` | 638 | `reeve-workspace/src/container.rs` |
| `fetch.rs` | 438 | `reeve-workspace/src/fetch.rs` |
| `reeve-protocol/src/lib.rs` | 675 | mostly **unchanged**; rename `Manifest.workspace` field with `#[serde(alias)]` for back-compat |
| `reeve/src/*` | 866 | **unchanged** (CLI+TUI; depends only on reeve-protocol) |

Three files (`handlers.rs`, `db.rs`, `clones.rs`) account for 3543 of 7069 daemon LoC and all three get split. The rest is mechanical relocation.

### Estimated post-split crate sizes

| Crate | Approx LoC | Files |
|---|---|---|
| `reeve-protocol` | ~700 | `lib.rs` (unchanged structure) |
| `reeve-card` | ~600 | `domain.rs`, `repo.rs`, `handler.rs`, `transitions.rs` |
| `reeve-workspace` | ~2800 | `domain.rs`, `repo.rs`, `handler.rs`, `lifecycle.rs`, `git.rs`, `clones.rs`, `container.rs`, `fetch.rs` |
| `reeve-household` | ~700 | `domain.rs`, `repo.rs`, `handler.rs`, `manifest.rs`, `scaffold.rs` |
| `reeved` | ~900 | `main.rs`, `dispatch.rs`, `app/state.rs`, `app/lifecycle.rs`, `transport/server.rs`, `transport/listener.rs` |
| `reeve` | 866 | unchanged |

### Workspace size: co-edit map and revisit threshold

`reeve-workspace` is the largest crate at ~2800 LoC across 8 files. It is one bounded context (all of these serve the Workspace aggregate), so sub-crating would fragment the aggregate's invariants across crates and make `LiveWorkspaceLifecycle` a new cross-crate orchestrator — a fresh choke point. Worse, not better.

Within the crate, files split along *kinds of work*, so parallel work on workspace usually does not collide:

| Kind of change | Files likely touched |
|---|---|
| New clone topology (e.g., new repo kind, KB-aware clones) | `clones.rs`, possibly `lifecycle.rs` |
| Fetch worker behaviour (interval, refspec, retry) | `fetch.rs` |
| Container/docker behaviour, attach, exec | `container.rs`, possibly `lifecycle.rs` |
| Workspace-side RPCs (push, attach, status) | `handler.rs` |
| Workspace state model / persistence | `domain.rs`, `repo.rs` |
| Provision/teardown orchestration | `lifecycle.rs` |

The shared overlap point is `lifecycle.rs` (the `WorkspaceLifecycle` impl), which orchestrates clone + container + listener. If we find ourselves repeatedly co-editing `lifecycle.rs` for *unrelated* work, that is the signal to split — the orchestrator has grown too omniscient.

**Revisit threshold:** if any single file in `reeve-workspace` exceeds **800 LoC** or the whole crate exceeds **3500 LoC**, revisit sub-crating. Likely first split would be `reeve-workspace-runtime` (container + attach + reconcile) extracted as a separate crate with its own `RuntimeOps` trait consumed by a slimmer workspace core. Until then, accept the size — the cohesion benefit of one Workspace aggregate dominates.

## Per-context interior (worked example: `reeve-card`)

Each context crate follows a consistent four-file structure. Anatomy:

```
reeve-card/
├── Cargo.toml
└── src/
    ├── lib.rs              ← facade: pub re-exports + factory functions
    ├── domain.rs           ← pure entity + invariants + constructors (no rusqlite/tokio)
    ├── repo.rs             ← trait CardRepo + pub(crate) SqliteCardRepo + init_schema
    ├── transitions.rs      ← cross-aggregate policy + the two ports it needs
    └── handler.rs          ← RPC handlers: thin, take ports as args
```

### `lib.rs` — the facade (sole public surface)

```rust
//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod repo;
mod handler;
mod transitions;

// Domain
pub use domain::{Card, CardError};

// Repository port + factory (adapter struct is pub(crate))
pub use repo::{CardRepo, make_card_repo, init_schema};

// Cross-aggregate policy + the two ports it requires
pub use transitions::{on_column_change, WorkspaceLifecycle, HouseholdLookup};

// RPC handlers (called by reeved/dispatch.rs)
pub use handler::{handle_ls, handle_new, handle_mv, handle_mv_self, handle_status_self};
```

### `domain.rs` — entity with private fields, invariants enforced at construction

```rust
//! Card entity + invariants. No rusqlite, no tokio, no docker.

use reeve_protocol::Column;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CardError {
    #[error("brief must be non-empty")]
    EmptyBrief,
    #[error("brief too long ({0} > 200 chars)")]
    BriefTooLong(usize),
}

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
    pub fn new(household: String, brief: String, needs_heavy: bool) -> Result<Self, CardError> {
        if brief.is_empty() { return Err(CardError::EmptyBrief); }
        let n = brief.chars().count();
        if n > 200 { return Err(CardError::BriefTooLong(n)); }
        let now = now_secs();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            household, brief,
            column: Column::Backlog,
            created_at: now, updated_at: now,
            needs_heavy,
            parent_id: None,
        })
    }

    pub fn id(&self) -> &str { &self.id }
    pub fn brief(&self) -> &str { &self.brief }
    pub fn column(&self) -> Column { self.column }
    pub fn household(&self) -> &str { &self.household }

    pub fn with_column(self, column: Column) -> Self {
        Self { column, updated_at: now_secs(), ..self }
    }
}

impl From<Card> for reeve_protocol::Card { /* field-by-field */ }

impl TryFrom<reeve_protocol::Card> for Card {
    type Error = CardError;
    fn try_from(d: reeve_protocol::Card) -> Result<Self, CardError> {
        // re-runs invariants on wire-inbound data
        // ...
    }
}
```

`From` for the domain → wire direction (always succeeds). `TryFrom` for the wire → domain direction (re-runs invariants). This makes the wire→entity boundary the validation point — if you hold a `domain::Card`, it is validated by construction.

### `repo.rs` — port + (crate-private) adapter + factory

```rust
//! Persistence port + SQLite adapter for the Card aggregate.

use crate::domain::Card;
use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{CardReference, Column};
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait CardRepo: Send + Sync {
    async fn list(&self) -> Result<Vec<Card>>;
    async fn get(&self, id: &str) -> Result<Option<Card>>;
    async fn insert(&self, card: &Card) -> Result<()>;
    async fn update_column(&self, id: &str, col: Column, now: i64) -> Result<Option<Card>>;
    async fn for_household(&self, name: &str) -> Result<Vec<CardReference>>;
}

pub(crate) struct SqliteCardRepo { conn: Arc<Mutex<Connection>> }

impl SqliteCardRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self { Self { conn } }
}

#[async_trait]
impl CardRepo for SqliteCardRepo {
    // each method hides the spawn_blocking + blocking_lock dance inside the impl
}

/// Construct the SQLite-backed CardRepo. The only way for reeved to obtain one.
pub fn make_card_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardRepo> {
    Arc::new(SqliteCardRepo::new(conn))
}

/// Initialise the cards table. Called by reeved/app/lifecycle.rs::migrate.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cards (
            id TEXT PRIMARY KEY, household TEXT NOT NULL, brief TEXT NOT NULL,
            column_ TEXT NOT NULL, created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL, needs_heavy INTEGER NOT NULL DEFAULT 0,
            parent_id TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);"
    )?;
    Ok(())
}
```

### `transitions.rs` — policy and the two cross-aggregate ports

```rust
//! Kanban policy + ports the Card aggregate needs from other aggregates.

use crate::domain::Card;
use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::Column;

#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
}

pub async fn on_column_change(
    old: Column, new: Column, card: &Card,
    workspace: &dyn WorkspaceLifecycle,
) -> Result<()> {
    // exactly today's transitions::on_column_change matrix
}
```

### `handler.rs` — thin, signature names exactly what it depends on

```rust
//! RPC handlers. Each handler signature names exactly what it depends on.

pub async fn handle_mv(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    workspace: &dyn WorkspaceLifecycle,
    params: MvParams,
) -> Result<Value, RpcError> {
    let existing = cards.get(&params.id).await.map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;

    // Precheck before mutation: container-bound moves require a configured household.
    if params.column == Column::Implementing
        && !households.exists(existing.household()).await.map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: format!("household '{}' not configured", existing.household()),
        });
    }

    let now = now_secs();
    let card = cards.update_column(&params.id, params.column, now).await
        .map_err(internal)?.ok_or_else(|| not_found(&params.id))?;

    on_column_change(existing.column(), params.column, &card, workspace).await
        .map_err(|e| RpcError { code: RpcError::INTERNAL, message: format!("{e:#}") })?;

    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(MvResult { card: dto }).map_err(internal)
}
```

Compare to today's `handle_mv(state: DaemonState, params: MvParams)` where `state` is a kitchen sink whose contents the function may or may not touch.

The same four-file shape recurs in `reeve-household`. `reeve-workspace` is larger (it gets `git.rs`, `clones.rs`, `container.rs`, `fetch.rs`, `lifecycle.rs` in addition to the core four), but the core structure is the same.

## Type strategy applied to all three aggregates

All three aggregates have **the same internal shape**: a typed entity with private fields, getters, factory constructors, and methods. Card *additionally* has a twin wire DTO because the JSON-RPC protocol transmits `Card` whole in `LsResult` / `NewResult` / `MvResult`. Workspace and Household are never transmitted whole — wire-side they appear only as projections constructed by handlers. Same internal pattern, different transmission shape.

### `reeve-workspace/src/domain.rs` (sketch)

```rust
//! Workspace entity: per-card materialization.

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
    /// Unprovisioned workspace — created when the card first transitions to Implementing.
    pub fn for_card(card_id: String, socket_path: PathBuf) -> Self {
        Self { card_id, container: None, socket_path }
    }

    pub fn card_id(&self) -> &str { &self.card_id }
    pub fn container(&self) -> Option<&Container> { self.container.as_ref() }
    pub fn is_running(&self) -> bool {
        matches!(self.container.as_ref().map(|c| c.status), Some(ContainerStatus::Running))
    }

    pub fn with_container(self, container: Container) -> Self {
        Self { container: Some(container), ..self }
    }
    pub fn without_container(self) -> Self {
        Self { container: None, ..self }
    }
}

impl Container {
    pub fn new(name: String, status: ContainerStatus, started_at: i64) -> Self {
        Self { name, status, started_at }
    }
    pub fn name(&self) -> &str { &self.name }
    pub fn status(&self) -> ContainerStatus { self.status }
    pub fn with_status(self, status: ContainerStatus) -> Self { Self { status, ..self } }
}
```

No twin DTO. The protocol never transmits `Workspace`; `StatusSelfResult.container_status: Option<ContainerStatus>` is a projection constructed in the handler.

### `reeve-household/src/domain.rs` (sketch)

```rust
//! Household entity: registered project bundle.

use crate::manifest::{Manifest, RepoEntry};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HouseholdError {
    #[error("name must be non-empty")]
    EmptyName,
    #[error("meta-repo path does not exist: {0}")]
    MetaRepoMissing(PathBuf),
    #[error("meta-repo is not a git repository: {0}")]
    MetaRepoNotGit(PathBuf),
}

pub struct Household {
    name: String,
    meta_repo: PathBuf,
    manifest: Manifest,
}

impl Household {
    pub fn new(name: String, meta_repo: PathBuf, manifest: Manifest)
        -> Result<Self, HouseholdError>
    {
        if name.is_empty() { return Err(HouseholdError::EmptyName); }
        if !meta_repo.exists() { return Err(HouseholdError::MetaRepoMissing(meta_repo)); }
        if !meta_repo.join(".git").exists() { return Err(HouseholdError::MetaRepoNotGit(meta_repo)); }
        // Manifest is already validated by manifest::load (its own constructor)
        Ok(Self { name, meta_repo, manifest })
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn meta_repo(&self) -> &Path { &self.meta_repo }
    pub fn manifest(&self) -> &Manifest { &self.manifest }

    pub fn devcontainer_path(&self) -> PathBuf {
        self.meta_repo.join(".devcontainer/devcontainer.json")
    }

    pub fn knowledge_base_repo(&self) -> Option<&RepoEntry> {
        self.manifest.knowledge_base.as_deref()
            .and_then(|name| self.manifest.repos.iter().find(|r| r.name == name))
    }
}
```

No twin DTO. Wire-side, `HouseholdSummary` and `HouseholdDetail` are read-model projections constructed in the handlers (e.g., `HouseholdSummary::from(&household)` or assembled in `handle_household_show`).

### Summary

| Aggregate | Domain entity | Wire DTO | Conversion |
|---|---|---|---|
| `Card` | `reeve_card::Card` | `reeve_protocol::Card` | `TryFrom` (wire→domain, with invariant re-check), `From` (domain→wire) |
| `Workspace` | `reeve_workspace::Workspace` | none | `StatusSelfResult` etc. constructed in handlers as projections |
| `Household` | `reeve_household::Household` | none | `HouseholdSummary` / `HouseholdDetail` constructed in handlers as projections |

The pattern is **"typed entity with methods, in the context crate"** for all three. The wire twin only exists when the protocol transmits the entity whole — and Card is the only one that does today.

## Cross-aggregate wiring and the composition root

### Dispatch routing — in `reeved/dispatch.rs`

The transport sub-module (`reeved/src/transport/`) stays generic: it knows about Unix sockets and JSON-line framing, nothing about RPC method names. The dispatch table lives in `reeved/src/dispatch.rs`, a single `match` block routing every method to the owning context's handler. Adding an RPC method requires touching both `reeved/dispatch.rs` and the owning crate — a deliberate ceremony that keeps routing auditable in one place.

### `DaemonState` — composition of port handles

```rust
// reeved/src/app/state.rs
pub struct DaemonState {
    // Repositories
    pub cards: Arc<dyn CardRepo>,
    pub workspaces: Arc<dyn WorkspaceRepo>,
    pub households: Arc<dyn HouseholdRepo>,

    // Cross-aggregate ports (consumed by reeve-card)
    pub households_lookup: Arc<dyn HouseholdLookup>,        // typically same backing struct as `households`
    pub workspace_lifecycle: Arc<dyn WorkspaceLifecycle>,    // points at LiveWorkspaceLifecycle

    // Transport
    pub listeners: ListenerRegistry,
}
```

Everything that is only used inside `LiveWorkspaceLifecycle` (GitOps, ContainerOps, claude_dir, api_key, scaffolding details) is held privately inside it. `DaemonState` exposes only what `dispatch.rs` actually calls.

### Per-consumer infrastructure ports

`reeve-workspace` defines `GitOps` for the operations it needs (`clone`, `fetch`, `push`). `reeve-household` defines `HouseholdScaffold` for the operations it needs (`clone_template`, `init_repo`, `commit_all`). Both impls shell out to `git`, but they are separate types in separate crates — no cross-context coupling.

By rule of three, if a third consumer of git operations appears (or duplication becomes painful in some other way), revisit and factor a shared `reeve-shared` infrastructure crate or expose `GitOps` from a more central home. Until then, accept the modest duplication.

### Fetch worker — background, not request-driven

`reeve-workspace/src/fetch.rs` exposes `pub async fn run(workspaces, cards, git, shutdown) -> Result<()>`. `reeved` spawns it via `tokio::spawn(reeve_workspace::fetch::run(...))` at startup; sends shutdown on signal. No state model changes; just another consumer of the existing port handles.

### Schema independence — no cross-context FKs

Each context's `init_schema` is independent. The previously-existing FK `containers.card_id REFERENCES cards.id ON DELETE CASCADE` is **dropped**. Justification:

- The FK leaked context isolation: `cargo test -p reeve-workspace` against a real (non-mocked) SQLite required inserting a Card row first to satisfy the constraint, coupling test setup across crates.
- The only behaviour the FK provided was `ON DELETE CASCADE`. Reeve has no card-delete RPC today; the cascade was theoretical.
- The "does the card exist?" check naturally belongs to `LiveWorkspaceLifecycle::ensure_running`, which already reads the card from the DB before provisioning. If the card disappears between read and write, that's a race with whatever deleted it — and when card-delete is added (post-v1), app-layer cleanup of the workspace row is two lines.

Schema-init ordering is therefore arbitrary; alphabetical for stability:

```rust
// reeved/src/app/lifecycle.rs
pub fn migrate(conn: &Connection) -> Result<()> {
    reeve_card::init_schema(conn)?;
    reeve_household::init_schema(conn)?;
    reeve_workspace::init_schema(conn)?;
    Ok(())
}
```

### Manifest field migration: `workspace` → `meta_repo`

```rust
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    /// Name of the repos[] entry that IS the meta-repo.
    #[serde(rename = "meta_repo", alias = "workspace")]
    pub meta_repo: String,

    #[serde(default)]
    pub knowledge_base: Option<String>,

    pub repos: Vec<RepoEntry>,
}
```

Existing `household.json` files using `workspace` continue to parse, with a `tracing::warn!` logged about the deprecated field. The `household new` scaffolder writes the new name unconditionally. After one release, drop the alias. The `witan-household` template repository is updated separately.

### End-to-end composition (`reeved/src/main.rs`, abbreviated)

```rust
fn main() -> Result<()> {
    init_tracing();
    set_umask();
    let conn = open_db(db_path()?)?;
    app::lifecycle::migrate(&conn)?;
    let conn = Arc::new(Mutex::new(conn));

    // Repos via factory functions — adapters are crate-private; only the trait objects escape.
    let cards: Arc<dyn CardRepo>            = reeve_card::make_card_repo(conn.clone());
    let workspaces: Arc<dyn WorkspaceRepo>  = reeve_workspace::make_workspace_repo(conn.clone());
    let (households, households_lookup)     = reeve_household::make_household_components(conn.clone());

    // Per-consumer infrastructure ports
    let git: Arc<dyn GitOps>                = reeve_workspace::make_git_ops();
    let containers: Arc<dyn ContainerOps>   = reeve_workspace::make_container_ops();
    let scaffold: Arc<dyn HouseholdScaffold> = reeve_household::make_scaffold();

    let claude_dir = resolve_claude_dir();
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let listeners = ListenerRegistry::new(/* deps */);

    let workspace_lifecycle: Arc<dyn WorkspaceLifecycle> = Arc::new(
        LiveWorkspaceLifecycle::new(
            workspaces.clone(), cards.clone(), households.clone(),
            git.clone(), containers.clone(), listeners.clone(),
            claude_dir, api_key,
        )
    );

    let state = DaemonState {
        cards, workspaces, households,
        households_lookup, workspace_lifecycle,
        listeners,
    };

    workspace::reconcile_on_startup(&state).await?;
    let fetch_handle = spawn_fetch_worker(&state, git.clone());
    let server_shutdown = spawn_signal_handler();

    transport::server::run(
        &socket_path(),
        CallerContext::host(),
        server_shutdown,
        move |req, caller| dispatch::dispatch(req, &state, caller),
    ).await?;

    fetch_handle.shutdown().await;
    Ok(())
}
```

Every Arc is wired explicitly. Every trait object is constructed once through a factory. No `SqliteCardRepo` (or any other adapter) is nameable here — only the traits are visible. Reading `main.rs` tells you the whole dependency graph.

## Migration plan

Eight stages, each keeping `cargo test --workspace` green. PR-sized.

| # | Stage | Touches | Risk |
|---|---|---|---|
| 0 | **Crate skeletons + reeved sub-folders** | Add empty `reeve-card/`, `reeve-workspace/`, `reeve-household/` with `Cargo.toml` + `src/lib.rs`. Wire into root `Cargo.toml`. Create empty `reeved/src/app/` and `reeved/src/transport/` sub-folders with stub `mod.rs`. Workspace compiles. | None |
| 1 | **Move transport into reeved sub-module** | Move `reeved/src/server.rs` → `reeved/src/transport/server.rs` and `reeved/src/listeners.rs` → `reeved/src/transport/listener.rs`. Update imports inside reeved. | Low |
| 2 | **Carve `reeve-household`** | Move `household.rs`, `manifest.rs`, household handlers, household DB code, schema-init. Define `HouseholdRepo` trait + `pub(crate) SqliteHouseholdRepo`. Define `HouseholdLookup` trait. Add `make_household_components` factory. Port `handle_household_new`'s inlined git workflow into `HouseholdScaffold` trait + `pub(crate) RealHouseholdScaffold` + `make_scaffold` factory. Domain entity (`Household`) with typed constructor + invariants. | Medium |
| 3 | **Carve `reeve-card`** | Move `transitions.rs`. Define `CardRepo` trait + `pub(crate) SqliteCardRepo` + `make_card_repo` factory. Define `WorkspaceLifecycle` trait. Move card handlers (ls/new/mv/mv-self/status-self). | Medium |
| 4 | **Carve `reeve-workspace` + drop FK** | Move `container.rs`, `clones.rs`, `effects.rs` (becomes `LiveWorkspaceLifecycle`), `fetch.rs`, workspace handlers (attach, push). Define `WorkspaceRepo` + `pub(crate) SqliteWorkspaceRepo` + `make_workspace_repo` factory. Define `pub(crate) RealGitOps` + `make_git_ops`, same for ContainerOps. Implement `WorkspaceLifecycle` from card crate. Domain entity (`Workspace` + `Container`) with typed constructor. **Drop `containers.card_id REFERENCES cards.id` from schema**; move card-existence check into `LiveWorkspaceLifecycle::ensure_running` (already reads the card before provisioning). | Highest |
| 5 | **Card γ type migration** | Add `domain::Card` to `reeve-card`. Add `TryFrom<protocol::Card>` and `From<domain::Card> for protocol::Card`. Round-trip test. Update `CardRepo` to take and return `domain::Card`. Update handlers to convert at wire boundary. Update `SqliteCardRepo` to convert at row boundary. | Medium |
| 6 | **Rename `Manifest.workspace` → `meta_repo`** | Add `#[serde(rename, alias)]`, update internal uses, add deprecation warning. Update witan-household template separately. | Low |
| 7 | **Wire migration & cleanup** | Update `reeved/src/main.rs` composition to final shape (factory functions). Delete the now-empty `reeved/src/*.rs` modules at top level (everything is now in sub-folders or other crates). | Low |
| 8 | **Quality pass + CI lint** | `cargo fmt`, `cargo clippy`, full `cargo test --workspace`, run e2e tests with docker. **Add CI lint script** (`scripts/check-context-isolation.sh` or `xtask check-arch`) that fails if any source file uses a cross-context internal path (`use reeve_<other>::<submodule>::*` outside the defining crate). Wire into CI. Update CLAUDE.md with the new architectural rules. | Low |

**Stage ordering rationale.** Stages 1–2 are independent. Stage 3 (card) must precede stage 4 (workspace) because workspace depends on card's `WorkspaceLifecycle` trait. Stage 5 (Card γ) is deliberately *after* the carve so we are not doing two refactors at once. Stages 6–8 are cleanup and validation.

## Testing strategy

### What changes

| Test concern | Today | After refactor |
|---|---|---|
| Domain logic unit tests | `#[cfg(test)] mod tests` inline | Same pattern, per-crate. Each context's `domain.rs` has its tests. |
| Handler tests with port mocks | `fresh_state()` builder with NoopOps/NoopGit/NoopDispatcher | Per-crate: card tests mock `CardRepo`, `WorkspaceLifecycle`, `HouseholdLookup`. Each test sets up only the ports it needs. |
| Adapter integration tests | `docker_tests` in container.rs (docker-availability-gated) | Stay in `reeve-workspace`, same `#[cfg(test)]` gate |
| End-to-end | `tests/e2e.rs` | Unchanged — the existing `tests/` crate exercises the daemon binary |
| Per-crate adapter tests against real SQLite | Coupled — needed cards row before workspace row (FK) | Independent — drop-FK means `cargo test -p reeve-workspace` can insert a containers row without any cards row |

### Mocks: keep them per-crate

Each context's `#[cfg(test)] mod tests` defines its own mock structs. If duplication ever bites (same mock needed in three places), promote to a `*-test-support` module or feature-gated submodule. Don't pre-build that scaffolding.

### Drift protection

For the Card γ migration (stage 5), add a round-trip test:

```rust
#[test]
fn protocol_to_domain_to_protocol_round_trips() {
    let dto = /* sample protocol::Card */;
    let entity = Card::try_from(dto.clone()).unwrap();
    let dto2: reeve_protocol::Card = entity.into();
    assert_eq!(dto, dto2);
}
```

If a field is added to either `protocol::Card` or `domain::Card`, this test (plus `cargo check`) catches missing field handling on either side.

### Gains

- **Speed**: each crate's unit tests compile and run independently. `cargo test -p reeve-card` is fast.
- **Clarity**: handler tests mock only what they use. No more `fresh_state()` constructing unrelated dependencies.
- **Domain isolation**: `reeve-card/src/domain.rs` and `transitions.rs` test with zero infra deps.
- **Real-SQLite isolation**: drop-FK means per-crate tests against a real connection no longer need cross-context fixtures.
- **Adapter exercise**: docker/git tests stay opt-in (gated on tool availability), where they already are.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| `handle_household_new`'s inline 120-line git workflow lost in transit | Stage 2 explicitly ports the workflow into `RealHouseholdScaffold`; test coverage carries over |
| `precheck_household_for_implementing` semantics drift | Express it as a one-method `HouseholdLookup::exists` trait; pin behaviour with existing `mv_to_implementing_without_household_leaves_column_unchanged` test |
| Card-existence check on workspace provision lost when FK dropped | Stage 4 explicitly moves the check into `LiveWorkspaceLifecycle::ensure_running` (existing `db::get_card` call there); e2e tests pin behaviour |
| `From`/`TryFrom` between `domain::Card` and `protocol::Card` drifts | Round-trip test in `reeve-card/src/domain.rs`; `cargo check` catches missing fields on either side |
| TUI work in progress collides with refactor | TUI lives in the `reeve` crate, which the refactor doesn't touch. Resume after refactor without conflict. |
| M5 tmux/attach integration accidentally lost | Stage 4 explicitly moves `ContainerOps::exec_in_tmux` + `handle_attach`; e2e test `m5: e2e attach` is the regression net |
| Adapter visibility regresses (someone makes `SqliteCardRepo` `pub` for convenience) | CI lint catches cross-context imports past `mod.rs`; code review catches `pub` widening |
| Workspace crate grows past the revisit threshold | Stage 8 adds a `cargo loc` check (or equivalent) to CI; sub-crate per the documented split when threshold is hit |
| Public API for the dispatch table accidentally widens | `reeved/src/dispatch.rs` is the only place that routes RPCs; review the diff at stage 7 with the question "is anything else now reaching into per-crate handlers directly?" |

## Out of scope for this refactor

Deliberately deferred. Worth revisiting in a follow-up round.

- **Per-context-per-layer crates (A++)**: keep domain-vs-infra as convention within each context for v1. Revisit if any single crate grows past ~3000 LoC with mixed concerns.
- **Sub-crating `reeve-workspace`**: stay one crate per the threshold documented above; revisit if `lifecycle.rs` becomes a contended choke point or the crate exceeds 3500 LoC.
- **Repository trait abstractions beyond SQLite**: `CardRepo` etc. are async traits, but we ship only the SQLite adapter. No `InMemoryCardRepo` etc. until tests genuinely need them.
- **`CardId` / `HouseholdName` newtypes**: today everything is `String`. Adding `pub struct CardId(String)` etc. is a separate cleanup — worthwhile but orthogonal.
- **Card field renames / data-type review**: user has flagged interest in reviewing field names and types on `Card` (and likely others) — a separate, post-refactor pass.
- **Schema versioning / migration framework**: `init_schema` stays idempotent `CREATE IF NOT EXISTS`. A real migration framework (versioned migrations, `_schema_version` table) waits until we need to alter an existing column.
- **A `reeve-shared` infrastructure crate**: duplicated git port-shapes are accepted per rule-of-three. Same for `now_secs()` if it ends up in two places.
- **The `b key` branching feature**: alluded to in the design spec; not part of this refactor. The architecture supports it cleanly when we get there.
- **Lorekeeper integration code**: spec defers Reeve↔Lorekeeper integration to post-v1 (per commit `a921b63`). No KB-aware code added in this refactor.
- **Witan-household template repo updates**: separate work in that repo; we update the docs reference but the template itself ships its own change.

## Open implementation questions

None at design-completion time. All major decisions have been made. The implementation plan (next document) will address ordering within each stage and any task-level fine-grain choices.

## Decision log (for the lore later)

| Decision | Choice | Why |
|---|---|---|
| Primary architectural goal | Enable parallel agent work | User's stated motivation; tests on file-level seam locations |
| Workflow owner for card workspace materialization | New `Workspace` aggregate | Distinct lifecycle, invariants, and transactional boundary from `Card` |
| Aggregate root count | Three: Card, Workspace, Household | Each has independent lifecycle and transactional boundary |
| Cross-aggregate coordination | Pure policy + inverted-dependency ports | Standard DDD; today's `transitions::on_column_change` already follows this shape |
| Type pattern for Card | γ — domain entity + twin wire DTO + From/TryFrom | Matches user's mental model (.NET Clean Architecture); compile-time wire/domain boundary; room to grow rich behaviour |
| Type pattern for Workspace and Household | Domain entity only (no twin DTO) | Protocol never transmits these whole; wire side carries read-model projections constructed in handlers |
| Folder organisation | Vertical slice (folder per context) | Maximises cohesion-per-context; directly serves parallel-agent-work goal |
| Crate granularity | One crate per bounded context (six crates total) | Compiler enforces context isolation; Cargo.toml is the architectural firewall |
| Transport: crate or sub-module | Sub-module of `reeved`, not its own crate | Rule of three — one consumer today, no second transport planned; extract when a real second consumer appears |
| Adapter visibility | `pub(crate)` + factory functions returning `Arc<dyn Trait>` | Compiler-enforced "use the port, not the impl"; no social-rule annotations |
| CI enforcement | Grep lint for cross-context internal-path imports | Belt and braces alongside `pub(crate)`; catches accidental reaches-past-facade |
| Manifest field rename | `workspace` → `meta_repo` | Frees "workspace" for unambiguous use as the per-card aggregate name |
| Aggregate name for per-card materialization | `Workspace` (not `CardWorkspace`) | After freeing the namespace, the short form is unambiguous and idiomatic |
| Schema cross-context FK | Drop (`containers.card_id REFERENCES cards.id`) | FK leaked context isolation in per-crate tests; CASCADE was theoretical (no card-delete RPC exists) |
| Schema-init ordering | Independent per-context; arbitrary order at composition | Consequence of dropping the FK — no constraint to honour |
| Migration sequencing | Big-bang in scope, incremental in execution (8 stages) | User chose to do the refactor as one focused effort, but stages keep tests green throughout |
