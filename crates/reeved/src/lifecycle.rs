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
