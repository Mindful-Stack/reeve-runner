//! Docker subprocess wrapper. The pure parts (argv building) live here;
//! the impure parts (shelling out, parsing output) are in `DockerOps`.

use std::sync::Arc;

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
    /// Returns the docker-assigned container name for the card tagged
    /// `reeve.card_id=<card-id>`. `Ok(None)` when no container is found yet
    /// (caller polls). `Err` when the query itself fails.
    async fn discover_container_name(&self, card_id: &str) -> Result<Option<String>>;

    /// Start a detached tmux session inside an existing running container.
    /// Shells `docker exec -u vscode <container> tmux new-session -d -s
    /// <session> -- bash -lc <command>`. The vscode user is hardcoded
    /// because the M3.5 smoke confirmed it matches the Microsoft devcontainer
    /// base image's default remoteUser; per-household overrides are M6 work.
    /// Errors propagate from the underlying `docker exec`.
    async fn exec_in_tmux(
        &self,
        container_name: &str,
        session_name: &str,
        command: &str,
    ) -> Result<()>;
}

/// Convenience handle used by the rest of the daemon.
pub type ContainerOpsHandle = Arc<dyn ContainerOps>;

/// Real Docker implementation. Shells out to `docker` via tokio.
pub(crate) struct DockerOps;

