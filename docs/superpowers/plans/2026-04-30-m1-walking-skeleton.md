# Reeve M1: Walking Skeleton — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the walking skeleton of Reeve: a Rust workspace with a `reeved` daemon, a `reeve` user binary providing CLI subcommands and a read-only Ratatui kanban viewer, communicating via a JSON-line protocol over a Unix socket, with SQLite-backed card storage. No container, agent, git, or service-scheduler functionality yet — pure metadata management.

**Architecture:** Cargo workspace with three crates. `reeve-protocol` defines `Card`, `Column`, and RPC request/response types shared by both binaries. `reeved` owns a SQLite database, accepts connections on a Unix socket at `$XDG_RUNTIME_DIR/reeved.sock`, and dispatches incoming JSON-line requests to handlers. `reeve` is the user-facing binary: with no args it launches the read-only TUI viewer; with a subcommand it speaks the same JSON-line protocol as a client. The walking skeleton is intentionally minimal — TUI is read-only; mutations happen via CLI.

**Tech Stack:** Rust 2024 edition, tokio for async runtime, rusqlite for SQLite, serde + serde_json for serialization, clap for CLI parsing, ratatui + crossterm for TUI, nix for Unix-specific operations, tracing for logging, anyhow + thiserror for errors.

---

## File structure (created in this milestone)

```
reeve/
├── Cargo.toml                          # workspace manifest
├── README.md                           # quickstart
├── .gitignore
├── crates/
│   ├── reeve-protocol/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                  # Card, Column, RpcRequest, RpcResponse, method types
│   ├── reeved/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                 # entry point, signal handling
│   │       ├── server.rs               # Unix socket accept loop
│   │       ├── handlers.rs             # RPC method dispatch
│   │       ├── db.rs                   # SQLite operations
│   │       └── lifecycle.rs            # socket path, PID file, db path
│   └── reeve/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                 # CLI vs TUI dispatch, clap setup
│           ├── client.rs               # IPC client
│           ├── cli.rs                  # CLI subcommand handlers
│           └── tui.rs                  # ratatui kanban viewer
└── tests/
    └── e2e.rs                          # workspace-level integration smoke test
```

Tests are colocated with code via `#[cfg(test)] mod tests { ... }` blocks. The workspace-level `tests/e2e.rs` runs end-to-end tests against built binaries.

---

## Task 1: Scaffold Cargo workspace with three crates

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `README.md`
- Create: `crates/reeve-protocol/Cargo.toml`
- Create: `crates/reeve-protocol/src/lib.rs`
- Create: `crates/reeved/Cargo.toml`
- Create: `crates/reeved/src/main.rs`
- Create: `crates/reeve/Cargo.toml`
- Create: `crates/reeve/src/main.rs`

- [ ] **Step 1: Create the workspace `Cargo.toml`**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/reeve-protocol", "crates/reeved", "crates/reeve"]

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "MIT"
authors = ["Daniel Ericson <daniel@mindfulstack.se>"]

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.42", features = ["full"] }
anyhow = "1.0"
thiserror = "2.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 2: Create `.gitignore`**

`.gitignore`:
```
/target
**/*.rs.bk
.cargo/
*.pdb
```

(Cargo.lock is committed for binary applications.)

- [ ] **Step 3: Create `README.md`**

`README.md`:
````markdown
# Reeve

A TUI orchestrator for parallel Claude Code sessions in a poly-repo setup.

See `docs/superpowers/specs/2026-04-29-reeve-design.md` for the design.

## Building

```sh
cargo build --release
```

## M1 status

Walking skeleton: daemon + CLI + read-only TUI for card metadata.
No containers, agents, or git plumbing yet.
````

- [ ] **Step 4: Create `crates/reeve-protocol/Cargo.toml`**

`crates/reeve-protocol/Cargo.toml`:
```toml
[package]
name = "reeve-protocol"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 5: Create placeholder `crates/reeve-protocol/src/lib.rs`**

`crates/reeve-protocol/src/lib.rs`:
```rust
//! Shared protocol types for Reeve daemon and clients.
```

- [ ] **Step 6: Create `crates/reeved/Cargo.toml`**

`crates/reeved/Cargo.toml`:
```toml
[package]
name = "reeved"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[[bin]]
name = "reeved"
path = "src/main.rs"

[dependencies]
reeve-protocol = { path = "../reeve-protocol" }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
rusqlite = { version = "0.32", features = ["bundled"] }
nix = { version = "0.29", features = ["signal", "process"] }
uuid = { version = "1.10", features = ["v4"] }
```

- [ ] **Step 7: Create placeholder `crates/reeved/src/main.rs`**

`crates/reeved/src/main.rs`:
```rust
fn main() {
    println!("reeved placeholder");
}
```

- [ ] **Step 8: Create `crates/reeve/Cargo.toml`**

`crates/reeve/Cargo.toml`:
```toml
[package]
name = "reeve"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[[bin]]
name = "reeve"
path = "src/main.rs"

[dependencies]
reeve-protocol = { path = "../reeve-protocol" }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
clap = { version = "4.5", features = ["derive"] }
ratatui = "0.29"
crossterm = "0.28"
nix = { version = "0.29", features = ["signal", "process"] }
```

- [ ] **Step 9: Create placeholder `crates/reeve/src/main.rs`**

`crates/reeve/src/main.rs`:
```rust
fn main() {
    println!("reeve placeholder");
}
```

- [ ] **Step 10: Verify the workspace builds**

Run: `cargo build`
Expected: All three crates compile. Cargo.lock is generated.

- [ ] **Step 11: Commit**

```sh
git add Cargo.toml Cargo.lock .gitignore README.md crates/
git commit -m "feat: scaffold Cargo workspace with reeve-protocol, reeved, reeve crates"
```

---

## Task 2: Define `Column` and `Card` domain types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Write failing tests for Column and Card serialization**

Add to `crates/reeve-protocol/src/lib.rs`:
```rust
//! Shared protocol types for Reeve daemon and clients.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_serializes_to_lowercase_string() {
        let json = serde_json::to_string(&Column::Implementing).unwrap();
        assert_eq!(json, "\"implementing\"");
    }

    #[test]
    fn column_deserializes_from_lowercase_string() {
        let col: Column = serde_json::from_str("\"review\"").unwrap();
        assert_eq!(col, Column::Review);
    }

    #[test]
    fn card_round_trips_through_json() {
        let card = Card {
            id: "abc".into(),
            household: "myarchy".into(),
            brief: "test brief".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let back: Card = serde_json::from_str(&json).unwrap();
        assert_eq!(back, card);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p reeve-protocol`
Expected: FAIL — `Column` and `Card` are not defined.

- [ ] **Step 3: Define `Column` and `Card`**

Replace `crates/reeve-protocol/src/lib.rs` with:
```rust
//! Shared protocol types for Reeve daemon and clients.

use serde::{Deserialize, Serialize};

/// The kanban columns a card may occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Backlog,
    Planning,
    Implementing,
    Testing,
    Review,
    Done,
}

impl Column {
    /// All columns in canonical left-to-right kanban order.
    pub const ALL: [Column; 6] = [
        Column::Backlog,
        Column::Planning,
        Column::Implementing,
        Column::Testing,
        Column::Review,
        Column::Done,
    ];

    /// Display name for headings.
    pub fn label(&self) -> &'static str {
        match self {
            Column::Backlog => "Backlog",
            Column::Planning => "Planning",
            Column::Implementing => "Implementing",
            Column::Testing => "Testing",
            Column::Review => "Review",
            Column::Done => "Done",
        }
    }
}

