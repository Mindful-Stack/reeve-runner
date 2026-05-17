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

// ---------------------------------------------------------------------------
// Helpers for M3.5 tests
// ---------------------------------------------------------------------------

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git invoke");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

fn build_test_template(dst: &std::path::Path) {
    std::fs::create_dir_all(dst.join(".devcontainer")).unwrap();
    // Declare "lore" as a sibling repo in the manifest but do NOT track any
    // `lore/` subdirectory inside the template. In real usage, sibling repos
    // live alongside the meta-repo and are never committed inline. If lore/
    // were committed inline here, cloning the meta-repo into the card workspace
    // would create workspace/lore/ which then conflicts when clone_all_for_card
    // tries to `git clone <sibling>` into that already-existing path.
    std::fs::write(
        dst.join("household.json"),
        r#"{
          "workspace": "template",
          "knowledge_base": "lore",
          "repos": [
            { "name": "template" },
            { "name": "lore" }
          ]
        }"#,
    )
    .unwrap();
    std::fs::write(
        dst.join(".devcontainer/devcontainer.json"),
        r#"{
  "name": "test",
  "image": "mcr.microsoft.com/devcontainers/base:ubuntu",
  "features": {
    "ghcr.io/devcontainers/features/node:1": {}
  },
  "postCreateCommand": "sudo apt-get update && sudo apt-get install -y tmux"
}"#,
    )
    .unwrap();
    std::fs::write(dst.join("CLAUDE.md"), "test workspace").unwrap();
    std::fs::write(dst.join(".gitignore"), "").unwrap();
    git_in(dst, &["init", "-b", "main", "--quiet"]);
    git_in(dst, &["config", "user.email", "t@t.com"]);
    git_in(dst, &["config", "user.name", "t"]);
    git_in(dst, &["add", "-A"]);
    git_in(dst, &["commit", "-m", "template", "--quiet"]);
}

fn init_sibling(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    git_in(path, &["init", "-b", "main", "--quiet"]);
    git_in(path, &["config", "user.email", "t@t.com"]);
    git_in(path, &["config", "user.name", "t"]);
    std::fs::write(path.join("README.md"), "hi\n").unwrap();
    git_in(path, &["add", "-A"]);
    git_in(path, &["commit", "-m", "initial", "--quiet"]);
}

