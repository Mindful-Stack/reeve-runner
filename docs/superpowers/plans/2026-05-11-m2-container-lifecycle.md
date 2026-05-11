# Reeve M2: Container Lifecycle — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `reeved` spawn a Docker container per card on `→ Implementing`, mount a per-card socket into it so in-container `reeve mv-self` works, and tear the container down on `→ Done`. Daemon restart reconciles surviving containers.

**Architecture:** New daemon modules `container.rs`, `transitions.rs`, `household.rs`. Per-card Unix sockets at `$XDG_RUNTIME_DIR/reeve/cards/<id>.sock` provide caller identity without peer-credential inspection — the daemon resolves the calling card from which listener received the request. `mv-self` and `status-self` are self-scoped RPC methods rejected on the host socket with `FORBIDDEN: 403`. Containers persist across `reeved` shutdown; reconcile-on-startup re-binds per-card listeners.

**Tech Stack:** Rust 2024, tokio, rusqlite, serde+serde_json, **toml** (new), clap, ratatui, tracing, anyhow+thiserror. Shells out to the `docker` CLI via `tokio::process::Command` (no `bollard` SDK in M2).

**Spec:** `docs/superpowers/specs/2026-05-11-m2-container-lifecycle-design.md`.

---

## File structure (created or modified in this milestone)

```
reeve/
├── Cargo.toml                          # add: toml workspace dep
├── crates/
│   ├── reeve-protocol/
│   │   ├── Cargo.toml                  # no change
│   │   └── src/lib.rs                  # add: MvSelf/StatusSelf types,
│   │                                   #      ContainerStatus, FORBIDDEN,
│   │                                   #      cards_socket_dir, card_socket_path,
│   │                                   #      detect_socket
│   ├── reeved/
│   │   ├── Cargo.toml                  # add: toml dep
│   │   └── src/
│   │       ├── main.rs                 # load households + reconcile on startup
│   │       ├── server.rs               # CallerContext threaded through dispatch
│   │       ├── handlers.rs             # mv-self, status-self, transitions hook
│   │       ├── db.rs                   # containers table + CRUD
│   │       ├── lifecycle.rs            # add: config_dir(), cards_socket_dir()
│   │       ├── household.rs            # NEW: TOML profile loader
│   │       ├── container.rs            # NEW: docker subprocess wrapper
│   │       ├── transitions.rs          # NEW: column-change side effects
│   │       └── listeners.rs            # NEW: per-card listener registry
│   └── reeve/
│       └── src/
│           ├── cli.rs                  # add: mv-self, status-self subcommands
│           └── client.rs               # may need socket path helper
└── tests/
    └── e2e.rs                          # add: docker-gated container lifecycle tests
```

Tests are colocated via `#[cfg(test)] mod tests { ... }`. Docker-gated tests in `tests/e2e.rs` skip with a printed message when `docker info` fails.

---

## Task 1: `toml` workspace dep + household profile loader

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/reeved/Cargo.toml`
- Modify: `crates/reeved/src/lifecycle.rs`
- Create: `crates/reeved/src/household.rs`
- Modify: `crates/reeved/src/main.rs` (mod declaration only)

- [ ] **Step 1: Add `toml` to workspace deps**

Edit `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
toml = "0.8"
```

- [ ] **Step 2: Add `toml` to reeved**

Edit `crates/reeved/Cargo.toml`, add to `[dependencies]`:

```toml
toml = { workspace = true }
```

- [ ] **Step 3: Add `config_dir()` helper to lifecycle.rs**

Append to `crates/reeved/src/lifecycle.rs`:

```rust
/// Returns the directory for persistent user configuration.
/// Prefers `$XDG_CONFIG_HOME/reeve`; falls back to `$HOME/.config/reeve`.
pub fn config_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir).join("reeve"));
        }
    }
    let home = std::env::var("HOME").context("$HOME not set")?;
    Ok(PathBuf::from(home).join(".config/reeve"))
}

pub fn households_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("households"))
}
```

- [ ] **Step 4: Write the failing household test**

Create `crates/reeved/src/household.rs`:

```rust
//! Household profile loader. M2 reads a minimal schema (image + cmd).
//! Unknown fields are tolerated so M3+ can extend without breaking M2 profiles.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HouseholdProfile {
    pub image: String,
    pub cmd: Vec<String>,
}

pub type HouseholdMap = Arc<HashMap<String, HouseholdProfile>>;

/// Load every `<name>.toml` from `dir` into a map keyed by `<name>`.
/// A missing dir returns an empty map; malformed TOML is an error.
pub fn load_all(dir: &Path) -> Result<HouseholdMap> {
    let mut out = HashMap::new();
    if !dir.exists() {
        return Ok(Arc::new(out));
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .with_context(|| format!("bad filename {}", path.display()))?
            .to_string();
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let profile: HouseholdProfile =
            toml::from_str(&body).with_context(|| format!("parse {}", path.display()))?;
        out.insert(name, profile);
    }
    Ok(Arc::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_minimal_profile() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "image = \"alpine:latest\"\ncmd = [\"sleep\", \"infinity\"]\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        assert_eq!(p.image, "alpine:latest");
        assert_eq!(p.cmd, vec!["sleep".to_string(), "infinity".to_string()]);
    }

    #[test]
    fn missing_dir_yields_empty_map() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        let map = load_all(&missing).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn ignores_non_toml_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "hello").unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "image = \"a\"\ncmd = [\"b\"]\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("test"));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "image = \"a\"\ncmd = [\"b\"]\nfuture_field = \"ignored\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert!(map.contains_key("test"));
    }

    #[test]
    fn malformed_toml_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("test.toml"), "image = (not valid").unwrap();
        assert!(load_all(dir.path()).is_err());
    }
}
```

- [ ] **Step 5: Register the module**

Edit `crates/reeved/src/main.rs`, replace the module list near the top:

```rust
mod db;
mod handlers;
mod household;
mod lifecycle;
mod server;
```

- [ ] **Step 6: Run tests; expect failure on undeclared module**

Run: `cargo test -p reeved household`
Expected: compile error if `toml` dep missing — fix Cargo.toml entries; then the four tests pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/reeved/Cargo.toml crates/reeved/src/lifecycle.rs crates/reeved/src/household.rs crates/reeved/src/main.rs
git commit -m "feat(m2): household profile loader (image + cmd, minimal schema)"
```

---

## Task 2: Protocol types for self-scoped RPC + container status + path helpers

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Write failing tests for the new types**

In `crates/reeve-protocol/src/lib.rs`, append inside the existing `#[cfg(test)] mod tests` block (just before its closing `}`):

```rust
    #[test]
    fn mv_self_params_serialize_with_just_column() {
        let p = MvSelfParams { column: Column::Review };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "{\"column\":\"review\"}");
    }

    #[test]
    fn status_self_result_round_trip() {
        let card = Card {
            id: "x".into(),
            household: "h".into(),
            brief: "b".into(),
            column: Column::Implementing,
            created_at: 1,
            updated_at: 1,
            needs_heavy: false,
            parent_id: None,
        };
        let r = StatusSelfResult {
            card: card.clone(),
            container_status: Some(ContainerStatus::Running),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: StatusSelfResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn container_status_serializes_to_lowercase() {
        assert_eq!(
            serde_json::to_string(&ContainerStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::from_str::<ContainerStatus>("\"failed\"").unwrap(),
            ContainerStatus::Failed
        );
    }

    #[test]
    fn forbidden_code_is_403() {
        assert_eq!(RpcError::FORBIDDEN, 403);
    }
```

Also append a path helper test in the same block:

```rust
    #[test]
    fn card_socket_path_lives_under_cards_socket_dir() {
        let p = paths::card_socket_path("abc-123");
        assert!(p.ends_with("reeve/cards/abc-123.sock"));
        assert_eq!(p.parent().unwrap(), paths::cards_socket_dir());
    }
```

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p reeve-protocol`
Expected: compile errors — types not defined.

- [ ] **Step 3: Add `FORBIDDEN` constant**

In the same file, extend the `RpcError` impl block:

```rust
impl RpcError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;
    pub const NOT_FOUND: i32 = 404;
    pub const FORBIDDEN: i32 = 403;
}
```

- [ ] **Step 4: Add `ContainerStatus` enum**

After the `MvResult` struct definition, add:

```rust
/// Lifecycle state of a card's container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerStatus {
    Spawning,
    Running,
    Stopped,
    Failed,
}

impl ContainerStatus {
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            ContainerStatus::Spawning => "spawning",
            ContainerStatus::Running => "running",
            ContainerStatus::Stopped => "stopped",
            ContainerStatus::Failed => "failed",
        }
    }

    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "spawning" => Some(ContainerStatus::Spawning),
            "running" => Some(ContainerStatus::Running),
            "stopped" => Some(ContainerStatus::Stopped),
            "failed" => Some(ContainerStatus::Failed),
            _ => None,
        }
    }
}
```

- [ ] **Step 5: Add self-scoped RPC params/results**

After `ContainerStatus`, add:

```rust
/// Parameters for the `mv-self` method. The card-id is implicit
/// from the per-card socket the request arrives on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvSelfParams {
    pub column: Column,
}

/// Result for the `mv-self` method. Returns the updated card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvSelfResult {
    pub card: Card,
}

