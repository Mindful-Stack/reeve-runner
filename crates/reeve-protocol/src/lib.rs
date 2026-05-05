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

    /// Lowercase wire form, used by the JSON envelope (via serde rename),
    /// the SQLite `column_` field, and the CLI `mv <id> <column>` argument
    /// parser. The single source of truth for all three.
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            Column::Backlog => "backlog",
            Column::Planning => "planning",
            Column::Implementing => "implementing",
            Column::Testing => "testing",
            Column::Review => "review",
            Column::Done => "done",
        }
    }

    /// Parse the lowercase wire form. Returns `None` for unknown values.
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "backlog" => Some(Column::Backlog),
            "planning" => Some(Column::Planning),
            "implementing" => Some(Column::Implementing),
            "testing" => Some(Column::Testing),
            "review" => Some(Column::Review),
            "done" => Some(Column::Done),
            _ => None,
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
/// `id` is `None` only when the daemon could not parse the request well
/// enough to recover its id (e.g. malformed JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
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

/// PID-file management and process-liveness checks shared between the
/// daemon's startup precheck and the user binary's `daemon start/stop/status`.
///
/// These return `std::io::Result` rather than `anyhow::Result` to keep the
/// protocol crate dep-light. Callers that want richer error context wrap
/// with `anyhow::Context` at the call site.
pub mod lifecycle {
    use std::fs;
    use std::io;
    use std::path::Path;

    /// Write the current process's PID to `path`, truncating any existing file.
    /// Creates the parent directory if needed. Does NOT enforce single-instance
    /// semantics — callers must check `read_pid_file` + `process_running` first.
    pub fn write_pid_file(path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{}\n", std::process::id()))
    }

    /// Remove a PID file, treating NotFound as success.
    pub fn remove_pid_file(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Read the PID from `path`, returning `Ok(None)` if the file doesn't exist.
    /// Returns an `InvalidData` error if the contents don't parse.
    pub fn read_pid_file(path: &Path) -> io::Result<Option<u32>> {
        match fs::read_to_string(path) {
            Ok(s) => s
                .trim()
                .parse::<u32>()
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Returns true if a process with the given PID is alive.
    /// Uses `kill(pid, 0)` — works for the same-uid case Reeve operates in.
    pub fn process_running(pid: u32) -> bool {
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
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
        fn read_pid_file_rejects_garbage() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("bad.pid");
            std::fs::write(&path, "not a pid").unwrap();
            let err = read_pid_file(&path).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        }

        #[test]
        fn process_running_for_self_returns_true() {
            assert!(process_running(std::process::id()));
        }
    }
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
            id: Some(1),
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn rpc_response_with_no_id_omits_id_field() {
        let resp = RpcResponse {
            id: None,
            result: None,
            error: Some(RpcError {
                code: RpcError::INVALID_PARAMS,
                message: "bad".into(),
            }),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"id\""));
        assert!(json.contains("\"error\""));
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

    #[test]
    fn column_wire_str_round_trips_for_all_variants() {
        for col in Column::ALL {
            let wire = col.as_wire_str();
            assert_eq!(Column::from_wire_str(wire), Some(col));
            // Wire form must match the serde lowercase representation.
            assert_eq!(
                serde_json::to_value(col).unwrap(),
                serde_json::Value::String(wire.into())
            );
        }
    }

    #[test]
    fn column_from_wire_str_rejects_unknown() {
        assert_eq!(Column::from_wire_str("nope"), None);
        assert_eq!(Column::from_wire_str(""), None);
        // Wire form is case-sensitive — uppercase is not accepted.
        assert_eq!(Column::from_wire_str("Backlog"), None);
    }
}
