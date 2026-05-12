//! Docker subprocess wrapper. The pure parts (argv building) live here;
//! the impure parts (shelling out, parsing output) are in `DockerOps`.

use crate::household::HouseholdProfile;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Container name for a card. Derived deterministically from card-id.
pub fn container_name(card_id: &str) -> String {
    format!("reeve-{card_id}")
}

/// Build the argv for `docker` (excluding the `docker` binary itself)
/// that spawns the container for `card_id` under `profile`.
///
/// - `cards_dir` is the host-side directory that holds per-card socket files
///   (`$XDG_RUNTIME_DIR/reeve/cards/`). It is bind-mounted into the container
///   at `IN_CONTAINER_CARDS_DIR` so that the in-container `reeve` CLI can
///   reach `<dir>/<card_id>.sock` after the daemon creates it post-spawn.
///   Mounting the directory (not the socket file itself) is essential: Docker
///   tracks file bind-mounts by inode, so a file deleted-and-recreated on the
///   host is invisible to the container. A directory mount is path-based and
///   reflects new entries automatically.
/// - `claude_dir` is the host `~/.claude` whose `skills/`, `settings.json`,
///   and `CLAUDE.md` are mounted read-only. Missing children are skipped
///   silently (the caller checks existence).
/// - `api_key` is injected as `ANTHROPIC_API_KEY` when `Some`.
pub fn spawn_command(
    card_id: &str,
    profile: &HouseholdProfile,
    cards_dir: &Path,
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
            cards_dir.display(),
            reeve_protocol::paths::IN_CONTAINER_CARDS_DIR
        ),
    ];

    let mounts: [(PathBuf, &str); 3] = [
        (claude_dir.join("skills"), "/home/agent/.claude/skills"),
        (
            claude_dir.join("settings.json"),
            "/home/agent/.claude/settings.json",
        ),
        (
            claude_dir.join("CLAUDE.md"),
            "/home/agent/.claude/CLAUDE.md",
        ),
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

use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::ContainerStatus;
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
            let stderr = String::from_utf8_lossy(&rm.stderr);
            // Container already gone (manually removed, or this is a defensive
            // teardown per spec). The desired end-state holds — don't bail.
            if stderr.to_ascii_lowercase().contains("no such") {
                return Ok(());
            }
            anyhow::bail!("docker rm -f {container_name} failed: {stderr}");
        }
        Ok(())
    }

    async fn inspect_status(&self, container_name: &str) -> Result<Option<String>> {
        let output = Command::new("docker")
            .args(["inspect", "--format", "{{.State.Status}}", container_name])
            .output()
            .await
            .context("exec docker inspect")?;
        if !output.status.success() {
            // "No such container" / "no such object" → None
            let stderr = String::from_utf8_lossy(&output.stderr);
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("no such") {
                return Ok(None);
            }
            anyhow::bail!("docker inspect failed: {stderr}");
        }
        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(status))
    }
}