/// Parameters for the `status-self` method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSelfParams {}

/// Result for the `status-self` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSelfResult {
    pub card: Card,
    pub container_status: Option<ContainerStatus>,
}
```

- [ ] **Step 6: Add path helpers**

Inside the existing `pub mod paths { ... }` block, after `pid_path`, append:

```rust
    /// Directory that holds per-card socket files. One file per active card.
    pub fn cards_socket_dir() -> PathBuf {
        runtime_dir().join("reeve/cards")
    }

    /// Socket path the daemon binds for a given card. The same path is
    /// bind-mounted into the card's container at /var/run/reeved.sock.
    pub fn card_socket_path(card_id: &str) -> PathBuf {
        cards_socket_dir().join(format!("{card_id}.sock"))
    }

    /// In-container socket the agent's `reeve` CLI sees after mount.
    pub const IN_CONTAINER_SOCKET: &str = "/var/run/reeved.sock";

    /// Pick the socket path a `reeve` CLI process should connect to.
    /// In-container processes find the mounted socket at `IN_CONTAINER_SOCKET`;
    /// host processes fall back to the daemon's host socket.
    pub fn detect_socket() -> PathBuf {
        let in_container = std::path::Path::new(IN_CONTAINER_SOCKET);
        if in_container.exists() {
            in_container.to_path_buf()
        } else {
            socket_path()
        }
    }
```

- [ ] **Step 7: Run tests; expect pass**

Run: `cargo test -p reeve-protocol`
Expected: all tests pass, including the five new ones.

- [ ] **Step 8: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(m2): protocol types for mv-self/status-self + ContainerStatus + per-card socket paths"
```

---

## Task 3: `containers` SQLite table and CRUD

**Files:**
- Modify: `crates/reeved/src/db.rs`

- [ ] **Step 1: Write failing tests**

Append inside the existing `#[cfg(test)] mod tests { ... }` block in `crates/reeved/src/db.rs`:

```rust
    use reeve_protocol::ContainerStatus;

    #[test]
    fn upsert_then_get_container_round_trip() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Spawning, 1000).unwrap();
        let row = get_container(&conn, "c1").unwrap().unwrap();
        assert_eq!(row.container_name, "reeve-c1");
        assert_eq!(row.status, ContainerStatus::Spawning);
        assert_eq!(row.started_at, 1000);
    }

    #[test]
    fn update_container_status_changes_status_and_updated_at() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Spawning, 1000).unwrap();
        update_container_status(&conn, "c1", ContainerStatus::Running, 2000).unwrap();
        let row = get_container(&conn, "c1").unwrap().unwrap();
        assert_eq!(row.status, ContainerStatus::Running);
        assert_eq!(row.updated_at, 2000);
    }

    #[test]
    fn list_containers_by_status_filters() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("a")).unwrap();
        insert_card(&conn, &sample_card("b")).unwrap();
        upsert_container(&conn, "a", "reeve-a", ContainerStatus::Running, 1).unwrap();
        upsert_container(&conn, "b", "reeve-b", ContainerStatus::Stopped, 1).unwrap();
        let running = list_containers_by_status(&conn, ContainerStatus::Running).unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].card_id, "a");
    }

    #[test]
    fn remove_container_deletes_row() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Running, 1).unwrap();
        remove_container(&conn, "c1").unwrap();
        assert!(get_container(&conn, "c1").unwrap().is_none());
    }

    #[test]
    fn cascade_delete_when_card_removed() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Running, 1).unwrap();
        conn.execute("DELETE FROM cards WHERE id = ?1", params!["c1"]).unwrap();
        assert!(get_container(&conn, "c1").unwrap().is_none());
    }
```

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p reeved db::tests`
Expected: compile errors — types and functions not defined.

- [ ] **Step 3: Extend the schema**

In `crates/reeved/src/db.rs`, replace the body of `migrate`:

```rust
fn migrate(conn: &Connection) -> Result<()> {
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
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);

         CREATE TABLE IF NOT EXISTS containers (
            card_id        TEXT PRIMARY KEY REFERENCES cards(id) ON DELETE CASCADE,
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

- [ ] **Step 4: Add the `ContainerRow` struct and CRUD functions**

In the same file, after `update_card_column`, append:

```rust
use reeve_protocol::ContainerStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRow {
    pub card_id: String,
    pub container_name: String,
    pub status: ContainerStatus,
    pub started_at: i64,
    pub updated_at: i64,
}

pub fn upsert_container(
    conn: &Connection,
    card_id: &str,
    container_name: &str,
    status: ContainerStatus,
    now: i64,
) -> Result<()> {
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
}

pub fn update_container_status(
    conn: &Connection,
    card_id: &str,
    status: ContainerStatus,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE containers SET status = ?1, updated_at = ?2 WHERE card_id = ?3",
        params![status.as_wire_str(), now, card_id],
    )?;
    Ok(())
}

pub fn get_container(conn: &Connection, card_id: &str) -> Result<Option<ContainerRow>> {
    let mut stmt = conn.prepare(
        "SELECT card_id, container_name, status, started_at, updated_at
         FROM containers WHERE card_id = ?1",
    )?;
    let row = stmt.query_row(params![card_id], row_to_container).optional()?;
    Ok(row)
}

pub fn list_containers_by_status(
    conn: &Connection,
    status: ContainerStatus,
) -> Result<Vec<ContainerRow>> {
    let mut stmt = conn.prepare(
        "SELECT card_id, container_name, status, started_at, updated_at
         FROM containers WHERE status = ?1",
    )?;
    let rows = stmt.query_map(params![status.as_wire_str()], row_to_container)?;
    let result: Result<Vec<_>, _> = rows.collect();
    Ok(result?)
}

pub fn remove_container(conn: &Connection, card_id: &str) -> Result<()> {
    conn.execute("DELETE FROM containers WHERE card_id = ?1", params![card_id])?;
    Ok(())
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
```

- [ ] **Step 5: Run; expect pass**

Run: `cargo test -p reeved db::tests`
Expected: all tests pass, including the five new ones.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/db.rs
git commit -m "feat(m2): containers table + CRUD (upsert, get, list-by-status, remove)"
```

---

## Task 4: Thread `CallerContext` through server → dispatch

**Files:**
- Modify: `crates/reeved/src/server.rs`
- Modify: `crates/reeved/src/handlers.rs`
- Modify: `crates/reeved/src/main.rs`

This task is a refactor with **no behavior change**. The host socket continues to work; all existing tests pass. `CallerContext` is plumbed through and `dispatch` accepts it as a parameter, ready for self-scoped methods in Task 5.

- [ ] **Step 1: Define `CallerContext` in server.rs**

At the top of `crates/reeved/src/server.rs`, after the existing imports and `DbHandle` alias, add:

```rust
/// Caller identity attached to each incoming RPC. The host socket
/// produces `CallerContext { card_id: None }`; per-card sockets
/// produce `CallerContext { card_id: Some("<id>") }`.
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub card_id: Option<String>,
}

impl CallerContext {
    pub fn host() -> Self {
        Self { card_id: None }
    }

    pub fn card(id: impl Into<String>) -> Self {
        Self { card_id: Some(id.into()) }
    }
}
```

- [ ] **Step 2: Change the dispatch trait bound on `run`**

In the same file, replace the existing `pub async fn run<F, Fut>(...)` signature and dispatch invocations:

```rust
pub async fn run<F, Fut>(
    socket_path: &Path,
    db: DbHandle,
    caller: CallerContext,
    mut shutdown: oneshot::Receiver<()>,
    dispatch: F,
) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle, CallerContext) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    // ... existing body up to the accept loop unchanged ...
```

Inside the accept loop, replace the `connections.spawn(...)` body so the per-connection task captures `caller`:

```rust
                    Ok((stream, _addr)) => {
                        let db = db.clone();
                        let dispatch = dispatch.clone();
                        let caller = caller.clone();
                        connections.spawn(async move {
                            if let Err(e) = handle_connection(stream, db, caller, dispatch).await {
                                warn!("connection error: {e:#}");
                            }
                        });
                    }
```

And update `handle_connection`:

```rust
async fn handle_connection<F, Fut>(
    stream: UnixStream,
    db: DbHandle,
    caller: CallerContext,
    dispatch: F,
) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle, CallerContext) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => {
                let id = req.id;
                match dispatch(req, db.clone(), caller.clone()).await {
                    Ok(value) => RpcResponse {
                        id: Some(id),
                        result: Some(value),
                        error: None,
                    },
                    Err(err) => RpcResponse {
                        id: Some(id),
                        result: None,
                        error: Some(err),
                    },
                }
            }
            Err(e) => RpcResponse {
                id: None,
                result: None,
                error: Some(RpcError {
                    code: RpcError::INVALID_PARAMS,
                    message: format!("malformed request: {e}"),
                }),
            },
        };
        let mut bytes = serde_json::to_vec(&resp)?;
        bytes.push(b'\n');
        write.write_all(&bytes).await?;
    }
    Ok(())
}
```

- [ ] **Step 3: Update server.rs tests for the new signature**

In the same file's `#[cfg(test)] mod tests` block, update each test that spawns `run` to also pass `CallerContext::host()` and update each dispatch closure to take three params:

