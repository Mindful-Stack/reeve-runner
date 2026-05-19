//! Daemon lifecycle helpers: data-dir resolution, DB opening, and schema migration.
//!
//! PID-file and process-liveness helpers (and the path resolvers) live in
//! `reeve_protocol` so the user binary can share them.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub use reeve_protocol::lifecycle::{
    process_running, read_pid_file, remove_pid_file, write_pid_file,
};
pub use reeve_protocol::paths::{pid_path, socket_path};

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

pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("reeved.sqlite"))
}

/// Ensure the parent directory of `path` exists.
pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {}", parent.display()))?;
    }
    Ok(())
}

/// Open a SQLite connection at `path` and run all schema migrations.
/// Pass `":memory:"` for an in-memory database (used in tests).
pub fn open_db(path: &str) -> Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path).with_context(|| format!("open db at {path}"))?;
    migrate(&conn)?;
    Ok(conn)
}

/// Orchestrate per-context schema initialisation.
/// Each context crate owns its own table definitions.
pub fn migrate(conn: &rusqlite::Connection) -> Result<()> {
    reeve_card::init_schema(conn)?;
    reeve_household::init_schema(conn)?;
    reeve_workspace::init_schema(conn)?;
    Ok(())
}
