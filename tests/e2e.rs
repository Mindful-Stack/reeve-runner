//! End-to-end smoke test for the M1 walking skeleton.
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
    ]
}

fn is_docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

/// Removes `reeve-<card_id>` on drop, even if the test panics.
struct ContainerGuard(String);
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        docker_rm_force(&format!("reeve-{}", self.0));
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
    ]
}

fn write_test_household(temp: &std::path::Path) {
    let dir = temp.join("config/reeve/households");
    std::fs::create_dir_all(&dir).unwrap();
    // Use a glibc-based image whose glibc version is >= the host's so the
    // host-built reeve binary can be docker-cp'd in and executed inside the
    // container for the in-container mv-self tests.
    std::fs::write(
        dir.join("test.toml"),
        "image = \"debian:trixie-slim\"\ncmd = [\"sleep\", \"infinity\"]\n",
    )
    .unwrap();
}

fn wait_for_container(name: &str, expect_running: bool, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        match docker_inspect_status(name) {
            Some(s) if s == "running" && expect_running => return true,
            None if !expect_running => return true,
            _ => {}
        }
        sleep(Duration::from_millis(100));
    }
    false
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

#[test]
fn m2_container_lifecycle_happy_path() {
    if !is_docker_available() {
        eprintln!("skipping m2_container_lifecycle_happy_path: docker unavailable");
        return;
    }
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());
    write_test_household(temp.path());

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
    assert!(socket.exists());

    // new card
    let out = cmd("reeve", &env)
        .args(["new", "--household", "test", "--brief", "smoke"])
        .output()
        .expect("reeve new");
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let card_id = parse_card_id_from_new_output(&String::from_utf8(out.stdout).unwrap());
    let _guard = ContainerGuard(card_id.clone());

    // mv → implementing
    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("reeve mv");
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    let container = format!("reeve-{card_id}");
    assert!(
        wait_for_container(&container, true, Duration::from_secs(10)),
        "container {container} did not reach running within 10s",
    );

    // Per-card socket should now exist.
    let card_socket = temp
        .path()
        .join("reeve/cards")
        .join(format!("{card_id}.sock"));
    assert!(
        card_socket.exists(),
        "per-card socket missing: {}",
        card_socket.display()
    );

    // Copy the reeve binary into the container so the in-container CLI exists.
    let reeve_bin = target_dir().join("reeve");
    let status = Command::new("docker")
        .args([
            "cp",
            reeve_bin.to_str().unwrap(),
            &format!("{container}:/usr/local/bin/reeve"),
        ])
        .status()
        .expect("docker cp reeve");
    assert!(status.success(), "docker cp failed");

    // From inside the container: mv-self review.
    let out = Command::new("docker")
        .args([
            "exec",
            &container,
            "/usr/local/bin/reeve",
            "mv-self",
            "review",
        ])
        .output()
        .expect("docker exec mv-self");
    assert!(
        out.status.success(),
        "docker exec mv-self failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Container should still be running (Review keeps the container warm).
    assert_eq!(
        docker_inspect_status(&container).as_deref(),
        Some("running")
    );

    // mv → done tears it down.
    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "done"])
        .output()
        .expect("reeve mv done");
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        wait_for_container(&container, false, Duration::from_secs(10)),
        "container {container} still present after Done",
    );
    assert!(
        !card_socket.exists(),
        "per-card socket lingered after teardown"
    );
}

#[test]
fn m2_reconcile_after_daemon_sigkill() {
    if !is_docker_available() {
        eprintln!("skipping m2_reconcile_after_daemon_sigkill: docker unavailable");
        return;
    }
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());
    write_test_household(temp.path());

    // First daemon session: create card, move to Implementing.
    let mut daemon = cmd("reeved", &env)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reeved");

    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists());

    let out = cmd("reeve", &env)
        .args(["new", "--household", "test", "--brief", "reconcile"])
        .output()
        .expect("reeve new");
    let card_id = parse_card_id_from_new_output(&String::from_utf8(out.stdout).unwrap());
    let _guard = ContainerGuard(card_id.clone());

    cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .status()
        .expect("reeve mv");

    let container = format!("reeve-{card_id}");
    assert!(wait_for_container(
        &container,
        true,
        Duration::from_secs(10)
    ));

    let reeve_bin = target_dir().join("reeve");
    Command::new("docker")
        .args([
            "cp",
            reeve_bin.to_str().unwrap(),
            &format!("{container}:/usr/local/bin/reeve"),
        ])
        .status()
        .expect("docker cp");

    // SIGKILL bypasses graceful shutdown. Container should survive.
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(daemon.id() as i32),
        nix::sys::signal::SIGKILL,
    )
    .unwrap();
    let _ = daemon.wait();
    assert_eq!(
        docker_inspect_status(&container).as_deref(),
        Some("running")
    );

    // Restart daemon — reconcile should re-bind the per-card socket.
    let _daemon2 = DaemonGuard(
        cmd("reeved", &env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reeved (2)"),
    );

    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }

    let card_socket = temp
        .path()
        .join("reeve/cards")
        .join(format!("{card_id}.sock"));
    let mut waited = Duration::ZERO;
    while !card_socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(
        card_socket.exists(),
        "per-card socket not re-bound after restart"
    );

    // In-container mv-self should now work again.
    let out = Command::new("docker")
        .args([
            "exec",
            &container,
            "/usr/local/bin/reeve",
            "mv-self",
            "review",
        ])
        .output()
        .expect("docker exec");
    assert!(
        out.status.success(),
        "in-container mv-self after restart: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