```rust
        let dispatch = |req: RpcRequest, _db: DbHandle, _caller: CallerContext| async move {
            Ok(req.params)
        };
```

And each `run(...)` call:

```rust
            run(&socket_clone, db, CallerContext::host(), rx, dispatch).await.unwrap();
```

Apply this consistently across all four tests in that module.

- [ ] **Step 4: Update handlers::dispatch signature**

In `crates/reeved/src/handlers.rs`, replace the signature and add a `caller` param that's currently unused:

```rust
use crate::server::{CallerContext, DbHandle};

pub async fn dispatch(
    req: RpcRequest,
    db: DbHandle,
    _caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => handle_ls(db).await,
        "new" => {
            let params = parse_params::<NewParams>(req.params)?;
            handle_new(db, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            handle_mv(db, params).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}
```

Remove the standalone `use crate::server::DbHandle;` line (now bundled with `CallerContext` above).

- [ ] **Step 5: Update handlers tests to pass `CallerContext::host()`**

In the `#[cfg(test)] mod tests` block, update every `dispatch(req, db)` call to `dispatch(req, db, CallerContext::host())`. Add `use crate::server::CallerContext;` at the top of the tests module.

- [ ] **Step 6: Update `main.rs` to pass `CallerContext::host()`**

In `crates/reeved/src/main.rs`, replace the `server::run` call:

```rust
    let result = server::run(
        &socket_path,
        db,
        server::CallerContext::host(),
        shutdown_rx,
        handlers::dispatch,
    ).await;
```

- [ ] **Step 7: Run the workspace tests**

Run: `cargo test --workspace`
Expected: all tests pass — refactor is behavior-preserving.

- [ ] **Step 8: Commit**

```bash
git add crates/reeved/src/server.rs crates/reeved/src/handlers.rs crates/reeved/src/main.rs
git commit -m "refactor(m2): thread CallerContext through server + dispatch (no behavior change)"
```

---

## Task 5: `mv-self` and `status-self` handlers with `FORBIDDEN` enforcement

**Files:**
- Modify: `crates/reeved/src/handlers.rs`

- [ ] **Step 1: Write failing handler tests**

Append inside the `#[cfg(test)] mod tests` block in `crates/reeved/src/handlers.rs`:

```rust
    #[tokio::test]
    async fn mv_self_on_host_socket_returns_forbidden() {
        let db = fresh_db();
        let req = RpcRequest {
            id: 1,
            method: "mv-self".into(),
            params: serde_json::json!({"column": "review"}),
        };
        let err = dispatch(req, db, CallerContext::host()).await.unwrap_err();
        assert_eq!(err.code, RpcError::FORBIDDEN);
    }

    #[tokio::test]
    async fn mv_self_on_card_socket_moves_card() {
        let db = fresh_db();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "b",
                    "needs_heavy": false,
                }),
            },
            db.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv-self".into(),
                params: serde_json::json!({"column": "review"}),
            },
            db,
            CallerContext::card(&card.id),
        )
        .await
        .unwrap();
        let result: reeve_protocol::MvSelfResult = serde_json::from_value(mv_value).unwrap();
        assert_eq!(result.card.column, Column::Review);
        assert_eq!(result.card.id, card.id);
    }

    #[tokio::test]
    async fn status_self_on_host_socket_returns_forbidden() {
        let db = fresh_db();
        let req = RpcRequest {
            id: 1,
            method: "status-self".into(),
            params: serde_json::json!({}),
        };
        let err = dispatch(req, db, CallerContext::host()).await.unwrap_err();
        assert_eq!(err.code, RpcError::FORBIDDEN);
    }

    #[tokio::test]
    async fn status_self_on_card_socket_returns_card_no_container() {
        let db = fresh_db();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "b",
                    "needs_heavy": false,
                }),
            },
            db.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let value = dispatch(
            RpcRequest {
                id: 2,
                method: "status-self".into(),
                params: serde_json::json!({}),
            },
            db,
            CallerContext::card(&card.id),
        )
        .await
        .unwrap();
        let result: reeve_protocol::StatusSelfResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.card.id, card.id);
        assert!(result.container_status.is_none());
    }
```

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p reeved handlers::tests`
Expected: tests fail with `METHOD_NOT_FOUND` for `mv-self` and `status-self`.

- [ ] **Step 3: Extend `dispatch` to route the new methods**

In `crates/reeved/src/handlers.rs`, replace the existing `pub async fn dispatch` body:

```rust
pub async fn dispatch(
    req: RpcRequest,
    db: DbHandle,
    caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => handle_ls(db).await,
        "new" => {
            let params = parse_params::<NewParams>(req.params)?;
            handle_new(db, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            handle_mv(db, params).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let params = parse_params::<MvSelfParams>(req.params)?;
            handle_mv_self(db, card_id, params).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            handle_status_self(db, card_id).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}

fn require_card(caller: &CallerContext) -> std::result::Result<String, RpcError> {
    caller.card_id.clone().ok_or_else(|| RpcError {
        code: RpcError::FORBIDDEN,
        message: "self-scoped methods require per-card socket".into(),
    })
}
```

Update the imports at the top of the file:

```rust
use reeve_protocol::{
    Card, Column, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult, NewParams, NewResult,
    RpcError, RpcRequest, StatusSelfResult,
};
```

- [ ] **Step 4: Implement `handle_mv_self` and `handle_status_self`**

Append after `handle_mv`:

```rust
async fn handle_mv_self(
    db: DbHandle,
    card_id: String,
    params: MvSelfParams,
) -> std::result::Result<Value, RpcError> {
    let column = params.column;
    let now = now_secs();
    let id = card_id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id, column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    match card {
        Some(card) => serde_json::to_value(MvSelfResult { card }).map_err(to_internal),
        None => Err(RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card not found: {card_id}"),
        }),
    }
}

async fn handle_status_self(
    db: DbHandle,
    card_id: String,
) -> std::result::Result<Value, RpcError> {
    let id = card_id.clone();
    let (card, container) = tokio::task::spawn_blocking(
        move || -> Result<(Option<Card>, Option<db::ContainerRow>)> {
            let conn = db.blocking_lock();
            let card = db::get_card(&conn, &id)?;
            let container = db::get_container(&conn, &id)?;
            Ok((card, container))
        },
    )
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    let card = card.ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;
    serde_json::to_value(StatusSelfResult {
        card,
        container_status: container.map(|c| c.status),
    })
    .map_err(to_internal)
}
```

- [ ] **Step 5: Run; expect pass**

Run: `cargo test -p reeved handlers::tests`
Expected: all tests pass, including the four new ones.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/handlers.rs
git commit -m "feat(m2): mv-self and status-self handlers, FORBIDDEN on host socket"
```

---

## Task 6: `container::spawn_command` pure-function argv builder

**Files:**
- Create: `crates/reeved/src/container.rs`
- Modify: `crates/reeved/src/main.rs` (mod declaration)

- [ ] **Step 1: Write the failing test first (TDD)**

Create `crates/reeved/src/container.rs`:

```rust
//! Docker subprocess wrapper. The pure parts (argv building) live here;
//! the impure parts (shelling out, parsing output) are in `DockerOps`.

use crate::household::HouseholdProfile;
use std::path::{Path, PathBuf};

/// Container name for a card. Derived deterministically from card-id.
pub fn container_name(card_id: &str) -> String {
    format!("reeve-{card_id}")
}

/// Build the argv for `docker` (excluding the `docker` binary itself)
/// that spawns the container for `card_id` under `profile`.
///
/// - `card_socket` is the daemon-side per-card socket path; it's bind-mounted
///   to /var/run/reeved.sock inside the container.
/// - `claude_dir` is the host `~/.claude` whose `skills/`, `settings.json`,
///   and `CLAUDE.md` are mounted read-only. Missing children are skipped
///   silently (the caller checks existence).
/// - `api_key` is injected as `ANTHROPIC_API_KEY` when `Some`.
pub fn spawn_command(
    card_id: &str,
    profile: &HouseholdProfile,
    card_socket: &Path,
    claude_dir: &Path,
    api_key: Option<&str>,
) -> Vec<String> {
    let name = container_name(card_id);
    let mut args = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        name.clone(),
        "--label".into(),
        format!("reeve.card_id={card_id}"),
        "-v".into(),
        format!(
            "{}:{}",
            card_socket.display(),
            reeve_protocol::paths::IN_CONTAINER_SOCKET
        ),
    ];

    let mounts: [(PathBuf, &str); 3] = [
        (claude_dir.join("skills"), "/home/agent/.claude/skills"),
        (claude_dir.join("settings.json"), "/home/agent/.claude/settings.json"),
        (claude_dir.join("CLAUDE.md"), "/home/agent/.claude/CLAUDE.md"),
    ];
    for (host, container) in mounts {
        if host.exists() {
            args.push("-v".into());
            args.push(format!("{}:{}:ro", host.display(), container));
        }
    }

    if let Some(key) = api_key {
        args.push("-e".into());
        args.push(format!("ANTHROPIC_API_KEY={key}"));
    }
    args.push("-e".into());
    args.push(format!("REEVE_CARD_ID={card_id}"));

    args.push(profile.image.clone());
    args.extend(profile.cmd.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn profile() -> HouseholdProfile {
        HouseholdProfile {
            image: "alpine:latest".into(),
            cmd: vec!["sleep".into(), "infinity".into()],
        }
    }

    #[test]
    fn container_name_prefixes_with_reeve() {
        assert_eq!(container_name("abc-123"), "reeve-abc-123");
    }

    #[test]
    fn spawn_command_includes_name_label_socket_image_and_cmd() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let claude = dir.path().join("claude");
        let argv = spawn_command("abc", &profile(), &socket, &claude, None);
        assert_eq!(argv[0], "run");
        assert!(argv.iter().any(|s| s == "reeve-abc"));
        assert!(argv.iter().any(|s| s == "reeve.card_id=abc"));
        assert!(argv.iter().any(|s| s.starts_with(&format!("{}:", socket.display()))));
        assert_eq!(argv.last().unwrap(), "infinity");
        assert_eq!(argv[argv.len() - 2], "sleep");
        let image_idx = argv.iter().position(|s| s == "alpine:latest").unwrap();
        assert!(image_idx < argv.len() - 1, "image precedes cmd");
    }

    #[test]
    fn spawn_command_skips_missing_claude_mounts() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let claude = dir.path().join("claude");
        // claude_dir does not exist
        let argv = spawn_command("abc", &profile(), &socket, &claude, None);
        let claude_mounts = argv
            .iter()
            .filter(|s| s.contains("/.claude/"))
            .count();
        assert_eq!(claude_mounts, 0);
    }

    #[test]
    fn spawn_command_includes_only_existing_claude_mounts() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("skills")).unwrap();
        std::fs::write(claude.join("CLAUDE.md"), "x").unwrap();
        // settings.json deliberately missing

        let argv = spawn_command("abc", &profile(), &socket, &claude, None);
        assert!(argv.iter().any(|s| s.contains("/.claude/skills:")));
        assert!(argv.iter().any(|s| s.contains("/CLAUDE.md:")));
        assert!(!argv.iter().any(|s| s.contains("settings.json")));
    }

    #[test]
    fn spawn_command_injects_api_key_only_when_some() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let claude = dir.path().join("claude");

        let with = spawn_command("abc", &profile(), &socket, &claude, Some("sk-test"));
        assert!(with.iter().any(|s| s == "ANTHROPIC_API_KEY=sk-test"));

        let without = spawn_command("abc", &profile(), &socket, &claude, None);
        assert!(!without.iter().any(|s| s.starts_with("ANTHROPIC_API_KEY=")));
    }

    #[test]
    fn spawn_command_always_sets_card_id_env() {
        let dir = tempdir().unwrap();
        let argv = spawn_command(
            "card-42",
            &profile(),
            &dir.path().join("s.sock"),
            &dir.path().join("c"),
            None,
        );
        assert!(argv.iter().any(|s| s == "REEVE_CARD_ID=card-42"));
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/reeved/src/main.rs`, add `mod container;` so the module list reads:

```rust
mod container;
mod db;
mod handlers;
mod household;
mod lifecycle;
mod server;
```

- [ ] **Step 3: Run; expect pass**

Run: `cargo test -p reeved container`
Expected: all six tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/src/container.rs crates/reeved/src/main.rs
git commit -m "feat(m2): container::spawn_command pure-function argv builder"
```

---

## Task 7: `DockerOps` runtime — spawn, teardown, inspect (Docker-gated)

**Files:**
- Modify: `crates/reeved/src/container.rs`

- [ ] **Step 1: Add the `ContainerOps` trait and `DockerOps` impl**

Append to `crates/reeved/src/container.rs`:

```rust
use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::ContainerStatus;
use std::sync::Arc;
use tokio::process::Command;
use tracing::warn;

/// Abstract container backend. Trait so we can mock it in tests for the
/// transitions module (Task 8) without requiring docker on every run.
#[async_trait]
pub trait ContainerOps: Send + Sync {
    async fn spawn(&self, argv: Vec<String>) -> Result<()>;
    async fn teardown(&self, container_name: &str) -> Result<()>;
    /// Returns the docker-side state of the container, or `None` if it doesn't exist.
    async fn inspect_status(&self, container_name: &str) -> Result<Option<String>>;
}

/// Real Docker implementation. Shells out to `docker` via tokio.
pub struct DockerOps;

#[async_trait]
impl ContainerOps for DockerOps {
    async fn spawn(&self, argv: Vec<String>) -> Result<()> {
        let output = Command::new("docker")
            .args(&argv)
            .output()
            .await
            .context("exec docker run")?;
        if !output.status.success() {
            anyhow::bail!(
                "docker run failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn teardown(&self, container_name: &str) -> Result<()> {
        let stop = Command::new("docker")
            .args(["stop", "--time", "5", container_name])
            .output()
            .await
            .context("exec docker stop")?;
        if !stop.status.success() {
            // Container may have already exited; log and continue to rm.
            warn!(
                "docker stop {container_name} non-zero: {}",
                String::from_utf8_lossy(&stop.stderr)
            );
        }
        let rm = Command::new("docker")
            .args(["rm", "-f", container_name])
            .output()
            .await
            .context("exec docker rm")?;
        if !rm.status.success() {
            anyhow::bail!(
                "docker rm -f {container_name} failed: {}",
                String::from_utf8_lossy(&rm.stderr)
            );
        }
        Ok(())
    }

    async fn inspect_status(&self, container_name: &str) -> Result<Option<String>> {
        let output = Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.State.Status}}",
                container_name,
            ])
            .output()
            .await
            .context("exec docker inspect")?;
        if !output.status.success() {
            // "No such container" → status::None
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("No such") {
                return Ok(None);
            }
            anyhow::bail!("docker inspect failed: {stderr}");
        }
        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(status))
    }
}