#[async_trait]
impl ContainerOps for DockerOps {
    async fn spawn(&self, argv: Vec<String>) -> Result<()> {
        // M3 swap: the argv comes from container::devcontainer_up_command,
        // so the subprocess is `devcontainer up …` (not `docker run …`).
        // Teardown / inspect / discover still go through docker because the
        // devcontainer CLI doesn't expose equivalents.
        let output = Command::new("devcontainer")
            .args(&argv)
            .output()
            .await
            .context("exec devcontainer up")?;
        if !output.status.success() {
            anyhow::bail!(
                "devcontainer up failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        // Surface devcontainer up's output even on success — otherwise
        // postCreateCommand logs (and any silent setup issues) are invisible.
        tracing::info!(
            "devcontainer up succeeded; stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
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

    async fn discover_container_name(&self, card_id: &str) -> Result<Option<String>> {
        let label = format!("label=reeve.card_id={card_id}");
        let output = Command::new("docker")
            .args(["ps", "--filter", &label, "--format", "{{.Names}}"])
            .output()
            .await
            .context("exec docker ps for container discovery")?;
        if !output.status.success() {
            anyhow::bail!(
                "docker ps failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        parse_discovered_name(&String::from_utf8_lossy(&output.stdout))
    }

    async fn exec_in_tmux(
        &self,
        container_name: &str,
        session_name: &str,
        command: &str,
    ) -> Result<()> {
        let output = Command::new("docker")
            .args([
                "exec",
                "-u",
                "vscode",
                container_name,
                "tmux",
                "new-session",
                "-d",
                "-s",
                session_name,
                "--",
                "bash",
                "-lc",
                command,
            ])
            .output()
            .await
            .context("exec docker exec for tmux new-session")?;
        if !output.status.success() {
            anyhow::bail!(
                "docker exec tmux new-session failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}

/// Factory: the only public way to obtain a `ContainerOpsHandle` backed by
/// the real Docker subprocess. `DockerOps` itself is `pub(crate)`.
pub fn make_container_ops() -> Arc<dyn ContainerOps> {
    Arc::new(DockerOps)
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

/// Build the argv for `devcontainer` (excluding the `devcontainer` binary
/// itself) that brings up the container for `card_id`.
///
/// - `workspace` is the host-side card workspace (per-card root containing
///   one subdir per cloned repo). Devcontainer CLI bind-mounts it to
///   `/workspaces/<basename>/` by convention.
/// - `devcontainer_json` is the absolute path to the household's
///   devcontainer.json (declares image, runArgs, remoteUser, postCreate, etc).
/// - `cards_socket_dir` is bind-mounted at `IN_CONTAINER_CARDS_DIR`, matching
///   the M2 contract so in-container `reeve mv-self` keeps working.
/// - `claude_dir` mounts are read-only and skipped if absent.
/// - `api_key` is injected as `ANTHROPIC_API_KEY` when `Some`.
///
/// Container is tagged with `--id-label reeve.card_id=<card-id>` so the
/// post-spawn discovery query can find its actual docker-assigned name.
pub fn devcontainer_up_command(
    card_id: &str,
    workspace: &std::path::Path,
    devcontainer_json: &std::path::Path,
    cards_socket_dir: &std::path::Path,
    claude_dir: &std::path::Path,
    api_key: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "up".into(),
        "--workspace-folder".into(),
        workspace.display().to_string(),
        "--config".into(),
        devcontainer_json.display().to_string(),
        "--id-label".into(),
        format!("reeve.card_id={card_id}"),
        "--mount".into(),
        format!(
            "type=bind,source={},target={}",
            cards_socket_dir.display(),
            reeve_protocol::paths::IN_CONTAINER_CARDS_DIR
        ),
    ];

    let ro_mounts: [(std::path::PathBuf, &str); 3] = [
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
    for (host, container_target) in ro_mounts {
        if host.exists() {
            args.push("--mount".into());
            // NOTE (M3-F5): devcontainer CLI's --mount flag does NOT accept
            // a `readonly` field — only type/source/target/external are
            // parsed (see error msg from devcontainer 0.87). The readonly
            // enforcement we want here (spec line 84-95: ~/.claude/ MUST be
            // read-only) requires synthesizing an --override-config
            // devcontainer.json with `readonly: true` in its mounts array.
            // For M3 we ship without readonly to unblock the lifecycle;
            // M3-F5 implements the override-config path properly.
            args.push(format!(
                "type=bind,source={},target={}",
                host.display(),
                container_target,
            ));
        } else {
            tracing::warn!(
                "skipping claude mount; not present on host: {}",
                host.display()
            );
        }
    }

    if let Some(key) = api_key {
        args.push("--remote-env".into());
        args.push(format!("ANTHROPIC_API_KEY={key}"));
    }
    args.push("--remote-env".into());
    args.push(format!("REEVE_CARD_ID={card_id}"));

    args
}

/// Parse the stdout of `docker ps --filter label=reeve.card_id=<id>
/// --format '{{.Names}}'` into a single container name. Returns:
/// - `Ok(Some(name))` when exactly one match is found.
/// - `Ok(None)` when no match is found (caller polls + retries).
/// - `Err(...)` when more than one match is found (shouldn't happen).
pub fn parse_discovered_name(stdout: &str) -> Result<Option<String>> {
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    match lines.as_slice() {
        [] => Ok(None),
        [name] => Ok(Some(name.trim().to_string())),
        many => anyhow::bail!("expected one container, got {}: {many:?}", many.len()),
    }
}

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
        // M2-era test deleted: it spawned via `docker run` shaped argv, but
        // M3's DockerOps::spawn now invokes `devcontainer up`. The integration
        // path is covered by tests/e2e.rs's docker-gated tests.
        // Keep the inspect+teardown half exercising the docker subprocess.
        if !docker_available() {
            eprintln!("skipping: docker unavailable");
            return;
        }
        let ops = DockerOps;
        let name = "reeve-test-roundtrip";
        let _ = ops.teardown(name).await;

        // Use docker directly to set up a sentinel container (we're testing
        // teardown + inspect, not spawn).
        let status = std::process::Command::new("docker")
            .args(["run", "-d", "--name", name, "alpine:latest", "sleep", "60"])
            .status()
            .unwrap();
        assert!(status.success());

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

    #[tokio::test]
    async fn docker_ops_exec_in_tmux_argv_shape() {
        if !docker_available() {
            eprintln!("skip: docker");
            return;
        }
        // This is a structural test that just verifies DockerOps::exec_in_tmux
        // can be called and that on a missing container it returns a clear
        // error. The full docker-side behavior is covered by the e2e test.
        let ops = DockerOps;
        let result = ops
            .exec_in_tmux("nonexistent-container-xyz", "reeve", "true")
            .await;
        assert!(
            result.is_err(),
            "exec_in_tmux into missing container should error"
        );
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.to_lowercase().contains("no such container")
                || msg.to_lowercase().contains("not found")
                || msg.contains("docker exec"),
            "error should mention docker / not-found / missing container: {msg}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn devcontainer_up_command_includes_workspace_config_label_and_mount() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        let dc = dir.path().join("devcontainer.json");
        let cards_socket = dir.path().join("sockets");
        let claude = dir.path().join("claude");

        let argv = devcontainer_up_command("card-1", &workspace, &dc, &cards_socket, &claude, None);

        assert_eq!(argv[0], "up");
        assert!(argv.iter().any(|s| s == "--workspace-folder"));
        assert!(argv.iter().any(|s| s == "--config"));
        assert!(argv.iter().any(|s| s == "reeve.card_id=card-1"));
        assert!(
            argv.iter()
                .any(|s| s.starts_with("type=bind,source=") && s.contains("/var/run/reeve-cards"))
        );
        assert!(argv.iter().any(|s| s == "REEVE_CARD_ID=card-1"));
    }

    #[test]
    fn devcontainer_up_command_skips_missing_claude_mounts() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("c");
        let dc = dir.path().join("dc.json");
        let socket = dir.path().join("s");
        let claude = dir.path().join("claude"); // does not exist

        let argv = devcontainer_up_command("c", &workspace, &dc, &socket, &claude, None);
        let claude_mounts = argv.iter().filter(|s| s.contains("/.claude/")).count();
        assert_eq!(claude_mounts, 0);
    }

    #[test]
    fn devcontainer_up_command_only_includes_existing_claude_mounts() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("c");
        let dc = dir.path().join("dc.json");
        let socket = dir.path().join("s");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("skills")).unwrap();
        std::fs::write(claude.join("CLAUDE.md"), "x").unwrap();
        // settings.json deliberately missing

        let argv = devcontainer_up_command("c", &workspace, &dc, &socket, &claude, None);
        assert!(argv.iter().any(|s| s.contains("/.claude/skills")));
        assert!(argv.iter().any(|s| s.contains("/CLAUDE.md")));
        assert!(!argv.iter().any(|s| s.contains("settings.json")));
        // M3 ships claude mounts without `,readonly` because devcontainer
        // CLI's --mount flag doesn't parse it (M3-F5 follow-up will
        // restore readonly via an --override-config devcontainer.json).
        // Pin that we currently produce a 3-field mount per claude path.
        for arg in &argv {
            if arg.contains("/.claude/") {
                assert!(arg.starts_with("type=bind,source="), "shape: {arg}");
                assert!(
                    !arg.contains(",readonly"),
                    "should not have readonly (M3-F5): {arg}"
                );
            }
        }
    }

    #[test]
    fn devcontainer_up_command_injects_api_key_only_when_some() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("c");
        let dc = dir.path().join("dc.json");
        let socket = dir.path().join("s");
        let claude = dir.path().join("claude");

        let with = devcontainer_up_command("c", &workspace, &dc, &socket, &claude, Some("sk-x"));
        assert!(with.iter().any(|s| s == "ANTHROPIC_API_KEY=sk-x"));

        let without = devcontainer_up_command("c", &workspace, &dc, &socket, &claude, None);
        assert!(!without.iter().any(|s| s.starts_with("ANTHROPIC_API_KEY=")));
    }

    #[test]
    fn parse_discovered_name_single_line() {
        let r = parse_discovered_name("vibrant_curie\n").unwrap();
        assert_eq!(r, Some("vibrant_curie".to_string()));
    }

    #[test]
    fn parse_discovered_name_empty_returns_none() {
        let r = parse_discovered_name("").unwrap();
        assert_eq!(r, None);
        let r = parse_discovered_name("\n\n").unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn parse_discovered_name_multi_line_is_error() {
        let r = parse_discovered_name("a\nb\n");
        assert!(r.is_err());
    }
}