/// A card in the kanban — one feature's worth of work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub household: String,
    pub brief: String,
    pub column: Column,
    pub created_at: i64,
    pub updated_at: i64,
    pub needs_heavy: bool,
    pub parent_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_serializes_to_lowercase_string() {
        let json = serde_json::to_string(&Column::Implementing).unwrap();
        assert_eq!(json, "\"implementing\"");
    }

    #[test]
    fn column_deserializes_from_lowercase_string() {
        let col: Column = serde_json::from_str("\"review\"").unwrap();
        assert_eq!(col, Column::Review);
    }

    #[test]
    fn card_round_trips_through_json() {
        let card = Card {
            id: "abc".into(),
            household: "myarchy".into(),
            brief: "test brief".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let back: Card = serde_json::from_str(&json).unwrap();
        assert_eq!(back, card);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p reeve-protocol`
Expected: PASS — all three tests green.

- [ ] **Step 5: Commit**

```sh
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(protocol): add Column and Card domain types"
```

---

## Task 3: Define RPC request/response types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Write failing tests for RPC type round-trips and method-specific params**

Append to `crates/reeve-protocol/src/lib.rs` `tests` module:
```rust
    #[test]
    fn rpc_request_serializes_with_method_and_params() {
        let req = RpcRequest {
            id: 7,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"id\":7"));
        assert!(json.contains("\"method\":\"ls\""));
    }

    #[test]
    fn rpc_response_with_result_omits_error_field() {
        let resp = RpcResponse {
            id: 1,
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn new_params_round_trip() {
        let p = NewParams {
            household: "myarchy".into(),
            brief: "do the thing".into(),
            needs_heavy: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: NewParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn mv_params_round_trip() {
        let p = MvParams {
            id: "abc".into(),
            column: Column::Implementing,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: MvParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p reeve-protocol`
Expected: FAIL — `RpcRequest`, `RpcResponse`, `NewParams`, `MvParams` are not defined.

- [ ] **Step 3: Define the RPC types**

Append to `crates/reeve-protocol/src/lib.rs` (above the `tests` module):
```rust
/// A JSON-line RPC request from client → daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

/// A JSON-line RPC response from daemon → client.
///
/// Exactly one of `result` or `error` is `Some` for any non-malformed response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcResponse {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("rpc error {code}: {message}")]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;
    pub const NOT_FOUND: i32 = 404;
}

/// Parameters for the `ls` method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsParams {}

/// Result for the `ls` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsResult {
    pub cards: Vec<Card>,
}

/// Parameters for the `new` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewParams {
    pub household: String,
    pub brief: String,
    pub needs_heavy: bool,
}

/// Result for the `new` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewResult {
    pub card: Card,
}

/// Parameters for the `mv` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvParams {
    pub id: String,
    pub column: Column,
}

/// Result for the `mv` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvResult {
    pub card: Card,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p reeve-protocol`
Expected: PASS — all tests green.

- [ ] **Step 5: Commit**

```sh
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(protocol): add RPC request/response and method-specific types"
```

---

## Task 4: SQLite schema and CRUD operations in `reeved::db`

**Files:**
- Create: `crates/reeved/src/db.rs`
- Modify: `crates/reeved/src/main.rs` (add `mod db`)

- [ ] **Step 1: Write failing tests for db CRUD against an in-memory connection**

Create `crates/reeved/src/db.rs`:
```rust
//! SQLite-backed card storage.

use anyhow::{Context, Result};
use reeve_protocol::{Card, Column};
use rusqlite::{Connection, OptionalExtension, params};

/// Open a SQLite connection at `path` and run any pending migrations.
/// Pass `":memory:"` for an in-memory database (used in tests).
pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("open db at {path}"))?;
    migrate(&conn)?;
    Ok(conn)
}

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
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);",
    )?;
    Ok(())
}

pub fn insert_card(conn: &Connection, card: &Card) -> Result<()> {
    conn.execute(
        "INSERT INTO cards (id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            card.id,
            card.household,
            card.brief,
            column_as_str(card.column),
            card.created_at,
            card.updated_at,
            card.needs_heavy as i64,
            card.parent_id,
        ],
    )?;
    Ok(())
}

pub fn list_cards(conn: &Connection) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
         FROM cards ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], row_to_card)?;
    let cards: Result<Vec<_>, _> = rows.collect();
    Ok(cards?)
}

pub fn get_card(conn: &Connection, id: &str) -> Result<Option<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
         FROM cards WHERE id = ?1",
    )?;
    let card = stmt.query_row(params![id], row_to_card).optional()?;
    Ok(card)
}