/// Maps a `docker inspect` `.State.Status` string to our `ContainerStatus`.
pub fn map_docker_state(s: &str) -> ContainerStatus {
    match s {
        "running" => ContainerStatus::Running,
        "created" | "restarting" => ContainerStatus::Spawning,
        _ => ContainerStatus::Failed, // exited, dead, paused, removing
    }
}

/// Convenience handle used by the rest of the daemon.
pub type ContainerOpsHandle = Arc<dyn ContainerOps>;

#[cfg(test)]
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod docker_tests {
    use super::*;

    #[tokio::test]
    async fn inspect_unknown_container_returns_none() {
        if !docker_available() {
            eprintln!("skipping: docker unavailable");
            return;
        }
        let ops = DockerOps;
        let status = ops.inspect_status("reeve-nonexistent-test").await.unwrap();
        assert!(status.is_none());
    }

    #[tokio::test]
    async fn spawn_inspect_teardown_round_trip() {
        if !docker_available() {
            eprintln!("skipping: docker unavailable");
            return;
        }
        let ops = DockerOps;
        let name = "reeve-test-roundtrip";
        // Best-effort cleanup of any leftover from a previous run.
        let _ = ops.teardown(name).await;

        let argv = vec![
            "run".into(), "-d".into(), "--name".into(), name.into(),
            "alpine:latest".into(), "sleep".into(), "60".into(),
        ];
        ops.spawn(argv).await.unwrap();

        let status = ops.inspect_status(name).await.unwrap();
        assert_eq!(status.as_deref(), Some("running"));

        ops.teardown(name).await.unwrap();
        let status = ops.inspect_status(name).await.unwrap();
        assert!(status.is_none());
    }

    #[test]
    fn map_docker_state_covers_known_states() {
        assert_eq!(map_docker_state("running"), ContainerStatus::Running);
        assert_eq!(map_docker_state("created"), ContainerStatus::Spawning);
        assert_eq!(map_docker_state("restarting"), ContainerStatus::Spawning);
        assert_eq!(map_docker_state("exited"), ContainerStatus::Failed);
        assert_eq!(map_docker_state("dead"), ContainerStatus::Failed);
    }
}
```

- [ ] **Step 2: Add `async-trait` to reeved deps**

Edit `crates/reeved/Cargo.toml`, append to `[dependencies]`:

```toml
async-trait = "0.1"
```

- [ ] **Step 3: Run tests; verify docker-gated tests pass when docker is available**

Run: `cargo test -p reeved container`
Expected: all unit tests pass (Task 6 + `map_docker_state` here). Docker-gated tests pass if `docker info` succeeds, otherwise print "skipping" and return.

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/Cargo.toml crates/reeved/src/container.rs
git commit -m "feat(m2): DockerOps runtime (spawn, teardown, inspect) with gated tests"
```

---

## Task 8: `transitions::on_column_change` (column-change side effects)

**Files:**
- Create: `crates/reeved/src/transitions.rs`
- Modify: `crates/reeved/src/main.rs` (mod declaration)

- [ ] **Step 1: Write the failing tests with a mock effects backend**

Create `crates/reeved/src/transitions.rs`:

