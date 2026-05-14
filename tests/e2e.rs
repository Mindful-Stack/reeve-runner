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

fn is_docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_devcontainer_cli_available() -> bool {
    std::process::Command::new("devcontainer")
        .arg("--version")
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

fn write_test_household(temp: &std::path::Path) {
    // Generate a minimal devcontainer.json the test household will reference.
    let dc_dir = temp.join(".devcontainer");
    std::fs::create_dir_all(&dc_dir).unwrap();
    std::fs::write(
        dc_dir.join("devcontainer.json"),
        r#"{"image":"debian:trixie-slim","remoteUser":"root","overrideCommand":true}"#,
    )
    .unwrap();

    let households = temp.join("config/reeve/households");
    std::fs::create_dir_all(&households).unwrap();
    std::fs::write(
        households.join("test.toml"),
        format!(
            "devcontainer = \"{}\"\n[[repos]]\nname = \"sample\"\npath = \"{}\"\n",
            dc_dir.join("devcontainer.json").display(),
            temp.join("host-repo").display(),
        ),
    )
    .unwrap();
}

fn init_host_repo(temp: &std::path::Path) {
    let repo = temp.join("host-repo");
    std::fs::create_dir_all(&repo).unwrap();
    fn git(repo: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invoke");
        assert!(status.success(), "git {args:?} failed");
    }
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "initial"]);
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

#[test]
fn m3_clone_and_devcontainer_happy_path() {
    if !is_docker_available() {
        eprintln!("skipping m3_clone_and_devcontainer_happy_path: docker unavailable");
        return;
    }
    if !is_devcontainer_cli_available() {
        eprintln!("skipping m3_clone_and_devcontainer_happy_path: devcontainer CLI unavailable");
        return;
    }
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());
    init_host_repo(temp.path());
    write_test_household(temp.path());

    let _daemon = DaemonGuard(
        cmd("reeved", &env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reeved"),
    );

    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists());

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

    let mut cleanup = DockerCleanup(None);

    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("reeve mv");
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Discover the actual container name via label.
    let mut container = None;
    let start = std::time::Instant::now();
    while container.is_none() && start.elapsed() < Duration::from_secs(10) {
        container = discover_container_by_label(&card_id);
        if container.is_none() {
            sleep(Duration::from_millis(200));
        }
    }
    let container = container.expect("container not discoverable within 10s");
    cleanup.0 = Some(container.clone());

    // Clone landed on disk and the dissociation severed.
    let workspace = temp.path().join("cards").join(&card_id);
    let clone_path = workspace.join("sample");
    assert!(
        clone_path.join(".git").exists(),
        "clone not at {}",
        clone_path.display()
    );
    let alternates = clone_path.join(".git/objects/info/alternates");
    assert!(!alternates.exists(), "clone not dissociated");

    // Per-card socket up.
    let card_socket = temp
        .path()
        .join("reeve/cards")
        .join(format!("{card_id}.sock"));
    assert!(card_socket.exists());

    // Copy reeve binary into container, run mv-self review.
    let reeve_bin = target_dir().join("reeve");
    let status = Command::new("docker")
        .args([
            "cp",
            reeve_bin.to_str().unwrap(),
            &format!("{container}:/usr/local/bin/reeve"),
        ])
        .status()
        .expect("docker cp reeve");
    assert!(status.success());

    // `docker exec -e REEVE_CARD_ID=…` is a workaround: devcontainer CLI's
    // `--remote-env` at up-time doesn't propagate to subsequent exec sessions.
    // The proper fix (M3-F5) is to inject `containerEnv` via a synthesized
    // --override-config devcontainer.json so the env var is set on the
    // container itself. Until then, tests + manual smokes set it at exec.
    let out = Command::new("docker")
        .args([
            "exec",
            "-e",
            &format!("REEVE_CARD_ID={card_id}"),
            &container,
            "/usr/local/bin/reeve",
            "mv-self",
            "review",
        ])
        .output()
        .expect("docker exec mv-self");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Container still running (Review keeps warm).
    assert_eq!(
        docker_inspect_status(&container).as_deref(),
        Some("running")
    );

    // mv → done tears it down.
    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "done"])
        .output()
        .expect("reeve mv done");
    assert!(out.status.success());

    let start = std::time::Instant::now();
    while docker_inspect_status(&container).is_some() && start.elapsed() < Duration::from_secs(10) {
        sleep(Duration::from_millis(100));
    }
    assert!(docker_inspect_status(&container).is_none());
    assert!(!card_socket.exists());

    // Workspace persists.
    assert!(
        workspace.exists(),
        "workspace removed after Done — should persist for inspection"
    );
}

#[test]
fn m3_reconcile_after_daemon_sigkill() {
    if !is_docker_available() || !is_devcontainer_cli_available() {
        eprintln!(
            "skipping m3_reconcile_after_daemon_sigkill: docker or devcontainer CLI unavailable"
        );
        return;
    }
    build_workspace();

    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());
    init_host_repo(temp.path());
    write_test_household(temp.path());

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

    let mut cleanup = DockerCleanup(None);

    cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .status()
        .expect("reeve mv");

    let mut container = None;
    let start = std::time::Instant::now();
    while container.is_none() && start.elapsed() < Duration::from_secs(10) {
        container = discover_container_by_label(&card_id);
        if container.is_none() {
            sleep(Duration::from_millis(200));
        }
    }
    let container = container.expect("container not discoverable");
    cleanup.0 = Some(container.clone());

    let reeve_bin = target_dir().join("reeve");
    Command::new("docker")
        .args([
            "cp",
            reeve_bin.to_str().unwrap(),
            &format!("{container}:/usr/local/bin/reeve"),
        ])
        .status()
        .expect("docker cp");

    // SIGKILL bypasses graceful shutdown. Container survives.
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
    assert!(card_socket.exists(), "per-card socket not re-bound");

    // See note on the happy-path test about REEVE_CARD_ID + M3-F5.
    let out = Command::new("docker")
        .args([
            "exec",
            "-e",
            &format!("REEVE_CARD_ID={card_id}"),
            &container,
            "/usr/local/bin/reeve",
            "mv-self",
            "review",
        ])
        .output()
        .expect("docker exec");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
