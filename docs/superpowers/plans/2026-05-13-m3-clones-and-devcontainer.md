# Reeve M3: Per-card clones + `devcontainer up` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a card moves to `Implementing`, clone the household's declared host repos into `~/reeve/cards/<card-id>/<repo>/` via `git clone --reference --dissociate`, then spawn the container via `devcontainer up` against a household-supplied devcontainer.json. The container's actual name is discovered post-spawn from a `reeve.card_id` label, replacing M2's deterministic `reeve-<id>` naming.

**Architecture:** A new `clones` module owns the git-side topology behind a `GitOps` trait (mockable for unit tests, real-git for integration tests). `container.rs` swaps `spawn_command` for `devcontainer_up_command` and adds `discover_container_name`. `HouseholdProfile` loses `image`/`cmd` and gains `devcontainer: PathBuf` + `repos: Vec<RepoSpec>`. `effects::ensure_running` orchestrates: precheck → workspace dir → clones → devcontainer up → discover name → record Running → spawn listener.

**Tech Stack:** Rust 2024 edition, tokio for async + subprocess, async-trait for trait objects. Runtime deps: `git` (already used implicitly in M1/M2 dev workflow), `docker` (M2), `devcontainer` CLI (new — `npm install -g @devcontainers/cli`).

**Spec:** `docs/superpowers/specs/2026-05-13-m3-clones-and-devcontainer-design.md`

---

## File structure (created or modified)

```
reeve/
├── crates/
│   ├── reeve-protocol/
│   │   └── src/lib.rs              # add: cards_workspace_dir, card_workspace_path, REEVE_CARDS_DIR_ENV
│   ├── reeved/
│   │   ├── src/
│   │   │   ├── main.rs             # add: mod clones
│   │   │   ├── clones.rs           # NEW: GitOps trait + clone_repo + clone_all_repos
│   │   │   ├── container.rs        # add: devcontainer_up_command + discover_container_name; drop spawn_command (Task 5)
│   │   │   ├── effects.rs          # rewrite ensure_running for clones + devcontainer (Task 5)
│   │   │   ├── handlers.rs         # extend precheck for devcontainer.json + repo paths (Task 5)
│   │   │   └── household.rs        # replace fields (Task 5)
│   │   └── Cargo.toml              # no changes
│   └── reeve/                       # no changes
└── tests/
    └── e2e.rs                      # rewrite m2_container_lifecycle_happy_path + m2_reconcile_after_daemon_sigkill (Task 5)
```

The 5-task layout keeps each commit independently buildable through Task 4, then bundles the breaking schema change with its dependents (effects rewrite + handler precheck + e2e rewrite) in Task 5 — there is no in-tree intermediate state that compiles cleanly between dropping M2's `image`/`cmd` and adopting M3's `devcontainer`+`repos`.

---

## Task 1: Protocol path helpers for the cards workspace

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Write the failing tests first**

In `crates/reeve-protocol/src/lib.rs`, append inside the existing `#[cfg(test)] mod tests` block (just before its closing `}`):

```rust
    #[test]
    fn cards_workspace_dir_uses_reeve_cards_dir_env() {
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
        let p = paths::card_workspace_path("abc-123");
        assert!(p.ends_with("abc-123"));
        assert_eq!(p.parent().unwrap(), paths::cards_workspace_dir());
    }
```

**Note on `unsafe`:** Rust 2024 edition requires `unsafe` around `std::env::set_var` / `remove_var` because the underlying glibc call is unsound under thread contention. Tests run single-threaded for env vars via `--test-threads=1` on the protocol crate if needed; this branch hasn't seen flakes there, so the unsafe blocks are sufficient.

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p reeve-protocol`
Expected: compile errors — `cards_workspace_dir` and `card_workspace_path` not defined.

- [ ] **Step 3: Add the path helpers**

In `crates/reeve-protocol/src/lib.rs`, inside the existing `pub mod paths { ... }` block, after the `card_socket_path` function (the one Task 2 of M2 added), append:

```rust
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
```

- [ ] **Step 4: Run tests; expect pass**

Run: `cargo test -p reeve-protocol`
Expected: all tests pass (existing 19 + 3 new = 22).

Also run: `cargo test --workspace`
Expected: 85 tests (was 82 on main + 3 new here).

- [ ] **Step 5: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(m3): cards_workspace_dir + card_workspace_path path helpers"
```

---

## Task 2: `clones` module — GitOps trait + clone_repo + clone_all_repos

**Files:**
- Create: `crates/reeved/src/clones.rs`
- Modify: `crates/reeved/src/main.rs` (add `mod clones;`)

This task introduces the abstraction. The trait lets unit tests assert which git commands would have been issued without running real git; Task 3 adds integration tests that hit real git.

- [ ] **Step 1: Create `crates/reeved/src/clones.rs`**

```rust
//! Per-card repo clones. Implements the `--reference --dissociate` topology
//! from the parent design: `git clone --reference <host> --dissociate <host>
//! <card-workspace>/<repo-name>/`. The clone borrows objects from the host at
//! clone time (cheap) then severs the dependency so the card survives
//! host-side `git gc --prune=now` or rebase drops.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tracing::warn;

/// Spec for one repo to clone into a card workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpec {
    pub name: String,
    pub path: PathBuf,
}

/// Abstract git-subprocess backend. The trait exists so unit tests can mock
/// out the subprocess and assert on the argv that would have been issued.
#[async_trait]
pub trait GitOps: Send + Sync {
    /// Run `git clone --reference <source> --dissociate <source> <dest>`.
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()>;
}