```rust
//! Column-transition side effects. Decides what container ops to invoke
//! when a card moves between columns. This is the only module that
//! encodes the transition matrix.

use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{Card, Column};

#[async_trait]
pub trait TransitionEffects: Send + Sync {
    /// Card needs a running container (spawn if not already).
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    /// Card no longer needs a container (teardown if running).
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

pub async fn on_column_change(
    old: Column,
    new: Column,
    card: &Card,
    fx: &dyn TransitionEffects,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    match new {
        Column::Implementing => fx.ensure_running(card).await,
        Column::Testing | Column::Review => {
            // Container stays warm if it was already running; otherwise
            // we don't spawn one. Skipping straight from Backlog to Review
            // is allowed but the user has nothing to run there yet.
            Ok(())
        }
        Column::Done => fx.ensure_stopped(&card.id).await,
        Column::Backlog | Column::Planning => {
            // Defensive: if a container exists from a prior Implementing
            // run, tear it down. Normal flow doesn't go back here, but
            // we tolerate it.
            fx.ensure_stopped(&card.id).await
        }
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
    impl TransitionEffects for MockFx {
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
    async fn implementing_to_review_is_noop() {
        let fx = run_one(Column::Implementing, Column::Review).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_testing_is_noop() {
        let fx = run_one(Column::Implementing, Column::Testing).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_done_tears_down() {
        let fx = run_one(Column::Implementing, Column::Done).await;
        assert!(fx.running.lock().await.is_empty());
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn review_to_done_tears_down() {
        let fx = run_one(Column::Review, Column::Done).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn done_back_to_backlog_defensively_tears_down() {
        let fx = run_one(Column::Done, Column::Backlog).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn same_column_is_noop() {
        let fx = run_one(Column::Implementing, Column::Implementing).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/reeved/src/main.rs`, the module list:

```rust
mod container;
mod db;
mod handlers;
mod household;
mod lifecycle;
mod server;
mod transitions;
```

- [ ] **Step 3: Run; expect pass**

Run: `cargo test -p reeved transitions`
Expected: all seven tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/src/transitions.rs crates/reeved/src/main.rs
git commit -m "feat(m2): transitions::on_column_change with mock-tested matrix"
```

---

## Task 9: Per-card socket listener registry

**Files:**
- Create: `crates/reeved/src/listeners.rs`
- Modify: `crates/reeved/src/main.rs` (mod declaration)

This task introduces the registry with its final API (`Dispatcher` trait + `spawn`). The trait lets the registry hold a single dispatcher reference without carrying generic parameters everywhere; Task 10 wires this to the live `handlers::dispatch`.

- [ ] **Step 1: Create `listeners.rs` with the final API**

Create `crates/reeved/src/listeners.rs`:

```rust
//! Per-card Unix-socket listeners. Each running card has its own
//! socket file at $XDG_RUNTIME_DIR/reeve/cards/<id>.sock; the daemon
//! resolves caller identity from which listener received the request.

use crate::server::{CallerContext, DbHandle, run};
use anyhow::Result;
use reeve_protocol::{RpcError, RpcRequest};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;
use tracing::warn;

/// Type-erased dispatcher so the registry can hold a single reference
/// without carrying generic parameters everywhere.
pub trait Dispatcher: Send + Sync {
    fn dispatch(
        &self,
        req: RpcRequest,
        db: DbHandle,
        caller: CallerContext,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>>
                + Send,
        >,
    >;
}

#[derive(Clone)]
pub struct ListenerRegistry {
    inner: Arc<Mutex<HashMap<String, ListenerEntry>>>,
    dispatcher: Arc<dyn Dispatcher>,
    db: DbHandle,
}

struct ListenerEntry {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
    socket: PathBuf,
}

impl ListenerRegistry {
    pub fn new(db: DbHandle, dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            dispatcher,
            db,
        }
    }

    /// Spawn a listener for `card_id` at `socket_path`. Idempotent: a
    /// second call with the same id is a no-op.
    pub async fn spawn(&self, card_id: &str, socket_path: PathBuf) {
        let mut map = self.inner.lock().await;
        if map.contains_key(card_id) {
            return;
        }
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
        // server::run unlinks any existing file at the socket path before bind.

        let caller = CallerContext::card(card_id);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let socket_for_task = socket_path.clone();
        let card_for_task = card_id.to_string();
        let dispatcher = self.dispatcher.clone();
        let db = self.db.clone();
        let dispatch = move |req: RpcRequest, db: DbHandle, caller: CallerContext| {
            dispatcher.dispatch(req, db, caller)
        };
        let task = tokio::spawn(async move {
            if let Err(e) = run(&socket_for_task, db, caller, shutdown_rx, dispatch).await {
                warn!("per-card listener for {card_for_task} exited: {e:#}");
            }
        });
        map.insert(
            card_id.to_string(),
            ListenerEntry {
                shutdown: shutdown_tx,
                task,
                socket: socket_path,
            },
        );
    }

    /// Stop the listener for `card_id` and remove the socket file.
    /// No-op if no listener exists for this id.
    pub async fn stop(&self, card_id: &str) -> Result<()> {
        let entry = self.inner.lock().await.remove(card_id);
        if let Some(entry) = entry {
            let _ = entry.shutdown.send(());
            let _ = entry.task.await;
            let _ = std::fs::remove_file(&entry.socket);
        }
        Ok(())
    }

    /// Drop every listener (used on daemon shutdown).
    pub async fn stop_all(&self) {
        let entries: Vec<_> = self.inner.lock().await.drain().collect();
        for (_, entry) in entries {
            let _ = entry.shutdown.send(());
            let _ = entry.task.await;
            let _ = std::fs::remove_file(&entry.socket);
        }
    }

    pub async fn contains(&self, card_id: &str) -> bool {
        self.inner.lock().await.contains_key(card_id)
    }

