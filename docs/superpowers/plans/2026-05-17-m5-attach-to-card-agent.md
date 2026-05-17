# M5 — `reeve attach <card>` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `reeve attach <card-id>` one command that drops the user into a persistent claude session inside the per-card container, via a daemon-managed tmux session created on `mv → Implementing`.

**Architecture:** `ensure_running` gains a tail step that starts a fixed-name `reeve` tmux session inside the container with `(claude || bash)`. A new RPC `attach` returns the container + session names; the CLI `reeve attach` then `unsafe`-Unix-`exec`s into `docker exec -it <container> tmux attach -t reeve` so the user's terminal becomes tmux directly. Tmux startup is warn-not-bail, the `|| bash` fallback covers a broken claude install.

**Tech Stack:** Rust 2024, tokio (async + sync::Mutex), `docker exec` subprocess, `tmux new-session`/`attach` subprocess, Unix `process::CommandExt::exec` for process replacement.

**Spec:** `docs/superpowers/specs/2026-05-17-m5-attach-to-card-agent-design.md`

**Deferred items (writing-plans considered, will NOT implement in M5):**

- TUI live attach (the parent design's `a` keybinding). Needs the TUI to grow process-management plumbing it doesn't have; separate milestone.
- TUI write-mutations (`new`, `mv`, household ops from the TUI). Same milestone as live attach.
- Card-id prefix matching for `reeve attach <prefix>`. Polish for M6.
- Auto-fallback in the CLI from `tmux attach` to bare `docker exec ... bash` when the session is missing. Tmux's own error message is clear enough; documented in spec.
- Restart claude on user `/exit`. If the user exits, they meant it.
- Multi-window tmux per session, configurable session name, configurable remoteUser. All M6+ polish.
- M3.5/M4 known-issues sweep. Separate stabilization milestone.

---

## File map

| Path | Responsibility | Change kind |
|---|---|---|
| `crates/reeve-protocol/src/lib.rs` | `AttachParams`, `AttachResult` | additive |
| `crates/reeved/src/container.rs` | `ContainerOps::exec_in_tmux` + `DockerOps` impl | additive |
| `crates/reeved/src/effects.rs` | tmux startup call at the tail of `ensure_running` | additive |
| `crates/reeved/src/handlers.rs` | `handle_attach` + dispatcher `"attach"` case; `NoopOps` updated | additive |
| `crates/reeved/src/fetch.rs` | `NoopOps` (in tests) updated to satisfy new trait method | mechanical |
| `crates/reeve/src/cli.rs` | `Command::Attach { id }` + handler with `unsafe` `exec` | additive |
| `tests/e2e.rs` | `m5_attach_drops_into_running_tmux_session` + tmux added to `build_test_template` | additive |

---

## Task 1: Protocol types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Append the M5 protocol types**

Open `crates/reeve-protocol/src/lib.rs`. Append after the existing `BranchPushStatus` enum (last M4 type, before `pub mod paths`):

```rust
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
```

- [ ] **Step 2: Verify the crate still builds**

Run: `cargo check -p reeve-protocol`
Expected: PASS.

- [ ] **Step 3: Run protocol tests**

Run: `cargo test -p reeve-protocol`
Expected: 22 pass (no new tests; types are consumed in later tasks).

- [ ] **Step 4: Commit**

```bash
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(m5): protocol types for attach (AttachParams, AttachResult)"
```

---

## Task 2: `ContainerOps::exec_in_tmux`

**Files:**
- Modify: `crates/reeved/src/container.rs`
- Modify: `crates/reeved/src/handlers.rs` (NoopOps in tests)
- Modify: `crates/reeved/src/fetch.rs` (NoopOps in tests)

- [ ] **Step 1: Write the failing unit test in `container.rs`**

In `crates/reeved/src/container.rs`'s existing `#[cfg(test)] mod tests` (or wherever container tests live — alongside the existing docker-gated tests). Append:

```rust
    #[tokio::test]
    async fn docker_ops_exec_in_tmux_argv_shape() {
        // This is a structural test that just verifies DockerOps::exec_in_tmux
        // can be called and that on a missing container it returns a clear
        // error. The full docker-side behavior is covered by the e2e test.
        let ops = DockerOps;
        let result = ops
            .exec_in_tmux("nonexistent-container-xyz", "reeve", "true")
            .await;
        assert!(result.is_err(), "exec_in_tmux into missing container should error");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.to_lowercase().contains("no such container")
                || msg.to_lowercase().contains("not found")
                || msg.contains("docker exec"),
            "error should mention docker / not-found / missing container: {msg}"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeved container::tests::docker_ops_exec_in_tmux_argv_shape`
Expected: FAIL with `no method named 'exec_in_tmux'`.

- [ ] **Step 3: Add `exec_in_tmux` to the `ContainerOps` trait**

In `crates/reeved/src/container.rs`, the trait at line 15. Append the method:

```rust
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
```

- [ ] **Step 4: Implement on `DockerOps`**

In the same file, in `impl ContainerOps for DockerOps`, after the existing `discover_container_name` impl, append:

```rust
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
```

- [ ] **Step 5: Add `exec_in_tmux` to `NoopOps` in `handlers.rs` tests**

In `crates/reeved/src/handlers.rs`, find the `impl ContainerOps for NoopOps` block (around line 805). Append the new method:

```rust
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> {
            Ok(())
        }
```

- [ ] **Step 6: Add `exec_in_tmux` to `NoopOps` in `fetch.rs` tests**

In `crates/reeved/src/fetch.rs`, find the `impl crate::container::ContainerOps for NoopOps` block (around line 186). Append:

```rust
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> {
            Ok(())
        }
```

- [ ] **Step 7: Run the unit test**

Run: `cargo test -p reeved container::tests::docker_ops_exec_in_tmux_argv_shape`
Expected: PASS. (Requires `docker` in `$PATH` per existing docker-gated test pattern.)

- [ ] **Step 8: Run the full reeved suite**

Run: `cargo test -p reeved`
Expected: 104 tests pass (103 + 1 new). All existing tests still pass because `NoopOps` got the new method in both test modules.

- [ ] **Step 9: Commit**

```bash
git add crates/reeved/src/container.rs crates/reeved/src/handlers.rs crates/reeved/src/fetch.rs
git commit -m "feat(m5): ContainerOps::exec_in_tmux — docker exec tmux new-session -d -s"
```

---

## Task 3: `ensure_running` tmux startup (warn-not-bail)

**Files:**
- Modify: `crates/reeved/src/effects.rs`

- [ ] **Step 1: Add the tmux startup call at the tail of `ensure_running`**

Edit `crates/reeved/src/effects.rs`. Find `ensure_running` — the path that ends with `self.state.listeners.spawn(&card_id, socket).await;` followed by `Ok(())` (NOT the adopt-running branch; that branch returns early and tmux startup must NOT happen on adoption per spec). Insert just before the final `Ok(())`:

```rust
        // M5: start the per-card claude session inside the container's tmux.
        // Defensive `|| bash` so a broken claude install doesn't lock the user out.
        // Warn-not-bail: if tmux isn't installed in the container, the column
        // transition still succeeds; the user discovers the issue when they
        // run `reeve attach`.
        let tmux_cmd = format!("cd /workspaces/{card_id} && (claude || bash)");
        if let Err(e) = self
            .state
            .containers
            .exec_in_tmux(&container_name, "reeve", &tmux_cmd)
            .await
        {
            tracing::warn!(
                "tmux session not started in {container_name}: {e:#} — \
                 `reeve attach` will fail with 'session not found'; \
                 install tmux in the container (add to devcontainer features)",
            );
        }
```

- [ ] **Step 2: Write a unit test confirming warn-not-bail**

`crates/reeved/src/effects.rs` currently has no test module — effects is tested indirectly via handlers tests and e2e. Adding a focused unit test requires importing several mocks. Pragmatic approach: skip a dedicated unit test for the warn-not-bail behavior — it's a one-line `if let Err(_) = { warn!; }`, hard to break accidentally, and is exercised end-to-end in Task 6's e2e.

If we DID want a unit test: extract `start_card_tmux(ops, name, card_id) -> Result<()>` as a free function that returns `Ok(())` even when `exec_in_tmux` errors, and unit-test that. For M5 we don't; the e2e covers it.

- [ ] **Step 3: Verify build**

Run: `cargo check -p reeved`
Expected: PASS.

- [ ] **Step 4: Run reeved tests**

Run: `cargo test -p reeved`
Expected: 104 pass (no new tests this task).

- [ ] **Step 5: Commit**

```bash
git add crates/reeved/src/effects.rs
git commit -m "feat(m5): ensure_running starts tmux session with (claude || bash) — warn-not-bail"
```

---

## Task 4: `handle_attach` + dispatcher

**Files:**
- Modify: `crates/reeved/src/handlers.rs`

- [ ] **Step 1: Write the failing unit test for the happy path**

Append to `handlers.rs`'s `#[cfg(test)] mod tests` (after the existing M4 push tests):

```rust
    use reeve_protocol::{AttachParams, AttachResult};

    /// ContainerOps test mock that returns a configured discover response.
    struct ConfigurableContainers {
        discover_response: std::sync::Mutex<Option<String>>,
    }
    impl ConfigurableContainers {
        fn new() -> Self { Self { discover_response: std::sync::Mutex::new(None) } }
        fn set_discover(&self, name: Option<String>) {
            *self.discover_response.lock().unwrap() = name;
        }
    }
    #[async_trait::async_trait]
    impl crate::container::ContainerOps for ConfigurableContainers {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> { Ok(()) }
        async fn teardown(&self, _name: &str) -> Result<()> { Ok(()) }
        async fn inspect_status(&self, _name: &str) -> Result<Option<String>> { Ok(None) }
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> {
            Ok(self.discover_response.lock().unwrap().clone())
        }
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> { Ok(()) }
    }

    fn fresh_state_with_containers(
        containers: std::sync::Arc<ConfigurableContainers>,
    ) -> DaemonState {
        let conn = crate::db::open(":memory:").unwrap();
        let db: crate::server::DbHandle = std::sync::Arc::new(tokio::sync::Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: containers as crate::container::ContainerOpsHandle,
            git: std::sync::Arc::new(NoopGit) as crate::clones::GitOpsHandle,
            listeners: crate::listeners::ListenerRegistry::new(db, std::sync::Arc::new(NoopDispatcher)),
            claude_dir: std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    #[tokio::test]
    async fn attach_returns_container_and_session_for_implementing_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        containers.set_discover(Some("happy-cool-container".into()));
        let state = fresh_state_with_containers(containers);
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-a".into(),
                    household: "h".into(),
                    brief: "a".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "card-a" }),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: AttachResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.container_name, "happy-cool-container");
        assert_eq!(result.session_name, "reeve");
        assert_eq!(result.in_container_workspace, "/workspaces/card-a");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reeved handlers::tests::attach_returns_container_and_session_for_implementing_card`
Expected: FAIL — `"attach"` method not handled by dispatcher.

- [ ] **Step 3: Add `handle_attach`**

Edit `handlers.rs`. Find the `use reeve_protocol::{...}` block at the top. Add `AttachParams, AttachResult` alphabetically.

Append the handler (alongside the other handlers, e.g. after `handle_push`):

```rust
async fn handle_attach(state: &DaemonState, params: AttachParams) -> Result<Value, RpcError> {
    // 1. Fetch the card from the DB.
    let card_id = params.id.clone();
    let db = state.db.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        crate::db::get_card(&conn, &card_id)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card '{}' not found", params.id),
    })?;

    // 2. Column check.
    if card.column != Column::Implementing {
        return Err(RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: format!(
                "card is in {}; attach only works in Implementing",
                card.column.label()
            ),
        });
    }

    // 3. Container discovery by label.
    let container_name = state
        .containers
        .discover_container_name(&card.id)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("container discovery failed: {e:#}"),
        })?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "container for card '{}' not running (try mv {} implementing again)",
                card.id, card.id
            ),
        })?;

    // 4. Return the address.
    let result = AttachResult {
        container_name,
        session_name: "reeve".to_string(),
        in_container_workspace: format!("/workspaces/{}", card.id),
    };
    serde_json::to_value(result).map_err(to_internal)
}
```

- [ ] **Step 4: Add dispatcher case for `"attach"`**

In the `dispatch` function's `match req.method.as_str()` block, add (alphabetical — likely just after `"household.show"` or after `"mv-self"`):

```rust
        "attach" => {
            let params = parse_params::<AttachParams>(req.params)?;
            handle_attach(&state, params).await
        }
```

- [ ] **Step 5: Run the happy-path test**

Run: `cargo test -p reeved handlers::tests::attach_returns_container_and_session_for_implementing_card`
Expected: PASS.

- [ ] **Step 6: Add the three failure-mode tests**

Append:

```rust
    #[tokio::test]
    async fn attach_returns_not_found_for_unknown_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        let state = fresh_state_with_containers(containers);

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "missing-card" }),
        };
        let err = dispatch(req, state, CallerContext::host()).await.unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
        assert!(err.message.contains("not found"), "msg: {}", err.message);
    }

    #[tokio::test]
    async fn attach_rejects_non_implementing_card() {
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        let state = fresh_state_with_containers(containers);
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-b".into(),
                    household: "h".into(),
                    brief: "b".into(),
                    column: Column::Backlog,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "card-b" }),
        };
        let err = dispatch(req, state, CallerContext::host()).await.unwrap_err();
        assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
        assert!(
            err.message.contains("Implementing"),
            "msg: {}", err.message
        );
    }

    #[tokio::test]
    async fn attach_errors_when_no_container_for_card() {
        // Card in Implementing but discover_container_name returns None.
        let containers = std::sync::Arc::new(ConfigurableContainers::new());
        // Don't call set_discover — defaults to None.
        let state = fresh_state_with_containers(containers);
        {
            let conn = state.db.lock().await;
            crate::db::insert_card(
                &conn,
                &Card {
                    id: "card-c".into(),
                    household: "h".into(),
                    brief: "c".into(),
                    column: Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
        }

        let req = RpcRequest {
            id: 1,
            method: "attach".into(),
            params: serde_json::json!({ "id": "card-c" }),
        };
        let err = dispatch(req, state, CallerContext::host()).await.unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("not running"), "msg: {}", err.message);
    }
```

- [ ] **Step 7: Run all four attach tests**

Run: `cargo test -p reeved handlers::tests::attach`
Expected: 4 tests PASS.

- [ ] **Step 8: Run the full reeved suite**

Run: `cargo test -p reeved`
Expected: 108 tests pass (104 + 4 new). Watch for regressions.

- [ ] **Step 9: Commit**

```bash
git add crates/reeved/src/handlers.rs
git commit -m "feat(m5): handle_attach + dispatcher — 4 tests covering happy + 3 error paths"
```

---

## Task 5: CLI `reeve attach <id>`

**Files:**
- Modify: `crates/reeve/src/cli.rs`

- [ ] **Step 1: Add the `Attach` subcommand variant**

Edit `crates/reeve/src/cli.rs`. In the existing `pub enum Command` (around line 29), add (alphabetical position — likely between an alphabetically-adjacent variant; near `Push` works since `Attach` precedes `Push`):

```rust
    /// Drop into the running claude session inside a card's container.
    /// The card must be in Implementing. Uses tmux for detach/reattach.
    /// Ctrl-B D detaches; claude keeps running.
    Attach { id: String },
```

- [ ] **Step 2: Wire dispatch in `run`**

In the `pub async fn run(cmd: Command)` match block, add:

```rust
        Command::Attach { id } => attach(id).await,
```

- [ ] **Step 3: Add imports**

In the `use reeve_protocol::{...}` block at the top, add `AttachResult` alphabetically.

- [ ] **Step 4: Add the `attach` handler function**

Append in `cli.rs` (alongside other handlers like `ls`, `new`, `mv`, `push`):

```rust
async fn attach(id: String) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "attach", serde_json::json!({ "id": id }))
        .await
        .context("calling attach")?;
    match result {
        Ok(value) => {
            let result: AttachResult = serde_json::from_value(value)?;
            println!(
                "attaching to {}:{} (ctrl-b d to detach, claude keeps running)",
                result.container_name, result.session_name
            );
            // Process-replace ourselves with `docker exec -it <container> tmux attach -t <session>`.
            // `CommandExt::exec` either replaces the process (never returns) or
            // returns the error that prevented exec from succeeding.
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new("docker")
                .args([
                    "exec",
                    "-it",
                    &result.container_name,
                    "tmux",
                    "attach",
                    "-t",
                    &result.session_name,
                ])
                .exec();
            anyhow::bail!("docker exec failed to start: {err}");
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}
```

- [ ] **Step 5: Verify the CLI builds**

Run: `cargo build -p reeve`
Expected: PASS.

- [ ] **Step 6: Verify the workspace builds**

Run: `cargo build --workspace`
Expected: PASS.

- [ ] **Step 7: Run all tests**

Run: `cargo test --workspace`
Expected: 108 reeved + 22 protocol + 5 e2e + 2 client = 137 tests pass. Task 5 adds no tests (CLI process-replacement is validated via manual smoke).

- [ ] **Step 8: Commit**

```bash
git add crates/reeve/src/cli.rs
git commit -m "feat(m5): reeve attach <id> CLI — process-replace into docker exec tmux attach"
```

After commit: `git status` must show `feat/m5-attach`. If detached, `git checkout feat/m5-attach`.

---

## Task 6: E2E test + tmux in template + workspace verification

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Extend `build_test_template` to install tmux in the container**

The current `build_test_template` in `tests/e2e.rs` writes a minimal `devcontainer.json` with image + node feature. For M5 we need tmux too. Find `build_test_template` (search for `fn build_test_template`). Modify the `devcontainer.json` body it writes to include a postCreate that installs tmux:

```rust
    std::fs::write(
        dst.join(".devcontainer/devcontainer.json"),
        r#"{
  "name": "test",
  "image": "mcr.microsoft.com/devcontainers/base:ubuntu",
  "features": {
    "ghcr.io/devcontainers/features/node:1": {}
  },
  "postCreateCommand": "apt-get update && apt-get install -y tmux"
}"#,
    ).unwrap();
```

The previous body just had `image` and (probably) features. The change: add `postCreateCommand` that runs `apt-get install -y tmux`. This runs as the postCreate user (vscode by default) which won't have apt permissions. Adjust by running as root via `sudo` (the devcontainer base image grants vscode passwordless sudo):

Actually, easier: postCreateCommand runs in the container's lifecycle context, and the devcontainer CLI typically gives postCreate root privileges via its own mechanism. Test the simpler form first:

```rust
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
    ).unwrap();
```

If `sudo` is needed and the base image provides it (it does — `mcr.microsoft.com/devcontainers/base:ubuntu` ships sudo + the vscode user in sudoers), the install succeeds. If not, the e2e will surface the issue and we adapt.

- [ ] **Step 2: Add the e2e test**

Append in `tests/e2e.rs` (after the existing M4 e2e):

```rust
#[test]
fn m5_attach_drops_into_running_tmux_session() {
    if !is_docker_available() { eprintln!("skip: docker"); return; }
    if !is_devcontainer_cli_available() { eprintln!("skip: devcontainer"); return; }

    let temp = tempfile::tempdir().unwrap();
    build_workspace();
    let env = isolated_env_with_config(temp.path());

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
        .args(["household", "new", "m5-household",
               "--path", &dest.to_string_lossy(),
               "--template", &format!("file://{}", template_src.display())])
        .output()
        .expect("run reeve household new");
    assert!(out.status.success(), "household new: {}", String::from_utf8_lossy(&out.stderr));

    // 4. Create card + mv to implementing.
    let out_new = cmd("reeve", &env)
        .args(["new", "--household", "m5-household", "--brief", "m5 attach test"])
        .output().expect("run reeve new");
    assert!(out_new.status.success(), "reeve new: {}", String::from_utf8_lossy(&out_new.stderr));
    let card_id = parse_card_id_from_new_output(&String::from_utf8_lossy(&out_new.stdout));

    let out_mv = cmd("reeve", &env)
        .args(["mv", &card_id, "implementing"])
        .output().expect("run reeve mv");
    assert!(out_mv.status.success(), "reeve mv: {}", String::from_utf8_lossy(&out_mv.stderr));

    let container_name = discover_container_by_label(&card_id)
        .expect("container should be discoverable by label");
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
        .args(["exec", "-u", "vscode", &container_name,
               "tmux", "send-keys", "-t", "reeve",
               &format!("echo {marker}"), "Enter"])
        .status()
        .expect("docker exec tmux send-keys");
    assert!(send.success(), "tmux send-keys failed");

    // Tmux sometimes needs a beat to flush.
    sleep(Duration::from_millis(500));

    let capture = std::process::Command::new("docker")
        .args(["exec", "-u", "vscode", &container_name,
               "tmux", "capture-pane", "-t", "reeve", "-p"])
        .output()
        .expect("docker exec tmux capture-pane");
    assert!(capture.status.success(), "tmux capture-pane failed");
    let capture_stdout = String::from_utf8_lossy(&capture.stdout);
    assert!(
        capture_stdout.contains(marker),
        "marker {marker} not found in pane: {capture_stdout}"
    );

    // 7. Tear down.
    let out_done = cmd("reeve", &env).args(["mv", &card_id, "done"]).output()
        .expect("run reeve mv done");
    assert!(out_done.status.success(),
        "reeve mv done: {}", String::from_utf8_lossy(&out_done.stderr));
}
```

- [ ] **Step 3: Run the e2e test under strict mode**

Run: `REEVE_TESTS_STRICT=1 cargo test --test e2e m5_attach_drops_into_running_tmux_session -- --nocapture --test-threads 1`
Expected: PASS. Runtime ~40-90s (devcontainer build + apt-get install tmux is slow on first run).

- [ ] **Step 4: Run the full e2e suite to catch regressions from `build_test_template` change**

Run: `REEVE_TESTS_STRICT=1 cargo test --test e2e -- --test-threads 1`
Expected: 6 e2e tests pass (5 previous + 1 new). The `build_test_template` change adds tmux installation to all e2e tests, but none of them care whether tmux is present — they should all still pass.

- [ ] **Step 5: Workspace-wide verification**

Run:
```bash
REEVE_TESTS_STRICT=1 cargo test --workspace -- --test-threads 1
cargo fmt --check
cargo clippy --workspace -- -D warnings
```
All three must pass clean. If `cargo fmt` finds drift, run `cargo fmt` and stage the changes; commit with a `chore(m5): fmt` message.

- [ ] **Step 6: Commit the e2e test**

```bash
git add tests/e2e.rs
git commit -m "test(m5): e2e attach — tmux session present in container after mv → implementing"
```

If fmt produced changes:
```bash
git add -A
git commit -m "chore(m5): cargo fmt residue"
```

- [ ] **Step 7: Manual smoke (run with the user)**

Defer the manual smoke — the user runs it interactively. Workflow when ready:

1. `reeve daemon stop && reeve daemon start` (pick up the new binary)
2. Pick or scaffold a household whose `devcontainer.json` includes tmux. The witan-household template should be PR'd to include tmux by default; until then, edit local `~/Source/reeve-household/.devcontainer/devcontainer.json` to add `"postCreateCommand": "sudo apt-get update && sudo apt-get install -y tmux"` (or merge with the existing claude install).
3. `reeve new --household reeve-self --brief "m5 smoke"` → capture `$CARD`.
4. `reeve mv $CARD implementing` → wait for container.
5. `reeve attach $CARD` → expect Claude Code TUI to appear in the terminal.
6. Send a message, get a response.
7. ctrl-b d → expect shell prompt back.
8. `reeve attach $CARD` again → same session, history intact.
9. `reeve mv $CARD done`.

If the postCreateCommand for tmux conflicts with the existing claude install postCreateCommand: combine into one chained command, e.g. `sudo apt-get update && sudo apt-get install -y tmux && curl -fsSL https://claude.ai/install.sh | bash`. The order doesn't matter; both must succeed.

---

## Self-review checklist

- [ ] **Spec coverage:** every section of `docs/superpowers/specs/2026-05-17-m5-attach-to-card-agent-design.md` maps to at least one task.
  - Protocol types → Task 1
  - `ContainerOps::exec_in_tmux` → Task 2
  - `ensure_running` tmux startup (warn-not-bail) → Task 3
  - `handle_attach` + dispatcher → Task 4
  - CLI `reeve attach <id>` with `exec` replacement → Task 5
  - E2E + workspace verification + manual smoke + tmux in template → Task 6
  - Session lifecycle (creation, destruction implicit, daemon-restart adoption-no-tmux) → covered by Task 3's adopt-path comment + Task 6 e2e
  - Error semantics → covered by Task 4's 4 tests + Task 3's warn message + Task 5's CLI error mapping
  - "(claude || bash) fallback" → Task 3's `tmux_cmd` string
  - Tmux session name `"reeve"` (fixed) → Tasks 3, 4, 5, 6 all reference `"reeve"`
  - Deferred items → documented at plan top, not implemented

- [ ] **Placeholder scan:** no `TBD/TODO/implement later/similar to Task N` patterns. Every code step contains complete code.

- [ ] **Type consistency:**
  - `AttachParams { id: String }` and `AttachResult { container_name: String, session_name: String, in_container_workspace: String }` — defined in Task 1, consumed in Tasks 4 and 5. Field names match.
  - `ContainerOps::exec_in_tmux(&self, container_name: &str, session_name: &str, command: &str) -> Result<()>` — defined in Task 2, called in Task 3. Signature consistent.
  - Session name `"reeve"` — used as literal string in Tasks 3 (creation), 4 (`AttachResult.session_name`), 5 (CLI uses returned name), 6 (e2e tmux assertions). No constant; the string is short enough to inline. If we want a constant later, easy refactor.
  - `tmux new-session -d -s <name> -- bash -lc <command>` — Task 2 (impl), reflected in Task 3 (caller passes `command` only, not the full argv). Consistent.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-m5-attach-to-card-agent.md`. Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, two-stage review (spec compliance → code quality) after each. Same workflow as M3.5/M4. Fast iteration.

**2. Inline Execution** — execute tasks in this session via executing-plans, batch with checkpoints.

Which approach?
