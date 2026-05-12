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
        // Every claude mount must be read-only — this is the security
        // property that protects the host's CC config from container writes.
        for arg in &argv {
            if arg.contains("/.claude/") {
                assert!(
                    arg.ends_with(":ro"),
                    "claude mount not :ro: {arg}",
                );
            }
        }
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