/// Convenience handle used by the rest of the daemon.
pub type GitOpsHandle = Arc<dyn GitOps>;

/// Real implementation. Shells out to `git` via tokio.
pub struct RealGitOps;

#[async_trait]
impl GitOps for RealGitOps {
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()> {
        let output = Command::new("git")
            .args([
                "clone",
                "--reference",
                source.to_str().context("source path not utf-8")?,
                "--dissociate",
                source.to_str().context("source path not utf-8")?,
                dest.to_str().context("dest path not utf-8")?,
            ])
            .output()
            .await
            .context("exec git clone")?;
        if !output.status.success() {
            anyhow::bail!(
                "git clone failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}

/// Clone every repo in `repos` into `<workspace>/<repo.name>/` serially.
/// On any clone failure, removes the partial workspace and returns the error.
/// Caller is responsible for creating the workspace dir before this call.
pub async fn clone_all_repos(
    ops: &dyn GitOps,
    workspace: &Path,
    repos: &[RepoSpec],
) -> Result<()> {
    for repo in repos {
        let dest = workspace.join(&repo.name);
        if let Err(e) = ops.clone(&repo.path, &dest).await {
            // Best-effort cleanup of the partial workspace.
            if let Err(rm_err) = std::fs::remove_dir_all(workspace) {
                warn!(
                    "failed to clean partial workspace {}: {rm_err:#}",
                    workspace.display()
                );
            }
            return Err(e.context(format!("clone repo '{}'", repo.name)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Records every `clone` call. Configurable failure on the Nth call.
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fail_after: Option<usize>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self { calls: Mutex::new(vec![]), fail_after: None }
        }

        fn failing_after(n: usize) -> Self {
            Self { calls: Mutex::new(vec![]), fail_after: Some(n) }
        }
    }

    #[async_trait]
    impl GitOps for MockGitOps {
        async fn clone(&self, source: &Path, dest: &Path) -> Result<()> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((source.to_path_buf(), dest.to_path_buf()));
            if let Some(n) = self.fail_after {
                if calls.len() > n {
                    anyhow::bail!("simulated clone failure");
                }
            }
            // Pretend the clone succeeded by creating the dest dir.
            std::fs::create_dir_all(dest).unwrap();
            Ok(())
        }
    }

    fn repo(name: &str, path: &Path) -> RepoSpec {
        RepoSpec { name: name.into(), path: path.to_path_buf() }
    }

    #[tokio::test]
    async fn clone_all_repos_issues_one_clone_per_repo() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::new();
        let host_a = dir.path().join("host-a");
        let host_b = dir.path().join("host-b");
        clone_all_repos(
            &ops,
            &workspace,
            &[repo("a", &host_a), repo("b", &host_b)],
        )
        .await
        .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, host_a);
        assert_eq!(calls[0].1, workspace.join("a"));
        assert_eq!(calls[1].0, host_b);
        assert_eq!(calls[1].1, workspace.join("b"));
    }

    #[tokio::test]
    async fn clone_all_repos_with_empty_repos_is_noop() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::new();
        clone_all_repos(&ops, &workspace, &[]).await.unwrap();
        assert_eq!(ops.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn clone_all_repos_removes_workspace_on_failure() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::failing_after(1);
        let host_a = dir.path().join("host-a");
        let host_b = dir.path().join("host-b");
        let result = clone_all_repos(
            &ops,
            &workspace,
            &[repo("a", &host_a), repo("b", &host_b)],
        )
        .await;
        assert!(result.is_err());
        assert!(
            !workspace.exists(),
            "workspace should be removed after clone failure",
        );
    }

    #[tokio::test]
    async fn clone_all_repos_serial_order_preserved() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::new();
        let host_a = dir.path().join("a");
        let host_b = dir.path().join("b");
        let host_c = dir.path().join("c");
        clone_all_repos(
            &ops,
            &workspace,
            &[
                repo("a", &host_a),
                repo("b", &host_b),
                repo("c", &host_c),
            ],
        )
        .await
        .unwrap();
        let calls = ops.calls.lock().unwrap();
        let names: Vec<&str> = calls.iter().map(|(s, _)| s.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/reeved/src/main.rs`. The module list should include `mod clones;` in alphabetical order between `db` (or wherever c-prefixed mods land) and the next mod. The current list (after M2) is:

```rust
mod container;
mod db;
mod effects;
mod handlers;
mod household;
mod lifecycle;
mod listeners;
mod server;
mod transitions;
```

Insert `mod clones;` between `container` and `db`:

```rust
mod clones;
mod container;
mod db;
mod effects;
mod handlers;
mod household;
mod lifecycle;
mod listeners;
mod server;
mod transitions;
```

- [ ] **Step 3: Run tests; expect pass**

Run: `cargo test -p reeved clones`
Expected: 4 tests pass.

Run: `cargo test --workspace`
Expected: 89 total tests (85 from Task 1 + 4 here).

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/src/clones.rs crates/reeved/src/main.rs
git commit -m "feat(m3): clones module with GitOps trait, clone_all_repos serial orchestrator"
```

---

## Task 3: Real-git integration tests for `clones.rs`

**Files:**
- Modify: `crates/reeved/src/clones.rs` (add integration_tests module)

This task verifies that `RealGitOps` actually produces a dissociated clone against real git. Requires `git` in `$PATH`, which every dev environment has.

- [ ] **Step 1: Append the integration test module**

At the bottom of `crates/reeved/src/clones.rs`, after the existing `mod tests` block, append:

```rust
#[cfg(test)]
mod integration_tests {
    //! Real-git tests. Require `git` in $PATH. Hit the filesystem.
    use super::*;
    use tempfile::tempdir;

    fn git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invoke");
        assert!(status.success(), "git {args:?} failed in {}", repo.display());
    }

    /// Initialise a host repo with one commit. Returns the repo path.
    fn host_repo(dir: &Path, name: &str) -> PathBuf {
        let repo = dir.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "test"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        repo
    }

    #[tokio::test]
    async fn real_clone_produces_dissociated_clone() {
        let dir = tempdir().unwrap();
        let source = host_repo(dir.path(), "host");
        let dest = dir.path().join("dest");

        let ops = RealGitOps;
        ops.clone(&source, &dest).await.unwrap();

        // Dest exists and has a .git directory.
        assert!(dest.join(".git").exists());
        // `objects/info/alternates` must NOT exist — that's the file
        // --dissociate is supposed to remove. Its absence is the proof
        // the dependency on the host repo was severed.
        let alternates = dest.join(".git/objects/info/alternates");
        assert!(
            !alternates.exists(),
            "alternates file present — clone was not dissociated",
        );
        // git log should show the commit we created in the host.
        let log = std::process::Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(&dest)
            .output()
            .unwrap();
        assert!(log.status.success());
        let stdout = String::from_utf8_lossy(&log.stdout);
        assert!(stdout.contains("initial"), "unexpected log: {stdout}");
    }

    #[tokio::test]
    async fn real_clone_two_repos_via_clone_all_repos() {
        let dir = tempdir().unwrap();
        let a = host_repo(dir.path(), "host-a");
        let b = host_repo(dir.path(), "host-b");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let ops = RealGitOps;
        clone_all_repos(
            &ops,
            &workspace,
            &[
                RepoSpec { name: "a".into(), path: a },
                RepoSpec { name: "b".into(), path: b },
            ],
        )
        .await
        .unwrap();

        assert!(workspace.join("a/.git").exists());
        assert!(workspace.join("b/.git").exists());
    }

    #[tokio::test]
    async fn real_clone_fails_when_source_is_not_a_repo() {
        let dir = tempdir().unwrap();
        let not_a_repo = dir.path().join("garbage");
        std::fs::create_dir_all(&not_a_repo).unwrap();
        let dest = dir.path().join("dest");

        let ops = RealGitOps;
        let result = ops.clone(&not_a_repo, &dest).await;
        assert!(result.is_err(), "clone of non-repo should fail");
    }
}
```

- [ ] **Step 2: Run; expect pass**

Run: `cargo test -p reeved clones::integration_tests`
Expected: 3 tests pass (git must be installed).

Run: `cargo test --workspace`
Expected: 92 total tests (89 from Task 2 + 3 here).

- [ ] **Step 3: Commit**

```bash
git add crates/reeved/src/clones.rs
git commit -m "test(m3): real-git integration tests for clone dissociation + clone_all_repos"
```

---

## Task 4: `container.rs` — devcontainer_up_command + trait-method discovery

**Files:**
- Modify: `crates/reeved/src/container.rs`

This task adds the new helpers ALONGSIDE the existing `spawn_command`. Task 5 drops `spawn_command` once all callers are migrated.

**Important:** `discover_container_name` is added as a **method on the `ContainerOps` trait** (not a free fn that shells directly to docker). This lets unit tests mock it via `NoopOps` and avoid hitting the 2-second discovery timeout when no real container exists. The free `parse_discovered_name` stays — it's the pure parser, used by the live `DockerOps` impl.

- [ ] **Step 1: Append new helpers to `crates/reeved/src/container.rs`**

After the existing `spawn_command` function and before the `use anyhow::...` line for the trait section (look for `pub trait ContainerOps`), append the new helpers. Or place them after the `ContainerOps` impl block — the order doesn't matter for compilation, but placing them next to `ContainerOps` is cleaner since they form the new spawn API. Specifically, after `pub type ContainerOpsHandle = Arc<dyn ContainerOps>;` (currently around line 177), insert:

```rust
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
        (claude_dir.join("settings.json"), "/home/agent/.claude/settings.json"),
        (claude_dir.join("CLAUDE.md"), "/home/agent/.claude/CLAUDE.md"),
    ];
    for (host, container_target) in ro_mounts {
        if host.exists() {
            args.push("--mount".into());
            args.push(format!(
                "type=bind,source={},target={},readonly",
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
```

Also extend the `ContainerOps` trait with a `discover_container_name` method. Find the existing trait definition (around line 78) and add a third method:

```rust
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
}
```

Implement it on `DockerOps` (find the existing `impl ContainerOps for DockerOps` block):

```rust
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
```

Add a `use anyhow::Result;` near the top of the file if not already present (it should be — Task 7 of M2 added it).

- [ ] **Step 2: Append unit tests for the new helpers**

At the bottom of the existing `#[cfg(test)] mod tests { ... }` block (the one created in Task 6 of M2), before the closing `}`, append:

```rust
    #[test]
    fn devcontainer_up_command_includes_workspace_config_label_and_mount() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        let dc = dir.path().join("devcontainer.json");
        let cards_socket = dir.path().join("sockets");
        let claude = dir.path().join("claude");

        let argv = devcontainer_up_command(
            "card-1",
            &workspace,
            &dc,
            &cards_socket,
            &claude,
            None,
        );

        assert_eq!(argv[0], "up");
        assert!(argv.iter().any(|s| s == "--workspace-folder"));
        assert!(argv.iter().any(|s| s == "--config"));
        assert!(argv.iter().any(|s| s == "reeve.card_id=card-1"));
        assert!(argv.iter().any(|s| s.starts_with("type=bind,source=") && s.contains("/var/run/reeve-cards")));
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
        assert!(argv.iter().any(|s| s.contains("/.claude/skills,")));
        assert!(argv.iter().any(|s| s.contains("/CLAUDE.md,")));
        assert!(!argv.iter().any(|s| s.contains("settings.json")));
        // Every claude mount must be readonly.
        for arg in &argv {
            if arg.contains("/.claude/") {
                assert!(arg.contains(",readonly"), "not readonly: {arg}");
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
```

- [ ] **Step 3: Run tests; expect pass**

Run: `cargo test -p reeved container::tests`
Expected: existing M2 tests pass plus 7 new ones.

Run: `cargo test --workspace`
Expected: 99 total tests (92 from Task 3 + 7 here).

- [ ] **Step 4: Commit**

```bash
git add crates/reeved/src/container.rs
git commit -m "feat(m3): devcontainer_up_command argv builder + parse_discovered_name"
```

---

## Task 5: The M3 swap — schema, ensure_running rewrite, precheck extension, e2e rewrite

**Files:**
- Modify: `crates/reeved/src/household.rs`
- Modify: `crates/reeved/src/container.rs` (drop `spawn_command` and its tests)
- Modify: `crates/reeved/src/effects.rs`
- Modify: `crates/reeved/src/handlers.rs`
- Modify: `tests/e2e.rs`

This task is the largest in M3 because the breaking schema change has no clean intermediate state — `image`/`cmd` and `devcontainer`/`repos` are mutually exclusive in production code paths. The substeps below are each independently green, but the **commit lands as one logical change**.

- [ ] **Step 1: Replace `HouseholdProfile` schema in `household.rs`**

Replace the entire contents of `crates/reeved/src/household.rs` with:

```rust
//! Household profile loader. M3 schema: devcontainer.json path + repo list.
//! Tilde expansion happens at load time. Unknown fields are tolerated so M4+
//! can extend without breaking M3 profiles.

use crate::clones::RepoSpec;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HouseholdProfile {
    /// Path to the household's devcontainer.json. Required.
    /// Tilde-expanded at load time.
    pub devcontainer: PathBuf,

    /// Repos to clone into each card's workspace. Optional; zero is allowed.
    /// Each repo's `path` is tilde-expanded at load time.
    #[serde(default)]
    pub repos: Vec<RepoSpec>,
}

// Manual Deserialize for RepoSpec to apply tilde expansion. The struct itself
// derives Deserialize in clones.rs but doesn't know about $HOME.
fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}

pub type HouseholdMap = Arc<HashMap<String, HouseholdProfile>>;

/// Load every `<name>.toml` from `dir` into a map keyed by `<name>`.
/// A missing dir returns an empty map; malformed TOML is an error.
/// Tilde expansion is applied to `devcontainer` and every `repos[].path`.
pub fn load_all(dir: &Path) -> Result<HouseholdMap> {
    let mut out = HashMap::new();
    if !dir.exists() {
        return Ok(Arc::new(out));
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .with_context(|| format!("bad filename {}", path.display()))?
            .to_string();
        let body =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let mut profile: HouseholdProfile =
            toml::from_str(&body).with_context(|| format!("parse {}", path.display()))?;
        profile.devcontainer = expand_tilde(&profile.devcontainer);
        for repo in profile.repos.iter_mut() {
            repo.path = expand_tilde(&repo.path);
        }
        out.insert(name, profile);
    }
    Ok(Arc::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_profile_with_devcontainer_and_repos() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            r#"
devcontainer = "/opt/dc/devcontainer.json"

[[repos]]
name = "backend"
path = "/srv/backend"

[[repos]]
name = "frontend"
path = "/srv/frontend"
"#,
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        assert_eq!(p.devcontainer, PathBuf::from("/opt/dc/devcontainer.json"));
        assert_eq!(p.repos.len(), 2);
        assert_eq!(p.repos[0].name, "backend");
        assert_eq!(p.repos[0].path, PathBuf::from("/srv/backend"));
        assert_eq!(p.repos[1].name, "frontend");
    }

    #[test]
    fn missing_dir_yields_empty_map() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        let map = load_all(&missing).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn ignores_non_toml_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "hello").unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "devcontainer = \"/x\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("test"));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "devcontainer = \"/x\"\nfuture_field = \"ignored\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert!(map.contains_key("test"));
    }

    #[test]
    fn malformed_toml_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("test.toml"), "devcontainer = (not valid").unwrap();
        assert!(load_all(dir.path()).is_err());
    }

    #[test]
    fn missing_devcontainer_field_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("test.toml"), "[[repos]]\nname = \"x\"\npath = \"/y\"\n").unwrap();
        let result = load_all(dir.path());
        assert!(result.is_err(), "missing devcontainer field should error");
    }

    #[test]
    fn empty_repos_is_valid() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "devcontainer = \"/x/devcontainer.json\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        assert!(p.repos.is_empty());
    }

    #[test]
    fn tilde_expands_in_devcontainer_and_repo_paths() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            r#"
devcontainer = "~/dc.json"

[[repos]]
name = "r"
path = "~/r"
"#,
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(p.devcontainer, PathBuf::from(&home).join("dc.json"));
        assert_eq!(p.repos[0].path, PathBuf::from(&home).join("r"));
    }
}
```

Also add `Deserialize` to the `RepoSpec` derive in `crates/reeved/src/clones.rs` so TOML can produce one:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RepoSpec {
    pub name: String,
    pub path: PathBuf,
}
```

- [ ] **Step 2: Drop `spawn_command` and its tests from `container.rs`**

In `crates/reeved/src/container.rs`:

1. Remove the `pub fn spawn_command(...)` function (and its doc comment).
2. Remove the `use crate::household::HouseholdProfile;` import at the top (no longer needed in this file).
3. Remove the unused `PathBuf` from the `use std::path::{Path, PathBuf};` line if it ends up unused (keep `Path`).
4. Inside the `#[cfg(test)] mod tests` block, delete the M2 unit tests for `spawn_command` (the `tests` module: `container_name_prefixes_with_reeve`, `spawn_command_includes_name_label_socket_image_and_cmd`, `spawn_command_skips_missing_claude_mounts`, `spawn_command_includes_only_existing_claude_mounts`, `spawn_command_injects_api_key_only_when_some`, `spawn_command_always_sets_card_id_env`). Keep `container_name_prefixes_with_reeve` — it's still used elsewhere.

Wait — re-check: `container_name(card_id)` is still useful as a fallback or for tests, but in M3 the actual container name is discovered, not constructed. Is `container_name` referenced anywhere else after the swap? Search:

```bash
grep -rn "container_name(" crates/reeved/src/
```

If `effects::ensure_running` no longer calls `container_name()` (it doesn't after Step 3 below — names come from discovery), then `container_name` becomes dead. Either:
- Delete it and its test, OR
- Keep it as `pub(crate)` with `#[allow(dead_code)]` for future use.

Recommendation: **delete it**. M3 names are docker-assigned; the deterministic-name concept is gone. Cleaner.

After this step, container.rs has: `ContainerOps` trait + `DockerOps` impl + `map_docker_state` + `ContainerOpsHandle` + `devcontainer_up_command` + `parse_discovered_name` + `reconcile_one` + `ReconcileDecision` + `reconcile_on_startup` + their tests.

Also remove from the top `use crate::household::HouseholdProfile;` since `spawn_command` was the only caller.

- [ ] **Step 3: Rewrite `effects::ensure_running`**

In `crates/reeved/src/effects.rs`, replace the entire `ensure_running` method on the `impl TransitionEffects for LiveEffects` block. The new body:

```rust
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        let card_id = card.id.clone();

        let profile = self
            .state
            .households
            .get(&card.household)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "household '{}' not configured (in ~/.config/reeve/households/)",
                    card.household,
                )
            })?;

        let socket = reeve_protocol::paths::card_socket_path(&card_id);
        let cards_socket_dir = reeve_protocol::paths::cards_socket_dir();
        let workspace = reeve_protocol::paths::card_workspace_path(&card_id);

        // Ensure the cards SOCKET directory exists so docker bind-mounts the
        // dir (not a fresh empty one). M2 contract.
        std::fs::create_dir_all(&cards_socket_dir)
            .with_context(|| format!("create cards socket dir {}", cards_socket_dir.display()))?;

        // If a container with our label is already running, adopt it and exit.
        // Discovery: the label is unique per card.
        if let Ok(Some(existing)) = self.state.containers.discover_container_name(&card_id).await {
            if let Some(state) = self.state.containers.inspect_status(&existing).await? {
                if state == "running" {
                    let id_for_update = card_id.clone();
                    let name_for_update = existing.clone();
                    let db = self.state.db.clone();
                    let now = now_secs();
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        let conn = db.blocking_lock();
                        db::upsert_container(
                            &conn,
                            &id_for_update,
                            &name_for_update,
                            ContainerStatus::Running,
                            now,
                        )
                    })
                    .await??;
                    self.state.listeners.spawn(&card_id, socket).await;
                    return Ok(());
                }
            }
        }

        // Create workspace dir + clone repos. Failures bubble up; caller
        // (handle_mv) has already mutated the column, so partial failure
        // leaves the card as Implementing+no-container (M2 documented gap).
        std::fs::create_dir_all(&workspace)
            .with_context(|| format!("create workspace {}", workspace.display()))?;
        crate::clones::clone_all_repos(&*self.state.git, &workspace, &profile.repos)
            .await
            .context("clone repos into card workspace")?;

        // Record Spawning before launching devcontainer. If we crash between
        // here and discovery, the row stays Spawning; reconcile marks Failed.
        // Use a synthetic name until discovery succeeds; reconcile only ever
        // queries by inspect/ps so a stale name doesn't break anything.
        let pending_name = format!("pending-{card_id}");
        let id_for_record = card_id.clone();
        let name_for_record = pending_name.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::upsert_container(
                &conn,
                &id_for_record,
                &name_for_record,
                ContainerStatus::Spawning,
                now,
            )
        })
        .await??;

        let argv = container::devcontainer_up_command(
            &card_id,
            &workspace,
            &profile.devcontainer,
            &cards_socket_dir,
            &self.state.claude_dir,
            self.state.api_key.as_deref(),
        );
        let spawn_result = self.state.containers.spawn(argv).await;

        if spawn_result.is_err() {
            let id_for_update = card_id.clone();
            let db = self.state.db.clone();
            let now = now_secs();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let conn = db.blocking_lock();
                db::update_container_status(&conn, &id_for_update, ContainerStatus::Failed, now)
            })
            .await??;
            return spawn_result;
        }

        // Discover the actual container name (devcontainer CLI picks it).
        // Poll briefly — devcontainer up may return success a moment before
        // the container is visible in docker ps.
        let discovered = discover_with_deadline(&self.state, &card_id).await?;

        // Upsert with the real name now.
        let id_for_update = card_id.clone();
        let name_for_update = discovered.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::upsert_container(
                &conn,
                &id_for_update,
                &name_for_update,
                ContainerStatus::Running,
                now,
            )
        })
        .await??;

        self.state.listeners.spawn(&card_id, socket).await;
        Ok(())
    }
```

Add this helper below the `impl TransitionEffects for LiveEffects { ... }` block, still inside `effects.rs`:

```rust
/// Poll `containers.discover_container_name` until it returns a name, with a
/// 2-second deadline. Returns `Err` if the deadline expires without a result.
/// `devcontainer up` may return success a moment before docker registers the
/// container, so this poll absorbs the race.
async fn discover_with_deadline(state: &DaemonState, card_id: &str) -> Result<String> {
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(100);
    loop {
        if let Some(name) = state.containers.discover_container_name(card_id).await? {
            return Ok(name);
        }
        if start.elapsed() >= deadline {
            anyhow::bail!("container not discoverable after devcontainer up");
        }
        tokio::time::sleep(backoff).await;
        if backoff < std::time::Duration::from_millis(400) {
            backoff *= 2;
        }
    }
}
```

Then add `pub git: GitOpsHandle,` to `DaemonState`:

```rust
#[derive(Clone)]
pub struct DaemonState {
    pub db: DbHandle,
    pub containers: ContainerOpsHandle,
    pub git: GitOpsHandle,                 // NEW in M3
    pub listeners: ListenerRegistry,
    pub households: HouseholdMap,
    pub claude_dir: PathBuf,
    pub api_key: Option<String>,
}
```

And add the import at the top of `effects.rs`:

```rust
use crate::clones::GitOpsHandle;
```

Update `crates/reeved/src/main.rs` to construct a real `GitOpsHandle` and pass it in:

```rust
    let git: clones::GitOpsHandle = Arc::new(clones::RealGitOps);
    // ... existing DaemonState construction ...
    let state = effects::DaemonState {
        db: db.clone(),
        containers,
        git,                                // NEW
        listeners: listeners.clone(),
        households,
        claude_dir,
        api_key,
    };
```

- [ ] **Step 4: Extend the precheck in `handlers.rs`**

In `crates/reeved/src/handlers.rs`, replace both precheck blocks (one in `handle_mv` around current line 131, one in `handle_mv_self` around current line 182). Both blocks currently look like:

```rust
    if new_column == Column::Implementing && !state.households.contains_key(&existing.household) {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }
```

Replace each occurrence with:

```rust
    if new_column == Column::Implementing {
        if let Err(e) = precheck_household_for_implementing(&state, &existing.household) {
            return Err(e);
        }
    }
```

Then add this helper near the bottom of the file (after `require_card` and before the test module):

```rust
fn precheck_household_for_implementing(
    state: &DaemonState,
    household_name: &str,
) -> std::result::Result<(), RpcError> {
    let profile = state.households.get(household_name).ok_or_else(|| RpcError {
        code: RpcError::INTERNAL,
        message: format!("household '{household_name}' not configured"),
    })?;

    let dc = &profile.devcontainer;
    if !dc.is_file() {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "devcontainer.json not readable at {}",
                dc.display()
            ),
        });
    }

    for repo in &profile.repos {
        let git_marker = repo.path.join(".git");
        if !git_marker.exists() {
            return Err(RpcError {
                code: RpcError::INTERNAL,
                message: format!(
                    "repo '{}' at {} is not a git repository (no .git found)",
                    repo.name,
                    repo.path.display(),
                ),
            });
        }
    }
    Ok(())
}
```

Update handler tests in `handlers.rs`:

1. `fresh_state_with_households` must produce a `DaemonState` with a `git: GitOpsHandle`. Add a `NoopGit` mock that does nothing, and extend the existing `NoopOps` to implement the new `discover_container_name` trait method:

```rust
    use crate::clones::{GitOps, GitOpsHandle};
    use std::path::Path;

    struct NoopGit;
    #[async_trait]
    impl GitOps for NoopGit {
        async fn clone(&self, _source: &Path, _dest: &Path) -> Result<()> {
            std::fs::create_dir_all(_dest).ok();
            Ok(())
        }
    }
```

Extend the existing `NoopOps` impl (already in this test module from M2) to add the new method. Find the existing `impl ContainerOps for NoopOps` block and add:

```rust
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> {
            // Test stub: return a fake name immediately so the discovery
            // poll in ensure_running doesn't hit its 2-second deadline.
            Ok(Some("test-container".to_string()))
        }
```

    fn fresh_state_with_households(map: HashMap<String, crate::household::HouseholdProfile>) -> DaemonState {
        let conn = db::open(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            git: Arc::new(NoopGit) as GitOpsHandle,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            households: Arc::new(map),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }
```

2. The existing `mv_to_implementing_with_configured_household_succeeds` test constructs a `HouseholdProfile { image, cmd }`. Rewrite it to use the new schema:

```rust
    #[tokio::test]
    async fn mv_to_implementing_with_configured_household_succeeds() {
        use crate::household::HouseholdProfile;
        let dir = tempfile::tempdir().unwrap();
        let dc = dir.path().join("dc.json");
        std::fs::write(&dc, "{}").unwrap();

        let mut households = HashMap::new();
        households.insert(
            "h".to_string(),
            HouseholdProfile {
                devcontainer: dc.clone(),
                repos: vec![],
            },
        );
        let state = fresh_state_with_households(households);

        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        // NoopOps returns Ok for spawn and Some("test-container") for
        // discover_container_name — so ensure_running runs end-to-end
        // without hitting real docker. NoopGit creates dest dirs to satisfy
        // clone_all_repos. The mv should succeed.
        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let result: MvResult = serde_json::from_value(mv_value).unwrap();
        assert_eq!(result.card.column, Column::Implementing);
    }
```

3. Add a new test for the extended precheck:

```rust
    #[tokio::test]
    async fn mv_to_implementing_without_devcontainer_file_rejects() {
        use crate::household::HouseholdProfile;
        let mut households = HashMap::new();
        households.insert(
            "h".to_string(),
            HouseholdProfile {
                devcontainer: PathBuf::from("/nonexistent/dc.json"),
                repos: vec![],
            },
        );
        let state = fresh_state_with_households(households);

        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let err = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("devcontainer.json not readable"));

        // Column unchanged.
        let ls_value = dispatch(
            RpcRequest { id: 3, method: "ls".into(), params: serde_json::json!({}) },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let ls: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls.cards[0].column, Column::Backlog);
    }
```

And another:

```rust
    #[tokio::test]
    async fn mv_to_implementing_with_repo_not_a_git_repo_rejects() {
        use crate::clones::RepoSpec;
        use crate::household::HouseholdProfile;
        let dir = tempfile::tempdir().unwrap();
        let dc = dir.path().join("dc.json");
        std::fs::write(&dc, "{}").unwrap();
        let fake_repo = dir.path().join("not-a-repo");
        std::fs::create_dir_all(&fake_repo).unwrap();

        let mut households = HashMap::new();
        households.insert(
            "h".to_string(),
            HouseholdProfile {
                devcontainer: dc.clone(),
                repos: vec![RepoSpec { name: "x".into(), path: fake_repo.clone() }],
            },
        );
        let state = fresh_state_with_households(households);

        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let err = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("is not a git repository"));
    }
```

- [ ] **Step 5: Rewrite the M2 e2e tests for the M3 flow**

In `tests/e2e.rs`:

1. Update `write_test_household` to produce the new schema. The function currently writes:

```toml
image = "debian:trixie-slim"
cmd = ["sleep", "infinity"]
```

Change it to write a devcontainer.json + reference it in the household:

```rust
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
```

2. Add a helper that initialises a host repo so the household's `[[repos]]` entry has a real target:

```rust
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
```

3. Update `isolated_env_with_config` to also set `REEVE_CARDS_DIR`:

```rust
fn isolated_env_with_config(temp: &std::path::Path) -> Vec<(String, String)> {
    vec![
        ("XDG_RUNTIME_DIR".into(), temp.to_string_lossy().into_owned()),
        ("XDG_DATA_HOME".into(), temp.join("data").to_string_lossy().into_owned()),
        ("XDG_CONFIG_HOME".into(), temp.join("config").to_string_lossy().into_owned()),
        ("REEVE_CARDS_DIR".into(), temp.join("cards").to_string_lossy().into_owned()),
    ]
}
```

4. Add a helper to check if `devcontainer` CLI is available:

```rust
fn is_devcontainer_cli_available() -> bool {
    std::process::Command::new("devcontainer")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
```

5. Replace the `m2_container_lifecycle_happy_path` and `m2_reconcile_after_daemon_sigkill` tests with M3 equivalents. The discovery query changes — there's no deterministic `reeve-<id>` name to look for. Instead, the test queries by label.

Add this helper:

```rust
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
```

Then the rewritten tests:

```rust
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
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));
    let card_id = parse_card_id_from_new_output(&String::from_utf8(out.stdout).unwrap());

    // Capture the cleanup hook BEFORE the move so a test panic still cleans up.
    let mut container_for_cleanup: Option<String> = None;
    let cleanup = scopeguard_cleanup(|| {
        if let Some(name) = &container_for_cleanup {
            docker_rm_force(name);
        }
    });
    let _cleanup = cleanup; // ensure drop runs

    let out = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output()
        .expect("reeve mv");
    assert!(out.status.success(), "{:?}", String::from_utf8_lossy(&out.stderr));

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
    container_for_cleanup = Some(container.clone());

    // Clone landed on disk and the dissociation severed.
    let workspace = temp.path().join("cards").join(&card_id);
    let clone_path = workspace.join("sample");
    assert!(clone_path.join(".git").exists(), "clone not at {}", clone_path.display());
    let alternates = clone_path.join(".git/objects/info/alternates");
    assert!(!alternates.exists(), "clone not dissociated");

    // Per-card socket up.
    let card_socket = temp.path().join("reeve/cards").join(format!("{card_id}.sock"));
    assert!(card_socket.exists());

    // Copy reeve binary into container, run mv-self review.
    let reeve_bin = target_dir().join("reeve");
    let status = Command::new("docker")
        .args(["cp", reeve_bin.to_str().unwrap(), &format!("{container}:/usr/local/bin/reeve")])
        .status()
        .expect("docker cp reeve");
    assert!(status.success());

    let out = Command::new("docker")
        .args(["exec", &container, "/usr/local/bin/reeve", "mv-self", "review"])
        .output()
        .expect("docker exec mv-self");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    // Container still running (Review keeps warm).
    assert_eq!(docker_inspect_status(&container).as_deref(), Some("running"));

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
    assert!(workspace.exists(), "workspace removed after Done — should persist for inspection");
}
```

The `scopeguard_cleanup` helper above is fake — use a `Drop`-impl struct instead. Replace it with:

```rust
struct DockerCleanup(Option<String>);
impl Drop for DockerCleanup {
    fn drop(&mut self) {
        if let Some(name) = &self.0 {
            docker_rm_force(name);
        }
    }
}
```

And update the test flow to:

```rust
    let mut cleanup = DockerCleanup(None);
    // ... after discovery ...
    cleanup.0 = Some(container.clone());
```

For the reconcile test, the rewritten flow:

```rust
#[test]
fn m3_reconcile_after_daemon_sigkill() {
    if !is_docker_available() || !is_devcontainer_cli_available() {
        eprintln!("skipping m3_reconcile_after_daemon_sigkill: docker or devcontainer CLI unavailable");
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
        if container.is_none() { sleep(Duration::from_millis(200)); }
    }
    let container = container.expect("container not discoverable");
    cleanup.0 = Some(container.clone());

    let reeve_bin = target_dir().join("reeve");
    Command::new("docker")
        .args(["cp", reeve_bin.to_str().unwrap(), &format!("{container}:/usr/local/bin/reeve")])
        .status()
        .expect("docker cp");

    // SIGKILL bypasses graceful shutdown. Container survives.
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(daemon.id() as i32),
        nix::sys::signal::SIGKILL,
    ).unwrap();
    let _ = daemon.wait();
    assert_eq!(docker_inspect_status(&container).as_deref(), Some("running"));

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

    let card_socket = temp.path().join("reeve/cards").join(format!("{card_id}.sock"));
    let mut waited = Duration::ZERO;
    while !card_socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(card_socket.exists(), "per-card socket not re-bound");

    let out = Command::new("docker")
        .args(["exec", &container, "/usr/local/bin/reeve", "mv-self", "review"])
        .output()
        .expect("docker exec");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}
```

Delete the old M2 versions of these two tests entirely. The `ContainerGuard` helper from M2 becomes unused — replace its uses (or keep it as `DockerCleanup` if you prefer that naming).

- [ ] **Step 6: Run the full test suite**

Run: `cargo test --workspace`
Expected: All M1 e2e tests pass + new clone integration tests pass + new container/effects/handler unit tests pass. The M3 e2e tests skip cleanly when docker or devcontainer CLI are unavailable (most CI hosts; pass when both are present).

Expected test count: ~99-105 (the e2e tests are a constant count of 3 — m1_walking_skeleton_round_trip + the two M3 ones — regardless of skip status).

- [ ] **Step 7: Run `cargo fmt --check` and clippy**

Run: `cargo fmt --all -- --check`
Expected: clean.

Run: `cargo clippy --workspace --all-targets`
Expected: clean (no dead-code warnings — `container_name` and `spawn_command` removed, all new symbols are referenced).

- [ ] **Step 8: Commit the swap**

```bash
git add crates/reeve-protocol/src/lib.rs \
        crates/reeved/src/clones.rs \
        crates/reeved/src/container.rs \
        crates/reeved/src/effects.rs \
        crates/reeved/src/handlers.rs \
        crates/reeved/src/household.rs \
        crates/reeved/src/main.rs \
        tests/e2e.rs
git commit -m "feat(m3): switch to devcontainer up + per-card clones; drop image/cmd"
```

---

## Definition of done

- [ ] All 5 tasks complete and committed.
- [ ] `cargo test --workspace` passes — including the new clone integration tests, the new container unit tests, the extended handler unit tests, and the two M3 e2e tests (when docker + devcontainer CLI available).
- [ ] `cargo clippy --workspace --all-targets` is warning-free.
- [ ] `cargo fmt --all -- --check` is clean.
- [ ] Manual smoke:
  1. `npm install -g @devcontainers/cli` (if not already).
  2. Pick a real repo `~/Source/foo`.
  3. Write `~/.config/reeve/households/foo.toml`:
     ```toml
     devcontainer = "~/Source/foo/.devcontainer/devcontainer.json"
     [[repos]]
     name = "foo"
     path = "~/Source/foo"
     ```
     (or any devcontainer.json that has `image` + `remoteUser`).
  4. `reeve daemon start`
  5. `reeve new --household foo --brief "smoke m3"` → note `<id>`.
  6. `reeve mv <id> implementing` → `~/reeve/cards/<id>/foo/` exists with the clone (severed: `cat .git/objects/info/alternates` reports "no such file"). `docker ps | grep reeve.card_id` shows the container.
  7. Discover container name: `docker ps --filter label=reeve.card_id=<id> --format '{{.Names}}'`.
  8. `docker exec <name> ls /workspaces/<id>/foo/` shows your repo contents.
  9. `reeve mv <id> done` → `docker ps -a | grep reeve.card_id` no longer shows the container. `~/reeve/cards/<id>/` still on disk.
  10. `reeve daemon stop`.
- [ ] Branch `feat/m3-clones-and-devcontainer` rebased on `main` and ready to merge.