    /// Best-effort: unlink any socket file at `path` before binding.
    /// Per-card socket files survive daemon shutdown (containers do too).
    pub fn unlink_stale(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    /// Test dispatcher: echoes the calling card_id back to the client.
    struct EchoCaller;
    impl Dispatcher for EchoCaller {
        fn dispatch(
            &self,
            _req: RpcRequest,
            _db: DbHandle,
            caller: CallerContext,
        ) -> Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<serde_json::Value, RpcError>,
                    > + Send,
            >,
        > {
            Box::pin(async move { Ok(json!({ "card_id": caller.card_id })) })
        }
    }

    fn fresh_db() -> DbHandle {
        Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))
    }

    fn registry() -> ListenerRegistry {
        ListenerRegistry::new(fresh_db(), Arc::new(EchoCaller))
    }

    async fn connect_with_retry(socket: &Path) -> UnixStream {
        for _ in 0..50 {
            if let Ok(s) = UnixStream::connect(socket).await {
                return s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("connect timeout: {}", socket.display());
    }

    #[tokio::test]
    async fn listener_passes_card_id_in_caller_context() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let reg = registry();

        reg.spawn("test-card", socket.clone()).await;

        let mut stream = connect_with_retry(&socket).await;
        let req = RpcRequest {
            id: 1,
            method: "noop".into(),
            params: serde_json::json!({}),
        };
        let mut bytes = serde_json::to_vec(&req).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains("\"card_id\":\"test-card\""), "got: {line}");

        reg.stop("test-card").await.unwrap();
    }

    #[tokio::test]
    async fn spawn_is_idempotent_for_same_card_id() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let reg = registry();

        reg.spawn("c", socket.clone()).await;
        reg.spawn("c", socket.clone()).await;
        assert!(reg.contains("c").await);

        reg.stop("c").await.unwrap();
    }

    #[tokio::test]
    async fn stop_removes_socket_file() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let reg = registry();

        reg.spawn("c", socket.clone()).await;
        for _ in 0..50 {
            if socket.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(socket.exists());

        reg.stop("c").await.unwrap();
        assert!(!socket.exists());
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/reeved/src/main.rs`, the module list:

```rust
mod container;
mod db;
mod handlers;
mod household;
mod lifecycle;
mod listeners;
mod server;
mod transitions;
```

- [ ] **Step 3: Run; expect pass**

Run: `cargo test -p reeved listeners`
Expected: all three tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/src/listeners.rs crates/reeved/src/main.rs
git commit -m "feat(m2): per-card socket listener registry with Dispatcher trait"
```

---

## Task 10: Wire transitions into `handle_mv`; per-card listener spawn on container up/down

**Files:**
- Create: `crates/reeved/src/effects.rs`
- Modify: `crates/reeved/src/handlers.rs`
- Modify: `crates/reeved/src/main.rs` (mod declaration + dispatcher wiring)

This task introduces a `DaemonState` bundle that the dispatcher closure captures and assembles the live `TransitionEffects` implementation that talks to `ContainerOps`, the SQLite `containers` table, and the `ListenerRegistry`. It also adds a household precheck so that a move to `Implementing` against a missing household profile fails *before* the column is mutated.

- [ ] **Step 1: Create the daemon state bundle and live effects impl**

Create `crates/reeved/src/effects.rs`:

```rust
//! Live wiring of `TransitionEffects` for the running daemon.
//! Bundled with the `DaemonState` that the dispatcher closure captures.

use crate::container::{self, ContainerOpsHandle};
use crate::db;
use crate::household::HouseholdMap;
use crate::listeners::ListenerRegistry;
use crate::server::DbHandle;
use crate::transitions::TransitionEffects;
use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{Card, ContainerStatus};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Everything the live transition effects need to do their job.
#[derive(Clone)]
pub struct DaemonState {
    pub db: DbHandle,
    pub containers: ContainerOpsHandle,
    pub listeners: ListenerRegistry,
    pub households: HouseholdMap,
    pub claude_dir: PathBuf,
    pub api_key: Option<String>,
}

pub struct LiveEffects {
    state: DaemonState,
}

impl LiveEffects {
    pub fn new(state: DaemonState) -> Self {
        Self { state }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl TransitionEffects for LiveEffects {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        let card_id = card.id.clone();
        let name = container::container_name(&card_id);

        // Precheck on the host-side (handlers::handle_mv) already verified
        // the household exists. We re-look-up here because TransitionEffects
        // is the boundary the trait gives us.
        let profile = self
            .state
            .households
            .get(&card.household)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "household '{}' not configured (in ~/.config/reeve/households/)",
                    card.household,
                )
            })?;

        let socket = reeve_protocol::paths::card_socket_path(&card_id);

        // Already running? Just ensure the listener is up and return.
        if let Some(state) = self.state.containers.inspect_status(&name).await? {
            if state == "running" {
                self.state.listeners.spawn(&card_id, socket).await;
                return Ok(());
            }
        }

        // Record Spawning before launching docker.
        let id_for_record = card_id.clone();
        let name_for_record = name.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::upsert_container(
                &conn,
                &id_for_record,
                &name_for_record,
                ContainerStatus::Spawning,
                now,
            )
        })
        .await??;

        let argv = container::spawn_command(
            &card_id,
            &profile,
            &socket,
            &self.state.claude_dir,
            self.state.api_key.as_deref(),
        );
        let spawn_result = self.state.containers.spawn(argv).await;

        let new_status = match &spawn_result {
            Ok(()) => ContainerStatus::Running,
            Err(_) => ContainerStatus::Failed,
        };
        let id_for_update = card_id.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::update_container_status(&conn, &id_for_update, new_status, now)
        })
        .await??;
        spawn_result?;

        // Bring up the per-card listener so the agent can call back.
        self.state.listeners.spawn(&card_id, socket).await;
        Ok(())
    }

    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        // Tear down listener first so the agent can't issue more calls.
        self.state.listeners.stop(card_id).await?;

        let name = container::container_name(card_id);
        // Container may not exist (card moved Backlog → Done with no prior
        // Implementing). That's fine — just remove the row if present.
        if self.state.containers.inspect_status(&name).await?.is_none() {
            let id_for_remove = card_id.to_string();
            let db = self.state.db.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let conn = db.blocking_lock();
                db::remove_container(&conn, &id_for_remove)
            })
            .await??;
            return Ok(());
        }

        self.state.containers.teardown(&name).await?;
        let id_for_remove = card_id.to_string();
        let db = self.state.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::remove_container(&conn, &id_for_remove)
        })
        .await??;
        Ok(())
    }
}
```

- [ ] **Step 2: Update `handlers::handle_mv` and `handle_mv_self` to precheck + fire transitions**

Edit `crates/reeved/src/handlers.rs`. Replace the existing imports and add the new state parameter to `dispatch`:

```rust
use crate::db;
use crate::effects::{DaemonState, LiveEffects};
use crate::server::{CallerContext, DbHandle};
use crate::transitions;
use anyhow::Result;
use reeve_protocol::{
    Card, Column, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult, NewParams, NewResult,
    RpcError, RpcRequest, StatusSelfResult,
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
```

Then replace the `dispatch` signature and bodies so transitions fire:

```rust
pub async fn dispatch(
    req: RpcRequest,
    state: DaemonState,
    caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => handle_ls(state.db).await,
        "new" => {
            let params = parse_params::<NewParams>(req.params)?;
            handle_new(state.db, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            handle_mv(state, params).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let params = parse_params::<MvSelfParams>(req.params)?;
            handle_mv_self(state, card_id, params).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            handle_status_self(state.db, card_id).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}
```

Update `handle_mv` to fetch the full card, **precheck** that the household is configured if the new column requires a container, then perform the move and dispatch the transition. The precheck means a missing household is rejected *before* mutating the column — the spec calls for "the column move is rolled back" semantics.

```rust
async fn handle_mv(state: DaemonState, params: MvParams) -> std::result::Result<Value, RpcError> {
    let id = params.id.clone();
    let new_column = params.column;
    let now = now_secs();

    // Read the existing card so we know the old column AND the household name.
    let db = state.db.clone();
    let id_for_query = id.clone();
    let existing = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::get_card(&conn, &id_for_query)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {}", params.id),
    })?;
    let old_column = existing.column;

    // Precheck: a move that requires a container must have a configured household.
    // Reject BEFORE the column is mutated so the user can fix their config and retry.
    if new_column == Column::Implementing && !state.households.contains_key(&existing.household) {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }

    let db = state.db.clone();
    let id_for_update = id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id_for_update, new_column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {}", params.id),
    })?;

    let fx = LiveEffects::new(state);
    transitions::on_column_change(old_column, new_column, &card, &fx)
        .await
        .map_err(to_internal)?;
    serde_json::to_value(MvResult { card }).map_err(to_internal)
}
```

Do the analogous change in `handle_mv_self`:

```rust
async fn handle_mv_self(
    state: DaemonState,
    card_id: String,
    params: MvSelfParams,
) -> std::result::Result<Value, RpcError> {
    let new_column = params.column;
    let now = now_secs();

    let db = state.db.clone();
    let id_for_query = card_id.clone();
    let existing = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::get_card(&conn, &id_for_query)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;
    let old_column = existing.column;

    if new_column == Column::Implementing && !state.households.contains_key(&existing.household) {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }

    let db = state.db.clone();
    let id_for_update = card_id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id_for_update, new_column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;

    let fx = LiveEffects::new(state);
    transitions::on_column_change(old_column, new_column, &card, &fx)
        .await
        .map_err(to_internal)?;
    serde_json::to_value(MvSelfResult { card }).map_err(to_internal)
}
```

And keep `handle_ls`, `handle_new`, `handle_status_self` taking `DbHandle` as before (they don't trigger transitions).

- [ ] **Step 3: Update existing handler tests to provide a `DaemonState`**

In the `#[cfg(test)] mod tests` block of `handlers.rs`, replace `fresh_db` with a `fresh_state()` helper, and update every `dispatch(..)` call:

```rust
    use crate::container::{ContainerOps, ContainerOpsHandle};
    use crate::effects::DaemonState;
    use crate::household::HouseholdMap;
    use crate::listeners::{Dispatcher, ListenerRegistry};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// No-op container backend that pretends nothing exists.
    struct NoopOps;
    #[async_trait]
    impl ContainerOps for NoopOps {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> { Ok(()) }
        async fn teardown(&self, _name: &str) -> Result<()> { Ok(()) }
        async fn inspect_status(&self, _name: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    /// Dispatcher stub; transitions don't need to call back here in tests.
    struct NoopDispatcher;
    impl Dispatcher for NoopDispatcher {
        fn dispatch(
            &self,
            _r: RpcRequest,
            _d: DbHandle,
            _c: CallerContext,
        ) -> Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<serde_json::Value, RpcError>,
                    > + Send,
            >,
        > {
            Box::pin(async { Ok(serde_json::Value::Null) })
        }
    }

    fn fresh_state() -> DaemonState {
        let conn = db::open(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            households: Arc::new(HashMap::new()),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }
```

Replace every `let db = fresh_db();` with `let state = fresh_state();` and every `dispatch(req, db, caller)` with `dispatch(req, state.clone(), caller)`. The existing assertions stay.

Also add a new test that verifies the household precheck rolls the column back (i.e., doesn't mutate the card when household isn't configured):

```rust
    #[tokio::test]
    async fn mv_to_implementing_without_household_leaves_column_unchanged() {
        let state = fresh_state();

        // Create a card under household "h" (which is NOT in fresh_state's
        // empty households map).
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        // Try to move to Implementing. Should fail because the household
        // isn't loaded.
        let err = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("household 'h' not configured"));

        // The card must still be in Backlog — the column move was rolled back.
        let ls_value = dispatch(
            RpcRequest {
                id: 3,
                method: "ls".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let ls: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls.cards[0].column, Column::Backlog);
    }
```

- [ ] **Step 4: Register `effects` module and wire the dispatcher in `main.rs`**

The listener registry needs a `Dispatcher`; the live dispatcher needs the `DaemonState`; and `DaemonState` contains the listener registry. We break this cycle with an `Arc<MainDispatcher>` that holds the state in a `OnceLock`: build the holder first, build the listeners with it, build the state, then initialise the lock.

In `crates/reeved/src/main.rs`, update the module list:

```rust
mod container;
mod db;
mod effects;
mod handlers;
mod household;
mod lifecycle;
mod listeners;
mod server;
mod transitions;
```

Add this struct definition at the bottom of `main.rs`:

```rust
struct MainDispatcher {
    state: std::sync::OnceLock<effects::DaemonState>,
}

impl listeners::Dispatcher for MainDispatcher {
    fn dispatch(
        &self,
        req: reeve_protocol::RpcRequest,
        _db: server::DbHandle,
        caller: server::CallerContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<serde_json::Value, reeve_protocol::RpcError>,
                > + Send,
        >,
    > {
        let state = self.state.get().cloned();
        Box::pin(async move {
            match state {
                Some(state) => handlers::dispatch(req, state, caller).await,
                None => Err(reeve_protocol::RpcError {
                    code: reeve_protocol::RpcError::INTERNAL,
                    message: "dispatcher not initialised".into(),
                }),
            }
        })
    }
}
```

Add these imports near the top of `main.rs` (alongside the existing `use` statements):

```rust
use reeve_protocol::RpcRequest;
use server::{CallerContext, DbHandle};
```

Then replace the existing `let result = server::run(&socket_path, db, ...).await;` block in `main()` with:

```rust
    let dispatcher_holder: Arc<MainDispatcher> = Arc::new(MainDispatcher {
        state: std::sync::OnceLock::new(),
    });

    let households = household::load_all(&lifecycle::households_dir()?)?;
    let containers: container::ContainerOpsHandle = Arc::new(container::DockerOps);
    let listeners = listeners::ListenerRegistry::new(db.clone(), dispatcher_holder.clone());
    let claude_dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".claude"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent"));
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();

    let state = effects::DaemonState {
        db: db.clone(),
        containers,
        listeners: listeners.clone(),
        households,
        claude_dir,
        api_key,
    };
    dispatcher_holder
        .state
        .set(state.clone())
        .map_err(|_| anyhow::anyhow!("dispatcher state already initialised"))?;

    let state_for_dispatch = state.clone();
    let dispatch = move |req: RpcRequest, _db: DbHandle, caller: CallerContext| {
        let state = state_for_dispatch.clone();
        async move { handlers::dispatch(req, state, caller).await }
    };

    let result = server::run(
        &socket_path,
        db,
        server::CallerContext::host(),
        shutdown_rx,
        dispatch,
    )
    .await;

    listeners.stop_all().await;
```

- [ ] **Step 5: Run the workspace tests**

Run: `cargo test --workspace`
Expected: every existing test passes plus the new precheck behaviour. Containers don't actually start (`NoopOps` is a no-op), but the dispatch wiring compiles and unit tests round-trip.

- [ ] **Step 6: Commit**

```bash
git add crates/reeved/src/effects.rs crates/reeved/src/handlers.rs crates/reeved/src/main.rs
git commit -m "feat(m2): wire LiveEffects into handle_mv/mv-self with household precheck"
```

---

## Task 11: Daemon startup reconcile of surviving containers

**Files:**
- Modify: `crates/reeved/src/container.rs`
- Modify: `crates/reeved/src/main.rs`

- [ ] **Step 1: Write the failing test for the reconcile decision function**

Append to `crates/reeved/src/container.rs` (inside the existing `#[cfg(test)] mod docker_tests` or a new `mod reconcile_tests`):

```rust
#[cfg(test)]
mod reconcile_tests {
    use super::*;
    use crate::db::ContainerRow;
    use reeve_protocol::ContainerStatus;

    fn row(card_id: &str, status: ContainerStatus) -> ContainerRow {
        ContainerRow {
            card_id: card_id.into(),
            container_name: container_name(card_id),
            status,
            started_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn reconcile_running_with_live_container_is_adopt() {
        let decision = reconcile_one(&row("c1", ContainerStatus::Running), Some("running"));
        assert_eq!(decision, ReconcileDecision::Adopt);
    }

    #[test]
    fn reconcile_running_with_no_container_is_orphan() {
        let decision = reconcile_one(&row("c1", ContainerStatus::Running), None);
        assert_eq!(decision, ReconcileDecision::Orphan);
    }

    #[test]
    fn reconcile_running_with_exited_container_is_orphan() {
        let decision = reconcile_one(&row("c1", ContainerStatus::Running), Some("exited"));
        assert_eq!(decision, ReconcileDecision::Orphan);
    }

    #[test]
    fn reconcile_spawning_with_no_container_is_orphan() {
        // Daemon died mid-spawn. We don't know whether docker started
        // it or not; safest is to mark it failed and let the user retry.
        let decision = reconcile_one(&row("c1", ContainerStatus::Spawning), None);
        assert_eq!(decision, ReconcileDecision::Orphan);
    }
}
```

- [ ] **Step 2: Add the reconcile decision function**

Append to `crates/reeved/src/container.rs`:

```rust
use crate::db::ContainerRow;
use crate::effects::DaemonState;
use crate::listeners::ListenerRegistry;
use reeve_protocol::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileDecision {
    Adopt,
    Orphan,
}

/// Decide what to do with a `Running`/`Spawning` row given the actual
/// docker state. Pure for testability.
pub fn reconcile_one(row: &ContainerRow, docker_state: Option<&str>) -> ReconcileDecision {
    match docker_state {
        Some("running") => ReconcileDecision::Adopt,
        _ => ReconcileDecision::Orphan,
    }
}

/// Run the reconcile pass at daemon startup. For each row with status
/// `Running` or `Spawning` in the DB:
///
/// - container is `running` → adopt (re-bind per-card listener)
/// - otherwise → mark `Failed` in DB and unlink stale socket file
///
/// Containers that survived daemon shutdown keep running; their socket
/// files were left on disk and are unlinked here (the listener-bind in
/// `spawn_record_only` would do it anyway, but doing it explicitly is
/// clearer in logs).
pub async fn reconcile_on_startup(state: &DaemonState) -> Result<()> {
    use reeve_protocol::ContainerStatus;
    let db = state.db.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<ContainerRow>> {
        let conn = db.blocking_lock();
        let mut rows = crate::db::list_containers_by_status(&conn, ContainerStatus::Running)?;
        rows.extend(crate::db::list_containers_by_status(
            &conn,
            ContainerStatus::Spawning,
        )?);
        Ok(rows)
    })
    .await??;

    for row in rows {
        let docker_state = state
            .containers
            .inspect_status(&row.container_name)
            .await
            .ok()
            .flatten();
        match reconcile_one(&row, docker_state.as_deref()) {
            ReconcileDecision::Adopt => {
                let socket = paths::card_socket_path(&row.card_id);
                ListenerRegistry::unlink_stale(&socket);
                state.listeners.spawn_record_only(&row.card_id, socket).await;
                tracing::info!(card = %row.card_id, "adopted surviving container");
            }
            ReconcileDecision::Orphan => {
                let socket = paths::card_socket_path(&row.card_id);
                ListenerRegistry::unlink_stale(&socket);
                let db = state.db.clone();
                let id = row.card_id.clone();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let conn = db.blocking_lock();
                    crate::db::update_container_status(
                        &conn,
                        &id,
                        ContainerStatus::Failed,
                        now,
                    )
                })
                .await??;
                tracing::warn!(card = %row.card_id, "orphaned container row → Failed");
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Wire reconcile into `main.rs`**

In `crates/reeved/src/main.rs`, right after the `state` is built and `dispatcher_holder.state.set(...)` is called, before constructing the dispatch closure, add:

```rust
    container::reconcile_on_startup(&state).await?;
```

- [ ] **Step 4: Run; expect pass**

Run: `cargo test -p reeved container::reconcile_tests`
Expected: four tests pass.

Run: `cargo test --workspace`
Expected: nothing regresses.

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/container.rs crates/reeved/src/main.rs
git commit -m "feat(m2): reconcile surviving containers on daemon startup"
```

---

## Task 12: CLI `mv-self` and `status-self` subcommands

**Files:**
- Modify: `crates/reeve/src/cli.rs`

- [ ] **Step 1: Add new variants to the `Command` enum**

In `crates/reeve/src/cli.rs`, extend `Command`:

```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    /// List cards.
    Ls,
    /// Create a new card in the Backlog column.
    New {
        #[arg(long)]
        household: String,
        #[arg(long)]
        brief: String,
        #[arg(long, default_value_t = false)]
        needs_heavy: bool,
    },
    /// Move a card to a different column.
    Mv {
        id: String,
        #[arg(value_parser = parse_column)]
        column: Column,
    },
    /// (in-container only) Move the calling card to a different column.
    MvSelf {
        #[arg(value_parser = parse_column)]
        column: Column,
    },
    /// (in-container only) Print the calling card and its container status.
    StatusSelf,
    /// Daemon management subcommands.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}
```

- [ ] **Step 2: Update `paths::detect_socket()` call sites**

In the same file, replace each `let socket = paths::socket_path();` inside `ls()`, `new()`, `mv()` with:

```rust
    let socket = paths::detect_socket();
```

This makes the existing `ls/new/mv` work transparently from inside a container too.

- [ ] **Step 3: Implement the new subcommands**

After the existing `mv` function, add:

```rust
async fn mv_self(column: Column) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(
        &socket,
        "mv-self",
        serde_json::json!({"column": column}),
    )
    .await
    .context("calling mv-self")?;
    match result {
        Ok(value) => {
            let result: reeve_protocol::MvSelfResult = serde_json::from_value(value)?;
            println!("{} -> {}", result.card.id, result.card.column.label());
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn status_self() -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "status-self", serde_json::json!({}))
        .await
        .context("calling status-self")?;
    match result {
        Ok(value) => {
            let r: reeve_protocol::StatusSelfResult = serde_json::from_value(value)?;
            println!(
                "{} ({})",
                r.card.id,
                r.card.column.label(),
            );
            match r.container_status {
                Some(s) => println!("container: {:?}", s),
                None => println!("container: (none)"),
            }
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}
```

- [ ] **Step 4: Route the new variants in `run`**

Replace the `pub async fn run(cmd: Command)` body:

```rust
pub async fn run(cmd: Command) -> Result<()> {
    match cmd {
        Command::Ls => ls().await,
        Command::New { household, brief, needs_heavy } => new(household, brief, needs_heavy).await,
        Command::Mv { id, column } => mv(id, column).await,
        Command::MvSelf { column } => mv_self(column).await,
        Command::StatusSelf => status_self().await,
        Command::Daemon(_) => unreachable!("Daemon commands are handled in main"),
    }
}
```

- [ ] **Step 5: Verify the build**

Run: `cargo build --workspace`
Expected: clean build.

Run: `cargo test --workspace`
Expected: nothing regresses.

- [ ] **Step 6: Commit**

```bash
git add crates/reeve/src/cli.rs
git commit -m "feat(m2): reeve mv-self and status-self subcommands; auto-detect in-container socket"
```

---

## Task 13: Docker-gated end-to-end test

**Files:**
- Modify: `tests/e2e.rs`
- Modify: `tests/Cargo.toml` (if needed)

- [ ] **Step 1: Add helper utilities to `tests/e2e.rs`**

Edit `tests/e2e.rs`. After the existing `isolated_env` function, add:

```rust
fn is_docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn docker_inspect_status(name: &str) -> Option<String> {
    let out = std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.State.Status}}", name])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Forcibly remove a container; tolerates missing.
fn docker_rm_force(name: &str) {
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Removes `reeve-<card_id>` on drop, even if the test panics.
struct ContainerGuard(String);
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        docker_rm_force(&format!("reeve-{}", self.0));
    }
}

fn isolated_env_with_config(temp: &std::path::Path) -> Vec<(String, String)> {
    vec![
        ("XDG_RUNTIME_DIR".into(), temp.to_string_lossy().into_owned()),
        ("XDG_DATA_HOME".into(), temp.join("data").to_string_lossy().into_owned()),
        ("XDG_CONFIG_HOME".into(), temp.join("config").to_string_lossy().into_owned()),
    ]
}

fn write_test_household(temp: &std::path::Path) {
    let dir = temp.join("config/reeve/households");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("test.toml"),
        "image = \"alpine:latest\"\ncmd = [\"sleep\", \"infinity\"]\n",
    )
    .unwrap();
}

fn wait_for_container(name: &str, expect_running: bool, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        match docker_inspect_status(name) {
            Some(s) if s == "running" && expect_running => return true,
            None if !expect_running => return true,
            _ => {}
        }
        sleep(Duration::from_millis(100));
    }
    false
}

fn parse_card_id_from_new_output(stdout: &str) -> String {
    // "created <uuid> (Backlog)"
    stdout
        .strip_prefix("created ")
        .and_then(|s| s.split_whitespace().next())
        .map(|s| s.to_string())
        .unwrap_or_else(|| panic!("could not parse card id from: {stdout}"))
}
```

- [ ] **Step 2: Write the happy-path e2e test**

Append to `tests/e2e.rs`:

```rust
#[test]
fn m2_container_lifecycle_happy_path() {
    if !is_docker_available() {
        eprintln!("skipping m2_container_lifecycle_happy_path: docker unavailable");
        return;
    }
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());
    write_test_household(temp.path());

    let _daemon = DaemonGuard(
        cmd("reeved", &env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reeved"),
    );

    // Wait for socket.
    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists());

    // new card
    let out = cmd("reeve", &env)
        .args(["new", "--household", "test", "--brief", "smoke"])
        .output()
        .expect("reeve new");
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));
    let card_id = parse_card_id_from_new_output(&String::from_utf8(out.stdout).unwrap());
    let _guard = ContainerGuard(card_id.clone());

    // mv → implementing
    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("reeve mv");
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));

    let container = format!("reeve-{card_id}");
    assert!(
        wait_for_container(&container, true, Duration::from_secs(10)),
        "container {container} did not reach running within 10s",
    );

    // Per-card socket should now exist.
    let card_socket = temp
        .path()
        .join("reeve/cards")
        .join(format!("{card_id}.sock"));
    assert!(card_socket.exists(), "per-card socket missing: {}", card_socket.display());

    // Copy the reeve binary into the container so the in-container CLI exists.
    let reeve_bin = target_dir().join("reeve");
    let status = Command::new("docker")
        .args([
            "cp",
            reeve_bin.to_str().unwrap(),
            &format!("{container}:/usr/local/bin/reeve"),
        ])
        .status()
        .expect("docker cp reeve");
    assert!(status.success(), "docker cp failed");

    // From inside the container: mv-self review.
    let out = Command::new("docker")
        .args([
            "exec",
            &container,
            "/usr/local/bin/reeve",
            "mv-self",
            "review",
        ])
        .output()
        .expect("docker exec mv-self");
    assert!(
        out.status.success(),
        "docker exec mv-self failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Container should still be running (Review keeps the container warm).
    assert_eq!(docker_inspect_status(&container).as_deref(), Some("running"));

    // mv → done tears it down.
    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "done"])
        .output()
        .expect("reeve mv done");
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));

    assert!(
        wait_for_container(&container, false, Duration::from_secs(10)),
        "container {container} still present after Done",
    );
    assert!(!card_socket.exists(), "per-card socket lingered after teardown");
}
```

- [ ] **Step 3: Write the reconcile e2e test**

Append to `tests/e2e.rs`:

```rust
#[test]
fn m2_reconcile_after_daemon_sigkill() {
    if !is_docker_available() {
        eprintln!("skipping m2_reconcile_after_daemon_sigkill: docker unavailable");
        return;
    }
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());
    write_test_household(temp.path());

    // First daemon session: create card, move to Implementing.
    let mut daemon = cmd("reeved", &env)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reeved");

    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists());

    let out = cmd("reeve", &env)
        .args(["new", "--household", "test", "--brief", "reconcile"])
        .output()
        .expect("reeve new");
    let card_id = parse_card_id_from_new_output(&String::from_utf8(out.stdout).unwrap());
    let _guard = ContainerGuard(card_id.clone());

    cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .status()
        .expect("reeve mv");

    let container = format!("reeve-{card_id}");
    assert!(wait_for_container(&container, true, Duration::from_secs(10)));

    let reeve_bin = target_dir().join("reeve");
    Command::new("docker")
        .args([
            "cp",
            reeve_bin.to_str().unwrap(),
            &format!("{container}:/usr/local/bin/reeve"),
        ])
        .status()
        .expect("docker cp");

    // SIGKILL bypasses graceful shutdown. Container should survive.
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(daemon.id() as i32),
        nix::sys::signal::SIGKILL,
    )
    .unwrap();
    let _ = daemon.wait();
    assert_eq!(docker_inspect_status(&container).as_deref(), Some("running"));

    // Restart daemon — reconcile should re-bind the per-card socket.
    let _daemon2 = DaemonGuard(
        cmd("reeved", &env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reeved (2)"),
    );

    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }

    let card_socket = temp
        .path()
        .join("reeve/cards")
        .join(format!("{card_id}.sock"));
    let mut waited = Duration::ZERO;
    while !card_socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(card_socket.exists(), "per-card socket not re-bound after restart");

    // In-container mv-self should now work again.
    let out = Command::new("docker")
        .args([
            "exec",
            &container,
            "/usr/local/bin/reeve",
            "mv-self",
            "review",
        ])
        .output()
        .expect("docker exec");
    assert!(
        out.status.success(),
        "in-container mv-self after restart: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
```

- [ ] **Step 4: Run all e2e tests**

Run: `cargo test --test e2e`
Expected: M1 test still passes; both M2 tests pass (when docker is available) or print a skip message (when it's not).

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every unit, integration, and e2e test passes.

- [ ] **Step 6: Commit**

```bash
git add tests/e2e.rs
git commit -m "test(m2): docker-gated end-to-end for container lifecycle and reconcile"
```

---

## Definition of done

- [ ] All 13 tasks above completed and committed.
- [ ] `cargo test --workspace` passes (M1 + M2 unit + integration + e2e).
- [ ] Manual smoke (with `docker` installed):
  1. Write `~/.config/reeve/households/test.toml` with `image = "alpine:latest"`, `cmd = ["sleep", "infinity"]`.
  2. `reeve daemon start`
  3. `reeve new --household test --brief "smoke"` → note `<card-id>`
  4. `reeve mv <card-id> implementing` → `docker ps | grep reeve-` shows the container.
  5. `docker cp $(which reeve) reeve-<card-id>:/usr/local/bin/reeve`
  6. `docker exec reeve-<card-id> /usr/local/bin/reeve mv-self review` → succeeds.
  7. `reeve mv <card-id> done` → `docker ps -a` no longer shows the container.
  8. `reeve daemon stop`.
- [ ] Manual reconcile smoke:
  1. After step 4 above, `kill -9 $(pidof reeved)`.
  2. Confirm `docker ps` still shows `reeve-<card-id>`.
  3. `reeve daemon start`.
  4. `docker exec reeve-<card-id> /usr/local/bin/reeve mv-self review` still works.
- [ ] Branch `feat/m2-container-lifecycle` rebased on `main` and ready to fast-forward merge.