pub fn update_card_column(conn: &Connection, id: &str, column: Column, now: i64) -> Result<Option<Card>> {
    let updated = conn.execute(
        "UPDATE cards SET column_ = ?1, updated_at = ?2 WHERE id = ?3",
        params![column_as_str(column), now, id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    get_card(conn, id)
}

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let column_str: String = row.get(3)?;
    let column = parse_column(&column_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
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

fn column_as_str(column: Column) -> &'static str {
    match column {
        Column::Backlog => "backlog",
        Column::Planning => "planning",
        Column::Implementing => "implementing",
        Column::Testing => "testing",
        Column::Review => "review",
        Column::Done => "done",
    }
}

fn parse_column(s: &str) -> Result<Column, ParseColumnError> {
    match s {
        "backlog" => Ok(Column::Backlog),
        "planning" => Ok(Column::Planning),
        "implementing" => Ok(Column::Implementing),
        "testing" => Ok(Column::Testing),
        "review" => Ok(Column::Review),
        "done" => Ok(Column::Done),
        _ => Err(ParseColumnError(s.to_owned())),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid column value: {0}")]
struct ParseColumnError(String);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card(id: &str) -> Card {
        Card {
            id: id.into(),
            household: "myarchy".into(),
            brief: "test".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        }
    }

    #[test]
    fn open_initialises_schema() {
        let conn = open(":memory:").unwrap();
        // Table exists if list_cards returns empty.
        let cards = list_cards(&conn).unwrap();
        assert!(cards.is_empty());
    }

    #[test]
    fn insert_then_list_returns_card() {
        let conn = open(":memory:").unwrap();
        let card = sample_card("a");
        insert_card(&conn, &card).unwrap();
        let cards = list_cards(&conn).unwrap();
        assert_eq!(cards, vec![card]);
    }

    #[test]
    fn update_card_column_changes_column_and_returns_updated() {
        let conn = open(":memory:").unwrap();
        let mut card = sample_card("a");
        insert_card(&conn, &card).unwrap();
        let updated = update_card_column(&conn, "a", Column::Implementing, 2000)
            .unwrap()
            .unwrap();
        card.column = Column::Implementing;
        card.updated_at = 2000;
        assert_eq!(updated, card);
    }

    #[test]
    fn update_missing_card_returns_none() {
        let conn = open(":memory:").unwrap();
        let result = update_card_column(&conn, "nonexistent", Column::Done, 2000).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_card_returns_none_for_missing() {
        let conn = open(":memory:").unwrap();
        assert!(get_card(&conn, "missing").unwrap().is_none());
    }
}
```

- [ ] **Step 2: Wire the new module into the daemon binary**

Replace `crates/reeved/src/main.rs` with:
```rust
mod db;

fn main() {
    println!("reeved placeholder");
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p reeved`
Expected: PASS — all five `db::tests` pass.

- [ ] **Step 4: Commit**

```sh
git add crates/reeved/src/db.rs crates/reeved/src/main.rs
git commit -m "feat(reeved): SQLite-backed card storage with CRUD"
```

---

## Task 5: Lifecycle module (paths and PID file)

**Files:**
- Create: `crates/reeved/src/lifecycle.rs`
- Modify: `crates/reeved/src/main.rs` (add `mod lifecycle`)

- [ ] **Step 1: Write failing tests for path resolution and PID file management**

Create `crates/reeved/src/lifecycle.rs`:
```rust
//! Daemon lifecycle helpers: path resolution, PID file management.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Returns the directory the daemon uses for runtime files.
/// Prefers `$XDG_RUNTIME_DIR`; falls back to `/tmp` with the user's UID embedded.
pub fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let uid = nix::unistd::Uid::current().as_raw();
    PathBuf::from(format!("/tmp/reeve-{uid}"))
}

/// Returns the directory for persistent daemon state (DB, etc.).
/// Prefers `$XDG_DATA_HOME/reeve`; falls back to `$HOME/.local/share/reeve`.
pub fn data_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir).join("reeve"));
        }
    }
    let home = std::env::var("HOME").context("$HOME not set")?;
    Ok(PathBuf::from(home).join(".local/share/reeve"))
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("reeved.sock")
}

pub fn pid_path() -> PathBuf {
    runtime_dir().join("reeved.pid")
}

pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("reeved.sqlite"))
}

/// Ensure the parent directory of `path` exists.
pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    Ok(())
}

/// Write the current process PID to `path`. Errors if the file already exists.
pub fn write_pid_file(path: &Path) -> Result<()> {
    ensure_parent_dir(path)?;
    let pid = std::process::id();
    let content = format!("{pid}\n");
    fs::write(path, content).with_context(|| format!("write pid file {}", path.display()))?;
    Ok(())
}

/// Remove a PID file, ignoring NotFound.
pub fn remove_pid_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("remove pid file {}", path.display()))),
    }
}

/// Read the PID from a PID file, returning `Ok(None)` if it doesn't exist.
pub fn read_pid_file(path: &Path) -> Result<Option<u32>> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let trimmed = s.trim();
            let pid: u32 = trimmed.parse()
                .with_context(|| format!("parse pid from {}: {trimmed:?}", path.display()))?;
            Ok(Some(pid))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e).context(format!("read pid file {}", path.display()))),
    }
}

/// Returns true if a process with the given PID is running.
pub fn process_running(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pid_file_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("reeved.pid");

        assert_eq!(read_pid_file(&path).unwrap(), None);
        write_pid_file(&path).unwrap();
        let pid = read_pid_file(&path).unwrap().unwrap();
        assert_eq!(pid, std::process::id());
        remove_pid_file(&path).unwrap();
        assert_eq!(read_pid_file(&path).unwrap(), None);
    }

    #[test]
    fn remove_pid_file_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.pid");
        remove_pid_file(&path).unwrap();
        remove_pid_file(&path).unwrap();
    }

    #[test]
    fn process_running_for_self_returns_true() {
        let pid = std::process::id();
        assert!(process_running(pid));
    }
}
```

- [ ] **Step 2: Add `tempfile` to reeved dev-dependencies**

Append to `crates/reeved/Cargo.toml`:
```toml

[dev-dependencies]
tempfile = "3.13"
```

- [ ] **Step 3: Wire the module into the daemon binary**

Replace `crates/reeved/src/main.rs` with:
```rust
mod db;
mod lifecycle;

fn main() {
    println!("reeved placeholder");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p reeved`
Expected: PASS — `db::tests` (5 tests) and `lifecycle::tests` (3 tests) all green.

- [ ] **Step 5: Commit**

```sh
git add crates/reeved/Cargo.toml crates/reeved/src/lifecycle.rs crates/reeved/src/main.rs
git commit -m "feat(reeved): lifecycle helpers for paths and PID files"
```

---

## Task 6: Unix-socket JSON-line server

**Files:**
- Create: `crates/reeved/src/server.rs`
- Modify: `crates/reeved/src/main.rs` (add `mod server`)

- [ ] **Step 1: Write a failing integration test that boots the server and round-trips a request**

Create `crates/reeved/src/server.rs`:
```rust
//! Unix-socket JSON-line server.

use anyhow::{Context, Result};
use reeve_protocol::{RpcError, RpcRequest, RpcResponse};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};
use tracing::{error, warn};

/// A request handler shared across connections.
pub type DbHandle = Arc<Mutex<Connection>>;