/// Maps a `docker inspect` `.State.Status` string to our `ContainerStatus`.
///
/// Not currently called from production code — `reconcile_one` only
/// distinguishes `"running"` from everything else. Kept around (with the
/// allow) because the more granular mapping will be needed once `status-self`
/// reports something richer than "is the container alive". See follow-up M2-F1.
#[allow(dead_code)]
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
pub fn reconcile_one(_row: &ContainerRow, docker_state: Option<&str>) -> ReconcileDecision {
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
/// `ListenerRegistry::spawn` would do it anyway, but doing it explicitly
/// is clearer in logs).
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
        // Propagate inspect errors — silently downgrading them to None would
        // misclassify a healthy container as orphaned during a transient
        // docker outage at startup.
        let docker_state = state
            .containers
            .inspect_status(&row.container_name)
            .await
            .with_context(|| format!("reconcile: inspect {}", row.container_name))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match reconcile_one(&row, docker_state.as_deref()) {
            ReconcileDecision::Adopt => {
                let socket = paths::card_socket_path(&row.card_id);
                ListenerRegistry::unlink_stale(&socket);
                state.listeners.spawn(&row.card_id, socket).await;
                // Reconcile Spawning → Running for rows whose write completed
                // just before a crash that didn't kill the container itself.
                if row.status == ContainerStatus::Spawning {
                    let db = state.db.clone();
                    let id = row.card_id.clone();
                    let name = row.container_name.clone();
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        let conn = db.blocking_lock();
                        crate::db::upsert_container(
                            &conn,
                            &id,
                            &name,
                            ContainerStatus::Running,
                            now,
                        )
                    })
                    .await??;
                }
                tracing::info!(card = %row.card_id, "adopted surviving container");
            }
            ReconcileDecision::Orphan => {
                let socket = paths::card_socket_path(&row.card_id);
                ListenerRegistry::unlink_stale(&socket);
                let db = state.db.clone();
                let id = row.card_id.clone();
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let conn = db.blocking_lock();
                    crate::db::update_container_status(&conn, &id, ContainerStatus::Failed, now)
                })
                .await??;
                tracing::warn!(card = %row.card_id, "orphaned container row → Failed");
            }
        }
    }
    Ok(())
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
    async fn teardown_is_idempotent_on_missing_container() {
        if !docker_available() {
            eprintln!("skipping: docker unavailable");
            return;
        }
        let ops = DockerOps;
        // A name that definitely doesn't exist. Teardown should succeed.
        ops.teardown("reeve-definitely-not-there-xyz")
            .await
            .unwrap();
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
            "run".into(),
            "-d".into(),
            "--name".into(),
            name.into(),
            "alpine:latest".into(),
            "sleep".into(),
            "60".into(),
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
        let cards_dir = dir.path().join("cards");
        let claude = dir.path().join("claude");
        let argv = spawn_command("abc", &profile(), &cards_dir, &claude, None);
        assert_eq!(argv[0], "run");
        assert!(argv.iter().any(|s| s == "reeve-abc"));
        assert!(argv.iter().any(|s| s == "reeve.card_id=abc"));
        assert!(
            argv.iter()
                .any(|s| s.starts_with(&format!("{}:", cards_dir.display())))
        );
        assert_eq!(argv.last().unwrap(), "infinity");
        assert_eq!(argv[argv.len() - 2], "sleep");
        let image_idx = argv.iter().position(|s| s == "alpine:latest").unwrap();
        assert!(image_idx < argv.len() - 1, "image precedes cmd");
    }

    #[test]
    fn spawn_command_skips_missing_claude_mounts() {
        let dir = tempdir().unwrap();
        let cards_dir = dir.path().join("cards");
        let claude = dir.path().join("claude");
        // claude_dir does not exist
        let argv = spawn_command("abc", &profile(), &cards_dir, &claude, None);
        let claude_mounts = argv.iter().filter(|s| s.contains("/.claude/")).count();
        assert_eq!(claude_mounts, 0);
    }

    #[test]
    fn spawn_command_includes_only_existing_claude_mounts() {
        let dir = tempdir().unwrap();
        let cards_dir = dir.path().join("cards");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("skills")).unwrap();
        std::fs::write(claude.join("CLAUDE.md"), "x").unwrap();
        // settings.json deliberately missing

        let argv = spawn_command("abc", &profile(), &cards_dir, &claude, None);
        assert!(argv.iter().any(|s| s.contains("/.claude/skills:")));
        assert!(argv.iter().any(|s| s.contains("/CLAUDE.md:")));
        assert!(!argv.iter().any(|s| s.contains("settings.json")));
        // Every claude mount must be read-only — this is the security
        // property that protects the host's CC config from container writes.
        for arg in &argv {
            if arg.contains("/.claude/") {
                assert!(arg.ends_with(":ro"), "claude mount not :ro: {arg}",);
            }
        }
    }

    #[test]
    fn spawn_command_injects_api_key_only_when_some() {
        let dir = tempdir().unwrap();
        let cards_dir = dir.path().join("cards");
        let claude = dir.path().join("claude");

        let with = spawn_command("abc", &profile(), &cards_dir, &claude, Some("sk-test"));
        assert!(with.iter().any(|s| s == "ANTHROPIC_API_KEY=sk-test"));

        let without = spawn_command("abc", &profile(), &cards_dir, &claude, None);
        assert!(!without.iter().any(|s| s.starts_with("ANTHROPIC_API_KEY=")));
    }

    #[test]
    fn spawn_command_always_sets_card_id_env() {
        let dir = tempdir().unwrap();
        let argv = spawn_command(
            "card-42",
            &profile(),
            &dir.path().join("cards"),
            &dir.path().join("c"),
            None,
        );
        assert!(argv.iter().any(|s| s == "REEVE_CARD_ID=card-42"));
    }
}

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
