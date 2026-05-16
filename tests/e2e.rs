//! End-to-end smoke tests for Reeve.
//!
//! Spawns `reeved` as a child process, drives it through the `reeve` CLI,
//! verifies metadata round-trips correctly, and shuts the daemon down cleanly.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// SIGTERMs and waits on the daemon child even if the test panics.
/// Without this, an `assert!` failure leaks `reeved` and the next test
/// run finds an orphan owning $XDG_RUNTIME_DIR/reeved.sock.
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.0.id() as i32),
            nix::sys::signal::SIGTERM,
        );
        // server.rs has its own 5s shutdown grace; we trust that here.
        let _ = self.0.wait();
    }
}

fn target_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is tests/ — one level below the workspace root.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set under cargo");
    PathBuf::from(manifest)
        .parent()
        .expect("tests/ has a parent")
        .join("target/debug")
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
        (
            "XDG_RUNTIME_DIR".into(),
            temp.to_string_lossy().into_owned(),
        ),
        (
            "XDG_DATA_HOME".into(),
            temp.join("data").to_string_lossy().into_owned(),
        ),
        // Symmetry with isolated_env_with_config — even though the M1 smoke
        // test never reaches Implementing, point REEVE_CARDS_DIR at the temp
        // dir so any incidental workspace touch doesn't write to the user's
        // real `~/reeve/cards`.
        (
            "REEVE_CARDS_DIR".into(),
            temp.join("cards").to_string_lossy().into_owned(),
        ),
    ]
}

/// When `REEVE_TESTS_STRICT=1` is set, tool-gated tests treat a missing
/// tool as a hard failure instead of silently skip-passing. Implementers
/// and any future CI should set it; casual contributors who don't have
/// the full toolchain can leave it unset and get the legacy skip-pass.
fn require_or_skip(tool: &str, available: bool) -> bool {
    if available {
        return true;
    }
    if std::env::var("REEVE_TESTS_STRICT").is_ok() {
        panic!(
            "REEVE_TESTS_STRICT set but `{tool}` is not available — \
             install it or unset REEVE_TESTS_STRICT to skip"
        );
    }
    false
}

fn is_docker_available() -> bool {
    let ok = std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    require_or_skip("docker", ok)
}

fn is_devcontainer_cli_available() -> bool {
    let ok = std::process::Command::new("devcontainer")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    require_or_skip("devcontainer", ok)
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

fn discover_container_by_label(card_id: &str) -> Option<String> {
    let out = std::process::Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("label=reeve.card_id={card_id}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

struct DockerCleanup(Option<String>);
impl Drop for DockerCleanup {
    fn drop(&mut self) {
        if let Some(name) = &self.0 {
            docker_rm_force(name);
        }
    }
}

fn isolated_env_with_config(temp: &std::path::Path) -> Vec<(String, String)> {
    vec![
        (
            "XDG_RUNTIME_DIR".into(),
            temp.to_string_lossy().into_owned(),
        ),
        (
            "XDG_DATA_HOME".into(),
            temp.join("data").to_string_lossy().into_owned(),
        ),
        (
            "XDG_CONFIG_HOME".into(),
            temp.join("config").to_string_lossy().into_owned(),
        ),
        (
            "REEVE_CARDS_DIR".into(),
            temp.join("cards").to_string_lossy().into_owned(),
        ),
    ]
}


fn parse_card_id_from_new_output(stdout: &str) -> String {
    // "created <uuid> (Backlog)"
    stdout
        .strip_prefix("created ")
        .and_then(|s| s.split_whitespace().next())
        .map(|s| s.to_string())
        .unwrap_or_else(|| panic!("could not parse card id from: {stdout}"))
}

fn cmd(bin: &str, env: &[(String, String)]) -> Command {
    let mut c = Command::new(target_dir().join(bin));
    c.env_clear();
    if let Ok(home) = std::env::var("HOME") {
        c.env("HOME", home);
    }
    // Carry PATH so the daemon can find `docker`, `devcontainer`, and `git`.
    // POSIX execvp falls back to /usr/bin:/bin when PATH is unset (which is
    // why M2's docker tests worked at all under env_clear), but devcontainer
    // CLI is typically under a node toolchain prefix (mise, nvm, asdf) that
    // is not in the default fallback.
    if let Ok(path) = std::env::var("PATH") {
        c.env("PATH", path);
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

    // Start daemon as a child we control directly. Wrap in a guard so
    // an assertion panic still tears the daemon down.
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
    assert!(
        stdout.contains("Backlog"),
        "expected Backlog column: {stdout}"
    );

    // mv should change column (use Planning to stay container-free in the M1 smoke test).
    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "planning"])
        .output()
        .expect("run reeve mv");
    assert!(out_mv.status.success());
    let stdout = String::from_utf8(out_mv.stdout).unwrap();
    assert!(
        stdout.contains("Planning"),
        "expected Planning in mv output: {stdout}"
    );

    // ls should now show Planning.
    let out_ls2 = cmd("reeve", &env).arg("ls").output().expect("run reeve ls");
    let stdout = String::from_utf8(out_ls2.stdout).unwrap();
    assert!(
        stdout.contains("Planning"),
        "expected column updated: {stdout}"
    );

    // DaemonGuard's Drop handles SIGTERM + wait.
}

// m3_clone_and_devcontainer_happy_path and m3_reconcile_after_daemon_sigkill
// were removed in M3.5 Task 3. They tested ensure_running with the M3
// TOML-backed household (devcontainer path + repos[] clone). Those code paths
// are now behind todo!("M3.5 Task 9 ...") and will be re-tested in Task 9.