/// Run the server on `socket_path`. Removes the socket on shutdown.
/// `dispatch` receives the parsed request and produces a result or RpcError.
pub async fn run<F, Fut>(
    socket_path: &Path,
    db: DbHandle,
    mut shutdown: oneshot::Receiver<()>,
    dispatch: F,
) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("remove stale socket {}", socket_path.display()))?;
    }
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        let db = db.clone();
                        let dispatch = dispatch.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, db, dispatch).await {
                                warn!("connection error: {e:#}");
                            }
                        });
                    }
                    Err(e) => {
                        error!("accept failed: {e:#}");
                    }
                }
            }
        }
    }

    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

async fn handle_connection<F, Fut>(stream: UnixStream, db: DbHandle, dispatch: F) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => match dispatch(req.clone(), db.clone()).await {
                Ok(value) => RpcResponse {
                    id: req.id,
                    result: Some(value),
                    error: None,
                },
                Err(err) => RpcResponse {
                    id: req.id,
                    result: None,
                    error: Some(err),
                },
            },
            Err(e) => RpcResponse {
                id: 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tempfile::tempdir;

    fn echo_db() -> DbHandle {
        Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()))
    }

    #[tokio::test]
    async fn server_round_trips_request_and_response() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("test.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();

        // Echo dispatcher: returns the request's params verbatim.
        let dispatch = |req: RpcRequest, _db: DbHandle| async move { Ok(req.params) };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        // Wait until the socket exists.
        for _ in 0..50 {
            if socket.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let mut stream = UnixStream::connect(&socket).await.unwrap();
        let req = RpcRequest {
            id: 42,
            method: "echo".into(),
            params: serde_json::json!({"hi": "there"}),
        };
        let mut bytes = serde_json::to_vec(&req).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: RpcResponse = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(resp.id, 42);
        assert_eq!(resp.result, Some(serde_json::json!({"hi": "there"})));
        assert!(resp.error.is_none());

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_request_yields_invalid_params_error() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("bad.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();
        let dispatch = |_req: RpcRequest, _db: DbHandle| async {
            Ok(serde_json::Value::Null)
        };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        for _ in 0..50 {
            if socket.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let mut stream = UnixStream::connect(&socket).await.unwrap();
        stream.write_all(b"not json\n").await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: RpcResponse = serde_json::from_str(line.trim()).unwrap();
        let err = resp.error.unwrap();
        assert_eq!(err.code, RpcError::INVALID_PARAMS);

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    // Suppress unused warning until test count is meaningful.
    #[allow(dead_code)]
    static _N: AtomicU32 = AtomicU32::new(0);
}
```

- [ ] **Step 2: Wire the module into the daemon binary**

Replace `crates/reeved/src/main.rs` with:
```rust
mod db;
mod lifecycle;
mod server;

fn main() {
    println!("reeved placeholder");
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p reeved`
Expected: PASS — db, lifecycle, and server tests all green.

- [ ] **Step 4: Commit**

```sh
git add crates/reeved/src/server.rs crates/reeved/src/main.rs
git commit -m "feat(reeved): Unix-socket JSON-line server with shutdown handling"
```

---

## Task 7: RPC handlers wiring (`ls`, `new`, `mv`)

**Files:**
- Create: `crates/reeved/src/handlers.rs`
- Modify: `crates/reeved/src/main.rs` (add `mod handlers`)

- [ ] **Step 1: Write failing tests for each handler**

Create `crates/reeved/src/handlers.rs`:
```rust
//! Method dispatch and per-method handlers.

use crate::db;
use crate::server::DbHandle;
use anyhow::Result;
use reeve_protocol::{
    Card, Column, LsResult, MvParams, MvResult, NewParams, NewResult, RpcError, RpcRequest,
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn dispatch(req: RpcRequest, db: DbHandle) -> std::result::Result<Value, RpcError> {
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

fn parse_params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError {
        code: RpcError::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
    })
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn to_internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

async fn handle_ls(db: DbHandle) -> std::result::Result<Value, RpcError> {
    let cards = tokio::task::spawn_blocking(move || -> Result<Vec<Card>> {
        let conn = db.blocking_lock();
        db::list_cards(&conn)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    serde_json::to_value(LsResult { cards }).map_err(to_internal)
}

async fn handle_new(db: DbHandle, params: NewParams) -> std::result::Result<Value, RpcError> {
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
    let card_for_insert = card.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = db.blocking_lock();
        db::insert_card(&conn, &card_for_insert)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    serde_json::to_value(NewResult { card }).map_err(to_internal)
}

async fn handle_mv(db: DbHandle, params: MvParams) -> std::result::Result<Value, RpcError> {
    let id = params.id.clone();
    let column = params.column;
    let now = now_secs();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id, column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    match card {
        Some(card) => serde_json::to_value(MvResult { card }).map_err(to_internal),
        None => Err(RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card not found: {}", params.id),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn fresh_db() -> DbHandle {
        let conn = db::open(":memory:").unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test]
    async fn ls_on_empty_returns_empty_list() {
        let db = fresh_db();
        let req = RpcRequest {
            id: 1,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let value = dispatch(req, db).await.unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert!(result.cards.is_empty());
    }

    #[tokio::test]
    async fn new_creates_card_then_ls_returns_it() {
        let db = fresh_db();
        let req_new = RpcRequest {
            id: 1,
            method: "new".into(),
            params: serde_json::json!({
                "household": "myarchy",
                "brief": "first card",
                "needs_heavy": false,
            }),
        };
        let new_value = dispatch(req_new, db.clone()).await.unwrap();
        let new_result: NewResult = serde_json::from_value(new_value).unwrap();
        assert_eq!(new_result.card.brief, "first card");
        assert_eq!(new_result.card.column, Column::Backlog);

        let req_ls = RpcRequest {
            id: 2,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let ls_value = dispatch(req_ls, db).await.unwrap();
        let ls_result: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls_result.cards.len(), 1);
        assert_eq!(ls_result.cards[0], new_result.card);
    }

    #[tokio::test]
    async fn mv_changes_column_and_returns_updated() {
        let db = fresh_db();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "myarchy",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            db.clone(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": card.id,
                    "column": "implementing",
                }),
            },
            db,
        )
        .await
        .unwrap();
        let updated = serde_json::from_value::<MvResult>(mv_value).unwrap().card;
        assert_eq!(updated.column, Column::Implementing);
    }

    #[tokio::test]
    async fn mv_unknown_id_returns_not_found() {
        let db = fresh_db();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": "missing",
                    "column": "done",
                }),
            },
            db,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let db = fresh_db();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "nope".into(),
                params: serde_json::json!({}),
            },
            db,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::METHOD_NOT_FOUND);
    }
}
```

- [ ] **Step 2: Wire the module into the daemon binary**

Replace `crates/reeved/src/main.rs` with:
```rust
mod db;
mod handlers;
mod lifecycle;
mod server;

fn main() {
    println!("reeved placeholder");
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p reeved`
Expected: PASS — all db, lifecycle, server, and handlers tests green.

- [ ] **Step 4: Commit**

```sh
git add crates/reeved/src/handlers.rs crates/reeved/src/main.rs
git commit -m "feat(reeved): RPC handlers for ls, new, mv"
```

---

## Task 8: Daemon main entry point

**Files:**
- Modify: `crates/reeved/src/main.rs`

- [ ] **Step 1: Replace placeholder main with full async entry point**

Replace `crates/reeved/src/main.rs` with:
```rust
mod db;
mod handlers;
mod lifecycle;
mod server;

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, oneshot};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reeved=info".into()),
        )
        .init();

    let socket_path = lifecycle::socket_path();
    let pid_path = lifecycle::pid_path();
    let db_path = lifecycle::db_path()?;

    lifecycle::ensure_parent_dir(&db_path)?;
    lifecycle::ensure_parent_dir(&socket_path)?;

    if let Some(existing_pid) = lifecycle::read_pid_file(&pid_path)? {
        if lifecycle::process_running(existing_pid) {
            anyhow::bail!(
                "reeved already running as PID {existing_pid} ({}). \
                 Stop it first with `reeve daemon stop`.",
                pid_path.display()
            );
        } else {
            // Stale PID file — overwrite.
            lifecycle::remove_pid_file(&pid_path)?;
        }
    }

    let conn = db::open(db_path.to_str().context("db path not utf-8")?)?;
    let db = Arc::new(Mutex::new(conn));

    lifecycle::write_pid_file(&pid_path)?;
    info!(socket = %socket_path.display(), "reeved started");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    spawn_signal_handler(shutdown_tx);

    let result = server::run(&socket_path, db, shutdown_rx, handlers::dispatch).await;

    lifecycle::remove_pid_file(&pid_path)?;
    info!("reeved stopped");
    result
}