fn write_manifest(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

fn commit_meta_changes(repo: &std::path::Path) {
    git_in(repo, &["config", "user.email", "t@t.com"]);
    git_in(repo, &["config", "user.name", "t"]);
    git_in(repo, &["add", "-A"]);
    git_in(repo, &["commit", "-m", "manifest update", "--quiet"]);
}

/// Create a bare git repo at `path` and return its `file://` URL.
fn init_bare_remote(path: &std::path::Path) -> String {
    let status = std::process::Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(path)
        .status()
        .expect("git invoke");
    assert!(
        status.success(),
        "git init --bare at {} failed",
        path.display()
    );
    format!("file://{}", path.display())
}

/// Add an `origin` remote to `repo` pointing at `url`.
fn add_origin(repo: &std::path::Path, url: &str) {
    git_in(repo, &["remote", "add", "origin", url]);
}

// ---------------------------------------------------------------------------
// M3.5 Task 12 — e2e: household new + → Implementing round-trip
// ---------------------------------------------------------------------------

#[test]
fn m3_5_household_new_then_implementing() {
    if !is_docker_available() {
        eprintln!("skip: docker not available");
        return;
    }
    if !is_devcontainer_cli_available() {
        eprintln!("skip: devcontainer CLI not available");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let mut env = isolated_env_with_config(temp.path());
    // The test isolates XDG_CONFIG_HOME so the daemon's internal `git commit`
    // can't find the user's ~/.config/git/config. Supply a minimal author
    // identity via env vars so the household-new initial commit succeeds.
    env.push(("GIT_AUTHOR_NAME".into(), "t".into()));
    env.push(("GIT_AUTHOR_EMAIL".into(), "t@t.com".into()));
    env.push(("GIT_COMMITTER_NAME".into(), "t".into()));
    env.push(("GIT_COMMITTER_EMAIL".into(), "t@t.com".into()));

    // Build a local template so we don't hit the network.
    let template_src = temp.path().join("template-source");
    build_test_template(&template_src);

    let dest = temp.path().join("my-household");

    // Spawn daemon.
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
    assert!(socket.exists(), "daemon socket not created within 2s");

    // Scaffold the household from our local template.
    let out = cmd("reeve", &env)
        .args([
            "household",
            "new",
            "test-household",
            "--path",
            &dest.to_string_lossy(),
            "--template",
            &format!("file://{}", template_src.display()),
        ])
        .output()
        .expect("run reeve household new");
    assert!(
        out.status.success(),
        "household new failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The template had "lore" as a sibling; initialise it next to the
    // meta-repo.  Also add an "extra" sibling and update the manifest so
    // we exercise the manifest-iteration path.
    init_sibling(&dest.join("lore"));
    init_sibling(&dest.join("extra"));
    write_manifest(
        &dest.join("household.json"),
        r#"{
          "workspace": "test-household",
          "knowledge_base": "lore",
          "repos": [
            { "name": "test-household" },
            { "name": "lore" },
            { "name": "extra" }
          ]
        }"#,
    );
    commit_meta_changes(&dest);

    // Create a card tied to the new household.
    let out_new = cmd("reeve", &env)
        .args([
            "new",
            "--household",
            "test-household",
            "--brief",
            "m3.5 smoke",
        ])
        .output()
        .expect("run reeve new");
    assert!(
        out_new.status.success(),
        "reeve new failed: {}",
        String::from_utf8_lossy(&out_new.stderr),
    );
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    // Defensive cleanup in case mv fails mid-spawn.
    let mut cleanup = DockerCleanup(None);

    // Move to Implementing — this clones repos and starts the devcontainer.
    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("run reeve mv");
    assert!(
        out_mv.status.success(),
        "reeve mv failed: {}",
        String::from_utf8_lossy(&out_mv.stderr),
    );

    // Discover and register the container for cleanup on drop.
    let container_name =
        discover_container_by_label(&card_id).expect("container not discovered by label after mv");
    cleanup.0 = Some(container_name.clone());

    // Verify the workspace was cloned correctly.
    let workspace = temp.path().join("cards").join(&card_id);
    assert!(workspace.join(".git").exists(), "meta-repo cloned");
    assert!(workspace.join("lore/.git").exists(), "lore sibling cloned");
    assert!(
        workspace.join("extra/.git").exists(),
        "extra sibling cloned"
    );
    assert!(
        workspace.join(".devcontainer/devcontainer.json").exists(),
        "devcontainer.json travelled with the meta-repo clone",
    );

    // Verify the container is running.
    let status = docker_inspect_status(&container_name).expect("container exists");
    assert_eq!(status, "running", "container should be running");

    // cleanup is dropped here, removing the container.
}

// ---------------------------------------------------------------------------
// M3.5 Task 13 — e2e: missing sibling blocks → Implementing with clear error
// ---------------------------------------------------------------------------

#[test]
fn m3_5_missing_sibling_blocks_implementing() {
    if !is_docker_available() {
        eprintln!("skip: docker not available");
        return;
    }
    if !is_devcontainer_cli_available() {
        eprintln!("skip: devcontainer CLI not available");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let mut env = isolated_env_with_config(temp.path());
    env.push(("GIT_AUTHOR_NAME".into(), "t".into()));
    env.push(("GIT_AUTHOR_EMAIL".into(), "t@t.com".into()));
    env.push(("GIT_COMMITTER_NAME".into(), "t".into()));
    env.push(("GIT_COMMITTER_EMAIL".into(), "t@t.com".into()));

    let template_src = temp.path().join("template-source");
    build_test_template(&template_src);

    let dest = temp.path().join("incomplete");

    // Spawn daemon.
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
    assert!(socket.exists(), "daemon socket not created within 2s");

    // Scaffold the household from our local template.
    let out = cmd("reeve", &env)
        .args([
            "household",
            "new",
            "incomplete",
            "--path",
            &dest.to_string_lossy(),
            "--template",
            &format!("file://{}", template_src.display()),
        ])
        .output()
        .expect("run reeve household new");
    assert!(
        out.status.success(),
        "household new failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Declare a "phantom" sibling in the manifest but do NOT create its repo.
    // "lore" is properly initialised; "phantom" is intentionally missing.
    init_sibling(&dest.join("lore"));
    write_manifest(
        &dest.join("household.json"),
        r#"{
          "workspace": "incomplete",
          "knowledge_base": "lore",
          "repos": [
            { "name": "incomplete" },
            { "name": "lore" },
            { "name": "phantom" }
          ]
        }"#,
    );
    // do NOT call init_sibling(&dest.join("phantom"))
    commit_meta_changes(&dest);

    // Create a card tied to the household.
    let out_new = cmd("reeve", &env)
        .args(["new", "--household", "incomplete", "--brief", "smoke"])
        .output()
        .expect("run reeve new");
    assert!(
        out_new.status.success(),
        "reeve new failed: {}",
        String::from_utf8_lossy(&out_new.stderr),
    );
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    // Attempt to move to Implementing — expect FAILURE due to missing phantom.
    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("run reeve mv");
    assert!(
        !out_mv.status.success(),
        "mv should have failed for missing sibling",
    );
    let stderr = String::from_utf8_lossy(&out_mv.stderr);
    let stdout = String::from_utf8_lossy(&out_mv.stdout);
    let combined = format!("{stderr}{stdout}");
    assert!(
        combined.contains("phantom") && combined.contains("not cloned"),
        "expected error mentioning phantom and 'not cloned', got: stdout={stdout:?} stderr={stderr:?}",
    );

    // Verify no container was spawned for the failed mv.
    assert!(
        discover_container_by_label(&card_id).is_none(),
        "container should NOT exist for failed mv",
    );
}

// ---------------------------------------------------------------------------
// M3.5 Task 14 — e2e: lore/ sibling lands in workspace and is accessible
//                       inside the running container
// ---------------------------------------------------------------------------

#[test]
fn m3_5_lore_subdir_lands_in_workspace() {
    if !is_docker_available() {
        eprintln!("skip: docker not available");
        return;
    }
    if !is_devcontainer_cli_available() {
        eprintln!("skip: devcontainer CLI not available");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let mut env = isolated_env_with_config(temp.path());
    env.push(("GIT_AUTHOR_NAME".into(), "t".into()));
    env.push(("GIT_AUTHOR_EMAIL".into(), "t@t.com".into()));
    env.push(("GIT_COMMITTER_NAME".into(), "t".into()));
    env.push(("GIT_COMMITTER_EMAIL".into(), "t@t.com".into()));

    let template_src = temp.path().join("template-source");
    build_test_template(&template_src);

    let dest = temp.path().join("with-kb");

    // Spawn daemon.
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
    assert!(socket.exists(), "daemon socket not created within 2s");

    // Scaffold the household from our local template.
    let out = cmd("reeve", &env)
        .args([
            "household",
            "new",
            "with-kb",
            "--path",
            &dest.to_string_lossy(),
            "--template",
            &format!("file://{}", template_src.display()),
        ])
        .output()
        .expect("run reeve household new");
    assert!(
        out.status.success(),
        "household new failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The template's household.json already declares "lore" as a sibling.
    // After `household new`, the handler rewrites the manifest so workspace
    // becomes "with-kb" and the matching repos[] entry is renamed accordingly:
    //   { "workspace": "with-kb", "knowledge_base": "lore",
    //     "repos": [{"name": "with-kb"}, {"name": "lore"}] }
    // We just need the lore repo to exist on disk so clone_all_for_card can
    // clone it, then commit any working-tree changes the init leaves behind.
    init_sibling(&dest.join("lore"));
    commit_meta_changes(&dest);

    // Create a card tied to the household.
    let out_new = cmd("reeve", &env)
        .args(["new", "--household", "with-kb", "--brief", "kb smoke"])
        .output()
        .expect("run reeve new");
    assert!(
        out_new.status.success(),
        "reeve new failed: {}",
        String::from_utf8_lossy(&out_new.stderr),
    );
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    // Move to Implementing — clones repos and starts the devcontainer.
    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("run reeve mv");
    assert!(
        out_mv.status.success(),
        "reeve mv failed: {}",
        String::from_utf8_lossy(&out_mv.stderr),
    );

    // Discover the container via label (devcontainer CLI assigns its own name).
    let container_name = discover_container_by_label(&card_id)
        .expect("container should be discoverable by label after mv to Implementing");
    let _cleanup = DockerCleanup(Some(container_name.clone()));

    // Verify lore landed on the host workspace.
    let workspace = temp.path().join("cards").join(&card_id);
    assert!(
        workspace.join("lore/.git").exists(),
        "lore sibling cloned to workspace",
    );

    // Verify lore is accessible inside the container.
    // Devcontainer CLI bind-mounts the workspace folder at
    // /workspaces/<basename> where basename is the last path component of
    // the host workspace directory — which is the card_id itself.
    let in_container_path = format!("/workspaces/{}/lore", card_id);
    let exec = std::process::Command::new("docker")
        .args(["exec", &container_name, "test", "-d", &in_container_path])
        .output()
        .expect("docker exec");
    assert!(
        exec.status.success(),
        "lore/ accessible inside container at {in_container_path}: {}",
        String::from_utf8_lossy(&exec.stderr),
    );

    // _cleanup dropped here — container removed.
}

// ---------------------------------------------------------------------------
// M4 Task 12 — e2e: fetch + push round-trip with bare-repo origins
// ---------------------------------------------------------------------------

#[test]
fn m4_fetch_and_push_round_trip() {
    if !is_docker_available() {
        eprintln!("skip: docker");
        return;
    }
    if !is_devcontainer_cli_available() {
        eprintln!("skip: devcontainer");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let mut env = isolated_env_with_config(temp.path());
    // The test isolates XDG_CONFIG_HOME so the daemon's internal `git commit`
    // can't find the user's ~/.config/git/config. Supply a minimal author
    // identity via env vars so the household-new initial commit succeeds.
    env.push(("GIT_AUTHOR_NAME".into(), "t".into()));
    env.push(("GIT_AUTHOR_EMAIL".into(), "t@t.com".into()));
    env.push(("GIT_COMMITTER_NAME".into(), "t".into()));
    env.push(("GIT_COMMITTER_EMAIL".into(), "t@t.com".into()));

    // 1. Build the host template (meta-repo + lore inline), then add a "code"
    //    sibling we'll commit through.
    let template_src = temp.path().join("template-source");
    build_test_template(&template_src);
    let dest = temp.path().join("m4-household");

    // 2. Daemon.
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
    assert!(socket.exists(), "socket not created");

    // 3. household new from the local template.
    let out = cmd("reeve", &env)
        .args([
            "household",
            "new",
            "m4-household",
            "--path",
            &dest.to_string_lossy(),
            "--template",
            &format!("file://{}", template_src.display()),
        ])
        .output()
        .expect("run reeve household new");
    assert!(
        out.status.success(),
        "household new: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 4. Populate a "lore" knowledge-base sibling and a "code" sibling repo,
    //    then wire bare-remote origins for all three repos so that `reeve push`
    //    can push every branch it finds in the namespace.
    let lore_sibling = dest.join("lore");
    init_sibling(&lore_sibling);
    let code_sibling = dest.join("code");
    init_sibling(&code_sibling);
    let bare_meta = temp.path().join("origin-meta.git");
    let bare_lore = temp.path().join("origin-lore.git");
    let bare_code = temp.path().join("origin-code.git");
    let bare_meta_url = init_bare_remote(&bare_meta);
    let bare_lore_url = init_bare_remote(&bare_lore);
    let bare_code_url = init_bare_remote(&bare_code);
    add_origin(&dest, &bare_meta_url);
    add_origin(&lore_sibling, &bare_lore_url);
    add_origin(&code_sibling, &bare_code_url);

    // 5. Update manifest to declare code as a sibling; commit.
    write_manifest(
        &dest.join("household.json"),
        &format!(
            r#"{{
      "workspace": "m4-household",
      "knowledge_base": "lore",
      "repos": [
        {{ "name": "m4-household" }},
        {{ "name": "code" }},
        {{ "name": "lore" }}
      ]
    }}"#
        ),
    );
    commit_meta_changes(&dest);

    // 6. Create card + mv to implementing.
    let out_new = cmd("reeve", &env)
        .args([
            "new",
            "--household",
            "m4-household",
            "--brief",
            "m4 round-trip",
        ])
        .output()
        .expect("run reeve new");
    assert!(
        out_new.status.success(),
        "reeve new: {}",
        String::from_utf8_lossy(&out_new.stderr)
    );
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("run reeve mv");
    assert!(
        out_mv.status.success(),
        "reeve mv: {}",
        String::from_utf8_lossy(&out_mv.stderr)
    );

    let container_name =
        discover_container_by_label(&card_id).expect("container should be discoverable by label");
    let _cleanup = DockerCleanup(Some(container_name.clone()));

    // 7. Commit inside the container as the `vscode` remoteUser, on both the
    //    meta-repo (repo zero) and the code sibling. Inline git config because
    //    the test container has no ambient identity.
    let workspace_in_container = format!("/workspaces/{card_id}");
    let exec = |args: &[&str]| -> std::process::Output {
        std::process::Command::new("docker")
            .args(["exec", "-u", "vscode", &container_name])
            .args(args)
            .output()
            .expect("docker exec")
    };

    // Meta-repo commit.
    let meta_commit = exec(&[
        "bash",
        "-lc",
        &format!(
            "set -e; cd {workspace_in_container} && \
            git -c user.email=t@t.com -c user.name=t checkout -b feat-m4-meta && \
            echo 'meta-edit' >> CLAUDE.md && \
            git -c user.email=t@t.com -c user.name=t commit -am meta-edit"
        ),
    ]);
    assert!(
        meta_commit.status.success(),
        "meta commit failed: {}",
        String::from_utf8_lossy(&meta_commit.stderr)
    );

    // Code-sibling commit.
    let code_commit = exec(&[
        "bash",
        "-lc",
        &format!(
            "set -e; cd {workspace_in_container}/code && \
            git -c user.email=t@t.com -c user.name=t checkout -b feat-m4-code && \
            echo 'hi' > smoke.txt && \
            git -c user.email=t@t.com -c user.name=t add smoke.txt && \
            git -c user.email=t@t.com -c user.name=t commit -m smoke"
        ),
    ]);
    assert!(
        code_commit.status.success(),
        "code commit failed: {}",
        String::from_utf8_lossy(&code_commit.stderr)
    );

    // 8. Wait at least one fetch tick.
    sleep(Duration::from_secs(7));

    // 9. Assert refs/reeve/<id>/* on the host.
    let meta_ref = std::process::Command::new("git")
        .args(["rev-parse", &format!("refs/reeve/{card_id}/feat-m4-meta")])
        .current_dir(&dest)
        .output()
        .unwrap();
    assert!(
        meta_ref.status.success(),
        "meta-repo namespaced ref: {}",
        String::from_utf8_lossy(&meta_ref.stderr)
    );

    let code_ref = std::process::Command::new("git")
        .args(["rev-parse", &format!("refs/reeve/{card_id}/feat-m4-code")])
        .current_dir(&code_sibling)
        .output()
        .unwrap();
    assert!(
        code_ref.status.success(),
        "code-sibling namespaced ref: {}",
        String::from_utf8_lossy(&code_ref.stderr)
    );

    // 10. reeve push.
    let out_push = cmd("reeve", &env)
        .args(["push", &card_id])
        .output()
        .expect("run reeve push");
    assert!(
        out_push.status.success(),
        "reeve push: stdout={} stderr={}",
        String::from_utf8_lossy(&out_push.stdout),
        String::from_utf8_lossy(&out_push.stderr)
    );

    // 11. Assert refs/heads/feat-m4-meta on host meta-repo AND in bare remote.
    let head_meta_host = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-meta"])
        .current_dir(&dest)
        .output()
        .unwrap();
    assert!(
        head_meta_host.status.success(),
        "host meta has refs/heads/feat-m4-meta"
    );
    let head_meta_remote = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-meta"])
        .current_dir(&bare_meta)
        .output()
        .unwrap();
    assert!(
        head_meta_remote.status.success(),
        "bare-meta has refs/heads/feat-m4-meta"
    );
    // And code sibling.
    let head_code_host = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-code"])
        .current_dir(&code_sibling)
        .output()
        .unwrap();
    assert!(
        head_code_host.status.success(),
        "host code has refs/heads/feat-m4-code"
    );
    let head_code_remote = std::process::Command::new("git")
        .args(["rev-parse", "refs/heads/feat-m4-code"])
        .current_dir(&bare_code)
        .output()
        .unwrap();
    assert!(
        head_code_remote.status.success(),
        "bare-code has refs/heads/feat-m4-code"
    );

    // 12. mv to done. Container is torn down. Namespaced refs persist.
    let out_done = cmd("reeve", &env)
        .args(["mv", &card_id, "done"])
        .output()
        .expect("run reeve mv done");
    assert!(
        out_done.status.success(),
        "reeve mv done: {}",
        String::from_utf8_lossy(&out_done.stderr)
    );

    let meta_ref_after = std::process::Command::new("git")
        .args(["rev-parse", &format!("refs/reeve/{card_id}/feat-m4-meta")])
        .current_dir(&dest)
        .output()
        .unwrap();
    assert!(
        meta_ref_after.status.success(),
        "namespaced ref should still exist after Done"
    );
}

// ---------------------------------------------------------------------------
// M5 Task 6 — e2e: attach — tmux session present in container after mv → implementing
// ---------------------------------------------------------------------------

#[test]
fn m5_attach_drops_into_running_tmux_session() {
    if !is_docker_available() {
        eprintln!("skip: docker");
        return;
    }
    if !is_devcontainer_cli_available() {
        eprintln!("skip: devcontainer");
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let mut env = isolated_env_with_config(temp.path());
    // The test isolates XDG_CONFIG_HOME so the daemon's internal `git commit`
    // can't find the user's ~/.config/git/config. Supply a minimal author
    // identity via env vars so the household-new initial commit succeeds.
    env.push(("GIT_AUTHOR_NAME".into(), "t".into()));
    env.push(("GIT_AUTHOR_EMAIL".into(), "t@t.com".into()));
    env.push(("GIT_COMMITTER_NAME".into(), "t".into()));
    env.push(("GIT_COMMITTER_EMAIL".into(), "t@t.com".into()));

    // 1. Build the template (now with tmux via postCreate per Task 6 Step 1).
    let template_src = temp.path().join("template-source");
    build_test_template(&template_src);
    let dest = temp.path().join("m5-household");

    // 2. Daemon.
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
    assert!(socket.exists(), "socket not created");

    // 3. household new.
    let out = cmd("reeve", &env)
        .args([
            "household",
            "new",
            "m5-household",
            "--path",
            &dest.to_string_lossy(),
            "--template",
            &format!("file://{}", template_src.display()),
        ])
        .output()
        .expect("run reeve household new");
    assert!(
        out.status.success(),
        "household new: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The template's household.json declares "lore" as a sibling.
    // Initialise it next to the meta-repo so clone_all_for_card can clone it.
    init_sibling(&dest.join("lore"));
    commit_meta_changes(&dest);

    // 4. Create card + mv to implementing.
    let out_new = cmd("reeve", &env)
        .args([
            "new",
            "--household",
            "m5-household",
            "--brief",
            "m5 attach test",
        ])
        .output()
        .expect("run reeve new");
    assert!(
        out_new.status.success(),
        "reeve new: {}",
        String::from_utf8_lossy(&out_new.stderr)
    );
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("run reeve mv");
    assert!(
        out_mv.status.success(),
        "reeve mv: {}",
        String::from_utf8_lossy(&out_mv.stderr)
    );

    let container_name =
        discover_container_by_label(&card_id).expect("container should be discoverable by label");
    let _cleanup = DockerCleanup(Some(container_name.clone()));

    // 5. Assert tmux session named `reeve` exists in the container.
    let tmux_ls = std::process::Command::new("docker")
        .args(["exec", "-u", "vscode", &container_name, "tmux", "ls"])
        .output()
        .expect("docker exec tmux ls");
    assert!(
        tmux_ls.status.success(),
        "tmux ls failed: stdout={} stderr={}",
        String::from_utf8_lossy(&tmux_ls.stdout),
        String::from_utf8_lossy(&tmux_ls.stderr),
    );
    let tmux_ls_stdout = String::from_utf8_lossy(&tmux_ls.stdout);
    assert!(
        tmux_ls_stdout.starts_with("reeve:"),
        "tmux ls should list 'reeve:' session, got: {tmux_ls_stdout}"
    );

    // 6. Send a marker line into the tmux session and read it back.
    let marker = "M5_MARK_FOXTROT_9001";
    let send = std::process::Command::new("docker")
        .args([
            "exec",
            "-u",
            "vscode",
            &container_name,
            "tmux",
            "send-keys",
            "-t",
            "reeve",
            &format!("echo {marker}"),
            "Enter",
        ])
        .status()
        .expect("docker exec tmux send-keys");
    assert!(send.success(), "tmux send-keys failed");

    // Tmux sometimes needs a beat to flush.
    sleep(Duration::from_millis(500));

    let capture = std::process::Command::new("docker")
        .args([
            "exec",
            "-u",
            "vscode",
            &container_name,
            "tmux",
            "capture-pane",
            "-t",
            "reeve",
            "-p",
        ])
        .output()
        .expect("docker exec tmux capture-pane");
    assert!(capture.status.success(), "tmux capture-pane failed");
    let capture_stdout = String::from_utf8_lossy(&capture.stdout);
    assert!(
        capture_stdout.contains(marker),
        "marker {marker} not found in pane: {capture_stdout}"
    );

    // 7. Tear down.
    let out_done = cmd("reeve", &env)
        .args(["mv", &card_id, "done"])
        .output()
        .expect("run reeve mv done");
    assert!(
        out_done.status.success(),
        "reeve mv done: {}",
        String::from_utf8_lossy(&out_done.stderr)
    );
}
