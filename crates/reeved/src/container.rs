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