fn spawn_signal_handler(tx: oneshot::Sender<()>) {
    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM");
        let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT");
        tokio::select! {
            _ = sigterm.recv() => {},
            _ = sigint.recv() => {},
        }
        let _ = tx.send(());
    });
}
```

- [ ] **Step 2: Build the daemon**

Run: `cargo build -p reeved`
Expected: success.

- [ ] **Step 3: Smoke-test the daemon manually**

Run (in one terminal):
```sh
RUST_LOG=reeved=debug ./target/debug/reeved
```
Expected: log line `reeved started socket=...`. The process stays alive.

In another terminal, send a `Ctrl-C` (SIGINT) to the daemon's terminal.
Expected: log line `reeved stopped`. Process exits cleanly. The socket and pid file under `$XDG_RUNTIME_DIR/` are cleaned up.

- [ ] **Step 4: Commit**

```sh
git add crates/reeved/src/main.rs
git commit -m "feat(reeved): async main with signal handling and pid file"
```

---

## Task 9: Client module in `reeve` crate

**Files:**
- Create: `crates/reeve/src/client.rs`
- Modify: `crates/reeve/src/main.rs` (add `mod client`)

- [ ] **Step 1: Write failing tests that exercise the client against an in-process echo server**

Create `crates/reeve/src/client.rs`:
```rust
//! IPC client: sends one RPC request over a Unix socket, returns the response.

use anyhow::{Context, Result};
use reeve_protocol::{RpcError, RpcRequest, RpcResponse};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Connect to the daemon socket and perform a single request/response cycle.
pub async fn call(
    socket_path: &Path,
    method: &str,
    params: serde_json::Value,
) -> Result<std::result::Result<serde_json::Value, RpcError>> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect to {}", socket_path.display()))?;

    let req = RpcRequest {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        method: method.to_owned(),
        params,
    };
    let mut bytes = serde_json::to_vec(&req)?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    stream.shutdown().await.ok();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        anyhow::bail!("daemon closed connection without responding");
    }
    let resp: RpcResponse = serde_json::from_str(line.trim())?;
    if let Some(err) = resp.error {
        return Ok(Err(err));
    }
    Ok(Ok(resp.result.unwrap_or(serde_json::Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::net::UnixListener;

    /// Starts a tiny echo server that replies with the same `params` field.
    async fn start_echo(socket_path: &Path) {
        let listener = UnixListener::bind(socket_path).unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let req: RpcRequest = serde_json::from_str(&line).unwrap();
                    let resp = RpcResponse {
                        id: req.id,
                        result: Some(req.params),
                        error: None,
                    };
                    let mut bytes = serde_json::to_vec(&resp).unwrap();
                    bytes.push(b'\n');
                    write.write_all(&bytes).await.unwrap();
                    break;
                }
            }
        });
    }

    #[tokio::test]
    async fn client_round_trips_to_echo_server() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("echo.sock");
        start_echo(&socket).await;
        // Give the listener a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = call(&socket, "ping", serde_json::json!({"hi": 1}))
            .await
            .unwrap();
        let value = result.unwrap();
        assert_eq!(value, serde_json::json!({"hi": 1}));
    }

    #[tokio::test]
    async fn client_propagates_rpc_error_response() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("err.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                if let Ok(Some(_)) = lines.next_line().await {
                    let resp = RpcResponse {
                        id: 1,
                        result: None,
                        error: Some(RpcError {
                            code: 404,
                            message: "nope".into(),
                        }),
                    };
                    let mut bytes = serde_json::to_vec(&resp).unwrap();
                    bytes.push(b'\n');
                    write.write_all(&bytes).await.unwrap();
                }
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = call(&socket, "anything", serde_json::json!({})).await.unwrap();
        let err = result.unwrap_err();
        assert_eq!(err.code, 404);
        assert_eq!(err.message, "nope");
    }
}
```

- [ ] **Step 2: Add `tempfile` to reeve dev-dependencies**

Append to `crates/reeve/Cargo.toml`:
```toml

[dev-dependencies]
tempfile = "3.13"
```

- [ ] **Step 3: Wire the module into the user binary**

Replace `crates/reeve/src/main.rs` with:
```rust
mod client;

