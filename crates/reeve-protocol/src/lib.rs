//! Shared protocol types for Reeve daemon and clients.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub const FORBIDDEN: i32 = 403;
    pub const CONFLICT: i32 = 409;
    pub const INVALID_ARGUMENT: i32 = 400;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HouseholdSource {
    Path { path: PathBuf },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdAddParams {
    pub name: String,
    pub source: HouseholdSource,
    /// Override default clone location for URL sources. Ignored for path sources.
    #[serde(default)]
    pub clone_into: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdNewParams {
    pub name: String,
    pub path: Option<PathBuf>,
    pub template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdSummary {
    pub name: String,
    pub meta_repo_path: PathBuf,
    pub workspace_name: String,
    pub knowledge_base: Option<String>,
    pub repo_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoOnDisk {
    /// Directory exists with its own .git/ — separate sibling repo.
    Sibling,
    /// Directory exists without .git/ — tracked as part of the workspace meta-repo.
    Inline,
    /// Directory doesn't exist on the host. Sibling that wasn't bootstrapped.
    Missing,
    /// The workspace entry itself — lives at the meta-repo root, always "present".
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDetail {
    pub name: String,
    pub url: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    /// True iff this entry is the manifest's `workspace`.
    pub is_workspace: bool,
    /// True iff this entry is the manifest's `knowledge_base`.
    pub is_knowledge_base: bool,
    /// On-disk classification, derived at `show` time.
    pub on_disk: RepoOnDisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReference {
    pub id: String,
    pub brief: String,
    pub column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseholdDetail {
    pub summary: HouseholdSummary,
    pub workspace_url: Option<String>,
    pub repos: Vec<RepoDetail>,
    pub cards: Vec<CardReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    pub branches: Vec<BranchPushResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchPushResult {
    /// `manifest.meta_repo` for the meta-repo; `repos[].name` for siblings.
    pub repo: String,
    pub branch: String,
    pub status: BranchPushStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BranchPushStatus {
    Pushed,
    NotFastForward,
    PushFailed { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachResult {
    /// docker-assigned container name (discoverable by `reeve.card_id` label).
    pub container_name: String,
    /// Fixed name `"reeve"` for M5. Reserved as a field for future per-card multi-session work.
    pub session_name: String,
    /// `/workspaces/<card-id>` — the in-container path the agent runs in.
    pub in_container_workspace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditParams {
    pub id: String,
    pub brief: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditResult {
    pub card: Card,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteParams {
    pub id: String,
}

/// Parameters for the `card_progress` method.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct CardProgressParams {
    pub id: String,
}

/// Result for the `card_progress` method.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct CardProgressResult {
    pub card_id: String,
    pub stages: Vec<ProgressStage>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ProgressStage {
    pub kind: ProgressKind,
    pub status: ProgressStatus,
    /// Unix-epoch seconds when the stage started. Set on the Active report
    /// and never mutated.
    pub started_at: i64,
    /// Unix-epoch seconds when the stage transitioned to Completed/Failed.
    /// `None` while Active. Duration = finished_at - started_at when set.
    pub finished_at: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgressKind {
    CloneMeta { name: String },
    CloneSibling { name: String },
    DevcontainerUp,
    StartingTmux,
    Done,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus {
    Active,
    Completed,
    Failed,
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

    /// Directory that holds per-card socket files. One file per active card.
    pub fn cards_socket_dir() -> PathBuf {
        runtime_dir().join("reeve/cards")
    }

    /// Socket path the daemon binds for a given card. The same path is
    /// bind-mounted into the card's container at /var/run/reeved.sock.
    pub fn card_socket_path(card_id: &str) -> PathBuf {
        cards_socket_dir().join(format!("{card_id}.sock"))
    }

    /// Env var that overrides the default cards workspace location.
    /// Used by tests to redirect clones into a temp dir.
    pub const REEVE_CARDS_DIR_ENV: &str = "REEVE_CARDS_DIR";

    /// Returns the directory holding per-card workspaces (one subdir per card).
    /// Prefers `$REEVE_CARDS_DIR`; falls back to `$HOME/reeve/cards`.
    ///
    /// User-visible by intent — external git tools (GitKraken, lazygit) point
    /// at this directory to inspect agent work. Not under `~/.local/share/`.
    pub fn cards_workspace_dir() -> PathBuf {
        if let Ok(dir) = std::env::var(REEVE_CARDS_DIR_ENV) {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join("reeve/cards")
    }

    /// Workspace root for a card — contains one subdir per declared repo.
    /// `devcontainer up --workspace-folder` targets this path.
    pub fn card_workspace_path(card_id: &str) -> PathBuf {
        cards_workspace_dir().join(card_id)
    }

    /// Directory inside the container where the daemon bind-mounts the host
    /// per-card socket directory. The individual socket file lives at
    /// `<IN_CONTAINER_CARDS_DIR>/<card_id>.sock`.
    pub const IN_CONTAINER_CARDS_DIR: &str = "/var/run/reeve-cards";

    /// Pick the socket path a `reeve` CLI process should connect to.
    ///
    /// In-container processes detect themselves via `REEVE_CARD_ID` and
    /// connect to `<IN_CONTAINER_CARDS_DIR>/<card_id>.sock` (the cards
    /// directory is bind-mounted so new socket files appear dynamically).
    /// Host processes fall back to the daemon's main socket.
    pub fn detect_socket() -> PathBuf {
        if let Ok(card_id) = std::env::var("REEVE_CARD_ID") {
            let sock = std::path::Path::new(IN_CONTAINER_CARDS_DIR).join(format!("{card_id}.sock"));
            if sock.exists() {
                return sock;
            }
        }
        socket_path()
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

    #[test]
    fn mv_self_params_serialize_with_just_column() {
        let p = MvSelfParams {
            column: Column::Review,
        };
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

    #[test]
    fn card_socket_path_lives_under_cards_socket_dir() {
        let p = paths::card_socket_path("abc-123");
        assert!(p.ends_with("reeve/cards/abc-123.sock"));
        assert_eq!(p.parent().unwrap(), paths::cards_socket_dir());
    }

    /// Serial gate for tests that mutate REEVE_CARDS_DIR. cargo runs tests
    /// in parallel by default; without this they race on the process-global
    /// env var (one set/remove can sneak between another's set and read).
    static REEVE_CARDS_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn cards_workspace_dir_uses_reeve_cards_dir_env() {
        let _guard = REEVE_CARDS_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("REEVE_CARDS_DIR").ok();
        unsafe { std::env::set_var("REEVE_CARDS_DIR", "/tmp/test-cards") };
        let dir = paths::cards_workspace_dir();
        assert_eq!(dir, std::path::PathBuf::from("/tmp/test-cards"));
        match prev {
            Some(v) => unsafe { std::env::set_var("REEVE_CARDS_DIR", v) },
            None => unsafe { std::env::remove_var("REEVE_CARDS_DIR") },
        }
    }

    #[test]
    fn cards_workspace_dir_falls_back_to_home_reeve_cards() {
        let _guard = REEVE_CARDS_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("REEVE_CARDS_DIR").ok();
        unsafe { std::env::remove_var("REEVE_CARDS_DIR") };
        let home = std::env::var("HOME").expect("HOME set in tests");
        let dir = paths::cards_workspace_dir();
        assert_eq!(dir, std::path::PathBuf::from(home).join("reeve/cards"));
        if let Some(v) = prev {
            unsafe { std::env::set_var("REEVE_CARDS_DIR", v) };
        }
    }

    #[test]
    fn card_workspace_path_lives_under_cards_workspace_dir() {
        let _guard = REEVE_CARDS_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let p = paths::card_workspace_path("abc-123");
        assert!(p.ends_with("abc-123"));
        assert_eq!(p.parent().unwrap(), paths::cards_workspace_dir());
    }

    #[test]
    fn progress_kind_serializes_with_tag_kind() {
        let k = ProgressKind::CloneSibling {
            name: "reeve".into(),
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"kind": "clone_sibling", "name": "reeve"}),
            "snake_case + #[serde(tag = \"kind\")] discriminator",
        );
    }

    #[test]
    fn progress_stage_round_trips() {
        let stage = ProgressStage {
            kind: ProgressKind::DevcontainerUp,
            status: ProgressStatus::Active,
            started_at: 1_700_000_000,
            finished_at: None,
        };
        let s = serde_json::to_string(&stage).unwrap();
        let back: ProgressStage = serde_json::from_str(&s).unwrap();
        assert_eq!(back.status, ProgressStatus::Active);
        assert_eq!(back.finished_at, None);
    }

    #[test]
    fn card_progress_result_with_no_stages_round_trips() {
        let r = CardProgressResult {
            card_id: "abc".into(),
            stages: vec![],
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: CardProgressResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back.card_id, "abc");
        assert!(back.stages.is_empty());
    }
}