fn main() {
    println!("reeve placeholder");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p reeve`
Expected: PASS — both client tests green.

- [ ] **Step 5: Commit**

```sh
git add crates/reeve/Cargo.toml crates/reeve/src/client.rs crates/reeve/src/main.rs
git commit -m "feat(reeve): IPC client for daemon socket"
```

---

## Task 10: CLI subcommands for `ls`, `new`, `mv`

**Files:**
- Create: `crates/reeve/src/cli.rs`
- Modify: `crates/reeve/src/main.rs`

- [ ] **Step 1: Add the lifecycle helpers shared with reeved**

The CLI needs the same socket path resolver as the daemon. To avoid duplicating, add the relevant helpers to `reeve-protocol` so both binaries can use them.

Modify `crates/reeve-protocol/Cargo.toml` — add `nix` dependency:
```toml
[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
nix = { version = "0.29", features = ["user"] }
```

Append to `crates/reeve-protocol/src/lib.rs`:
```rust
/// Shared runtime-directory helpers (used by both the daemon and clients).
pub mod paths {
    use std::path::PathBuf;

    /// Returns the runtime directory used for sockets and pid files.
    pub fn runtime_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        let uid = nix::unistd::Uid::current().as_raw();
        PathBuf::from(format!("/tmp/reeve-{uid}"))
    }

    pub fn socket_path() -> PathBuf {
        runtime_dir().join("reeved.sock")
    }

    pub fn pid_path() -> PathBuf {
        runtime_dir().join("reeved.pid")
    }
}
```

Then modify `crates/reeved/src/lifecycle.rs` so the daemon delegates to these — replace the `runtime_dir`, `socket_path`, `pid_path` functions with:
```rust
pub fn runtime_dir() -> PathBuf {
    reeve_protocol::paths::runtime_dir()
}

pub fn socket_path() -> PathBuf {
    reeve_protocol::paths::socket_path()
}

pub fn pid_path() -> PathBuf {
    reeve_protocol::paths::pid_path()
}
```

Run: `cargo test --workspace`
Expected: PASS — all existing tests still green.

- [ ] **Step 2: Write failing CLI logic**

Create `crates/reeve/src/cli.rs`:
```rust
//! CLI subcommands handlers.

use crate::client;
use anyhow::{Context, Result};
use clap::Subcommand;
use reeve_protocol::{Card, Column, LsResult, MvResult, NewResult, paths};

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
    /// Daemon management subcommands.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Start the daemon as a background process.
    Start,
    /// Stop the running daemon (SIGTERM).
    Stop,
    /// Check whether the daemon is running.
    Status,
}

fn parse_column(s: &str) -> Result<Column, String> {
    match s.to_lowercase().as_str() {
        "backlog" => Ok(Column::Backlog),
        "planning" => Ok(Column::Planning),
        "implementing" => Ok(Column::Implementing),
        "testing" => Ok(Column::Testing),
        "review" => Ok(Column::Review),
        "done" => Ok(Column::Done),
        other => Err(format!("unknown column {other:?}; expected one of {:?}", Column::ALL.iter().map(|c| c.label()).collect::<Vec<_>>())),
    }
}

pub async fn run(cmd: Command) -> Result<()> {
    match cmd {
        Command::Ls => ls().await,
        Command::New { household, brief, needs_heavy } => new(household, brief, needs_heavy).await,
        Command::Mv { id, column } => mv(id, column).await,
        Command::Daemon(_) => unreachable!("Daemon commands are handled in main"),
    }
}

async fn ls() -> Result<()> {
    let socket = paths::socket_path();
    let result = client::call(&socket, "ls", serde_json::json!({}))
        .await
        .context("calling ls")?;
    match result {
        Ok(value) => {
            let LsResult { cards } = serde_json::from_value(value)?;
            print_cards(&cards);
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn new(household: String, brief: String, needs_heavy: bool) -> Result<()> {
    let socket = paths::socket_path();
    let result = client::call(
        &socket,
        "new",
        serde_json::json!({
            "household": household,
            "brief": brief,
            "needs_heavy": needs_heavy,
        }),
    )
    .await
    .context("calling new")?;
    match result {
        Ok(value) => {
            let NewResult { card } = serde_json::from_value(value)?;
            println!("created {} ({})", card.id, card.column.label());
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn mv(id: String, column: Column) -> Result<()> {
    let socket = paths::socket_path();
    let result = client::call(
        &socket,
        "mv",
        serde_json::json!({ "id": id, "column": column }),
    )
    .await
    .context("calling mv")?;
    match result {
        Ok(value) => {
            let MvResult { card } = serde_json::from_value(value)?;
            println!("{} -> {}", card.id, card.column.label());
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

fn print_cards(cards: &[Card]) {
    if cards.is_empty() {
        println!("(no cards)");
        return;
    }
    println!("{:<36}  {:<14}  {:<12}  {}", "ID", "COLUMN", "HOUSEHOLD", "BRIEF");
    for card in cards {
        let brief: String = if card.brief.len() > 60 {
            format!("{}…", &card.brief[..59])
        } else {
            card.brief.clone()
        };
        println!(
            "{:<36}  {:<14}  {:<12}  {}",
            card.id,
            card.column.label(),
            card.household,
            brief,
        );
    }
}
```

- [ ] **Step 3: Replace `main.rs` with a clap-driven entry point that dispatches**

Replace `crates/reeve/src/main.rs` with:
```rust
mod cli;
mod client;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "reeve", version, about = "Reeve TUI orchestrator")]
struct Args {
    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        None => {
            eprintln!("TUI not implemented yet (M1 task 12)");
            Ok(())
        }
        Some(cli::Command::Daemon(_sub)) => {
            eprintln!("daemon subcommand not implemented yet (M1 task 11)");
            Ok(())
        }
        Some(cmd) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::run(cmd))
        }
    }
}
```

- [ ] **Step 4: Build the workspace**

Run: `cargo build`
Expected: success — both binaries compile.

- [ ] **Step 5: Smoke-test against a running daemon**

Start the daemon in one terminal:
```sh
./target/debug/reeved
```

In another terminal:
```sh
./target/debug/reeve ls
./target/debug/reeve new --household myarchy --brief "first card"
./target/debug/reeve ls
```

Expected:
- First `ls` → `(no cards)`
- `new` → `created <uuid> (Backlog)`
- Second `ls` → table row with the new card

Then take the printed UUID and:
```sh
./target/debug/reeve mv <uuid> implementing
./target/debug/reeve ls
```
Expected: `<uuid> -> Implementing`, then table shows column `Implementing`.

Stop the daemon with `Ctrl-C`.

- [ ] **Step 6: Commit**

```sh
git add crates/
git commit -m "feat(reeve): CLI subcommands ls/new/mv"
```

---

## Task 11: CLI `daemon` subcommands (start, stop, status)

**Files:**
- Create: `crates/reeve/src/cli/daemon.rs` (or extend `cli.rs` — choose extension to avoid splitting modules)

For simplicity, extend `crates/reeve/src/cli.rs` with daemon subcommand handlers.

- [ ] **Step 1: Locate the path of the `reeved` binary alongside `reeve`**

Append helper near the top of `crates/reeve/src/cli.rs`:
```rust
use std::path::PathBuf;

fn reeved_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locate current_exe")?;
    let parent = exe.parent().context("current_exe has no parent")?;
    let candidate = parent.join("reeved");
    if candidate.exists() {
        Ok(candidate)
    } else if let Ok(p) = std::env::var("REEVED_PATH") {
        Ok(PathBuf::from(p))
    } else {
        anyhow::bail!(
            "reeved binary not found at {}; set REEVED_PATH if it lives elsewhere",
            candidate.display()
        )
    }
}
```

- [ ] **Step 2: Implement `daemon start`/`stop`/`status`**

Append to `crates/reeve/src/cli.rs`:
```rust
use std::process::Stdio;

pub fn run_daemon(cmd: DaemonCommand) -> Result<()> {
    match cmd {
        DaemonCommand::Start => start(),
        DaemonCommand::Stop => stop(),
        DaemonCommand::Status => status(),
    }
}

fn start() -> Result<()> {
    let pid_path = paths::pid_path();
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            if process_running(pid as u32) {
                anyhow::bail!("daemon already running as PID {pid}");
            }
        }
    }

    let reeved = reeved_path()?;
    let child = std::process::Command::new(&reeved)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", reeved.display()))?;

    // Give the daemon a moment to write its pid file.
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < std::time::Duration::from_secs(2) {
        if let Some(pid) = read_pid_from(&pid_path)? {
            if pid == child.id() {
                println!("daemon started (PID {pid})");
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not write a PID file within 2s");
}

fn stop() -> Result<()> {
    let pid_path = paths::pid_path();
    let pid = read_pid_from(&pid_path)?
        .context("daemon not running (no pid file)")?;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::SIGTERM,
    )
    .with_context(|| format!("send SIGTERM to PID {pid}"))?;

    // Wait up to 5s for the process to exit.
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < std::time::Duration::from_secs(5) {
        if !process_running(pid) {
            println!("daemon stopped (was PID {pid})");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("daemon (PID {pid}) did not exit within 5s of SIGTERM");
}

fn status() -> Result<()> {
    let pid_path = paths::pid_path();
    match read_pid_from(&pid_path)? {
        Some(pid) if process_running(pid) => {
            println!("running (PID {pid})");
            Ok(())
        }
        Some(pid) => {
            println!("not running (stale PID file references {pid})");
            Ok(())
        }
        None => {
            println!("not running");
            Ok(())
        }
    }
}

fn read_pid_from(path: &std::path::Path) -> Result<Option<u32>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s.trim().parse::<u32>()?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e)),
    }
}

fn process_running(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}
```

- [ ] **Step 3: Wire the daemon subcommand in `main.rs`**

Replace `crates/reeve/src/main.rs` with:
```rust
mod cli;
mod client;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "reeve", version, about = "Reeve TUI orchestrator")]
struct Args {
    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        None => {
            eprintln!("TUI not implemented yet (M1 task 12)");
            Ok(())
        }
        Some(cli::Command::Daemon(sub)) => cli::run_daemon(sub),
        Some(cmd) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::run(cmd))
        }
    }
}
```

- [ ] **Step 4: Build and smoke-test daemon subcommands**

Run: `cargo build`

Smoke test:
```sh
./target/debug/reeve daemon status     # not running
./target/debug/reeve daemon start      # daemon started (PID …)
./target/debug/reeve daemon status     # running (PID …)
./target/debug/reeve ls                # (no cards)
./target/debug/reeve daemon stop       # daemon stopped
./target/debug/reeve daemon status     # not running
```

- [ ] **Step 5: Commit**

```sh
git add crates/reeve/src/cli.rs crates/reeve/src/main.rs
git commit -m "feat(reeve): daemon start/stop/status subcommands"
```

---

## Task 12: Read-only Ratatui kanban viewer

**Files:**
- Create: `crates/reeve/src/tui.rs`
- Modify: `crates/reeve/src/main.rs`

- [ ] **Step 1: Create the TUI module**

Create `crates/reeve/src/tui.rs`:
```rust
//! Minimal read-only Ratatui kanban viewer for M1.

use crate::client;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use reeve_protocol::{Card, Column, LsResult, paths};
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};

pub fn run() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let mut terminal = setup_terminal()?;
    let result = runtime.block_on(run_loop(&mut terminal));
    restore_terminal(&mut terminal)?;
    result
}

type Term = Terminal<CrosstermBackend<Stdout>>;

fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_loop(terminal: &mut Term) -> Result<()> {
    let mut cards: Vec<Card> = fetch_cards().await.unwrap_or_default();
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| render(f, &cards))?;

        // Poll for input with a short timeout so we can refresh periodically.
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => {
                        cards = fetch_cards().await.unwrap_or_default();
                        last_refresh = Instant::now();
                    }
                    _ => {}
                }
            }
        }

        if last_refresh.elapsed() > Duration::from_millis(500) {
            cards = fetch_cards().await.unwrap_or_default();
            last_refresh = Instant::now();
        }
    }
}

async fn fetch_cards() -> Result<Vec<Card>> {
    let socket = paths::socket_path();
    match client::call(&socket, "ls", serde_json::json!({})).await? {
        Ok(value) => {
            let LsResult { cards } = serde_json::from_value(value)?;
            Ok(cards)
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

fn render(f: &mut ratatui::Frame<'_>, cards: &[Card]) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled("reeve", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw("q quit  r refresh"),
    ]));
    f.render_widget(title, chunks[0]);

    render_columns(f, chunks[1], cards);
}

fn render_columns(f: &mut ratatui::Frame<'_>, area: Rect, cards: &[Card]) {
    let columns = Column::ALL;
    let constraints = vec![Constraint::Percentage(100 / columns.len() as u16); columns.len()];
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, col) in columns.iter().enumerate() {
        let items: Vec<ListItem> = cards
            .iter()
            .filter(|c| c.column == *col)
            .map(card_to_listitem)
            .collect();
        let count = items.len();
        let title = format!("{} ({count})", col.label());
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().fg(Color::White));
        f.render_widget(list, cells[i]);
    }
}

fn card_to_listitem(card: &Card) -> ListItem<'_> {
    let id_short = card.id.get(..8).unwrap_or(&card.id);
    let line = Line::from(vec![
        Span::styled(
            format!("{id_short} "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(card.brief.as_str()),
    ]);
    ListItem::new(line)
}
```

- [ ] **Step 2: Wire the TUI into `main`**

Replace `crates/reeve/src/main.rs` with:
```rust
mod cli;
mod client;
mod tui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "reeve", version, about = "Reeve TUI orchestrator")]
struct Args {
    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        None => tui::run(),
        Some(cli::Command::Daemon(sub)) => cli::run_daemon(sub),
        Some(cmd) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::run(cmd))
        }
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: success.

- [ ] **Step 4: Smoke-test the TUI**

```sh
./target/debug/reeve daemon start
./target/debug/reeve new --household myarchy --brief "alpha"
./target/debug/reeve new --household myarchy --brief "beta"
./target/debug/reeve mv <id-of-alpha> implementing
./target/debug/reeve   # launches TUI
```

Expected: a kanban grid with six columns, "alpha" appearing in `Implementing (1)`, "beta" in `Backlog (1)`. Press `q` to quit.

Then: `./target/debug/reeve daemon stop`

- [ ] **Step 5: Commit**

```sh
git add crates/reeve/src/tui.rs crates/reeve/src/main.rs
git commit -m "feat(reeve): minimal read-only Ratatui kanban viewer"
```

---

## Task 13: Workspace-level end-to-end smoke test

**Files:**
- Create: `tests/e2e.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Add a top-level integration test crate**

Create `tests/e2e.rs` at the workspace root:
```rust
//! End-to-end smoke test for the M1 walking skeleton.
//!
//! Spawns `reeved` as a child process, drives it through the `reeve` CLI,
//! verifies metadata round-trips correctly, and shuts the daemon down cleanly.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

fn target_dir() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set under cargo");
    PathBuf::from(manifest).join("target/debug")
}

fn build_workspace() {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--workspace"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "workspace build failed");
}

fn isolated_env(temp: &std::path::Path) -> Vec<(String, String)> {
    vec![
        ("XDG_RUNTIME_DIR".into(), temp.to_string_lossy().into_owned()),
        ("XDG_DATA_HOME".into(), temp.join("data").to_string_lossy().into_owned()),
    ]
}

fn cmd(bin: &str, env: &[(String, String)]) -> Command {
    let mut c = Command::new(target_dir().join(bin));
    c.env_clear();
    if let Ok(home) = std::env::var("HOME") {
        c.env("HOME", home);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c
}

#[test]
fn m1_walking_skeleton_round_trip() {
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env(temp.path());

    // Start daemon as a child we control directly.
    let mut daemon = cmd("reeved", &env)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reeved");

    // Wait for socket.
    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists(), "socket not created within 2s");

    // ls should be empty.
    let ls_empty = cmd("reeve", &env).arg("ls").output().expect("run reeve ls");
    assert!(ls_empty.status.success());
    let stdout = String::from_utf8(ls_empty.stdout).unwrap();
    assert!(stdout.contains("(no cards)"), "unexpected stdout: {stdout}");

    // new should create a card.
    let out_new = cmd("reeve", &env)
        .args(["new", "--household", "myarchy", "--brief", "first"])
        .output()
        .expect("run reeve new");
    assert!(out_new.status.success());
    let stdout = String::from_utf8(out_new.stdout).unwrap();
    let card_id = stdout
        .split_whitespace()
        .nth(1)
        .expect("uuid in stdout")
        .to_owned();

    // ls should now show one card.
    let out_ls = cmd("reeve", &env).arg("ls").output().expect("run reeve ls");
    let stdout = String::from_utf8(out_ls.stdout).unwrap();
    assert!(stdout.contains("first"), "expected card brief: {stdout}");
    assert!(stdout.contains("Backlog"), "expected Backlog column: {stdout}");

    // mv should change column.
    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("run reeve mv");
    assert!(out_mv.status.success());
    let stdout = String::from_utf8(out_mv.stdout).unwrap();
    assert!(
        stdout.contains("Implementing"),
        "expected Implementing in mv output: {stdout}"
    );

    // ls should now show Implementing.
    let out_ls2 = cmd("reeve", &env).arg("ls").output().expect("run reeve ls");
    let stdout = String::from_utf8(out_ls2.stdout).unwrap();
    assert!(stdout.contains("Implementing"), "expected column updated: {stdout}");

    // SIGTERM the daemon and wait briefly.
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(daemon.id() as i32),
        nix::sys::signal::SIGTERM,
    )
    .unwrap();
    let _ = daemon.wait();
}
```

- [ ] **Step 2: Add an `e2e-tests` test crate to the workspace**

Tests at the workspace root require a small dummy crate to host them. Create `tests/Cargo.toml`:
```toml
[package]
name = "reeve-e2e"
version = "0.1.0"
edition.workspace = true

[lib]
path = "lib.rs"

[[test]]
name = "e2e"
path = "e2e.rs"

[dev-dependencies]
tempfile = "3.13"
nix = { version = "0.29", features = ["signal", "process"] }
```

Create `tests/lib.rs` (empty placeholder so the crate compiles):
```rust
//! E2E test crate for Reeve.
```

Add `tests` to the workspace members in the root `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/reeve-protocol", "crates/reeved", "crates/reeve", "tests"]
```

- [ ] **Step 3: Run the E2E test**

Run: `cargo test -p reeve-e2e`
Expected: PASS — single E2E test green. (Build will compile the workspace first.)

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --workspace`
Expected: PASS — all unit and integration tests green.

- [ ] **Step 5: Commit**

```sh
git add Cargo.toml tests/
git commit -m "test: end-to-end smoke test for M1 walking skeleton"
```

---

## Definition of done

After Task 13:

- `cargo test --workspace` passes
- `reeve daemon start` boots the daemon; it writes a PID file and listens on the Unix socket
- `reeve new --household X --brief Y` creates a card; `reeve ls` shows it
- `reeve mv <id> <column>` moves the card; `reeve ls` reflects it
- `reeve` (no args) launches the TUI showing six columns and existing cards
- `reeve daemon stop` cleanly terminates the daemon
- All state survives daemon restart (SQLite under `$XDG_DATA_HOME/reeve/reeved.sqlite`)

The walking skeleton is ready. M2 (containers + git plumbing + first running agent) builds on this.
