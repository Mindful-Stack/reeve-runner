# Reeve M5 — `reeve attach <card>` (daemon-managed tmux + claude per card)

**Status:** Design (2026-05-17)
**Predecessor:** M4 namespaced refs + push (`2026-05-14-m4-refs-and-push-and-kb-design.md`)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

## Context

After M1-M4, Reeve can spawn a per-card devcontainer, clone the meta-repo + siblings into it, mount the per-card socket, fetch agent commits back into the host on a 5-second timer, and push to `origin` on demand. But the container is empty of agent activity — nothing in it actually runs Claude Code.

Today the user's path into a card is:

```sh
CONTAINER=$(docker ps --filter label=reeve.card_id=<full-uuid> --format '{{.Names}}')
docker exec -u vscode -it $CONTAINER bash -lc "cd /workspaces/<full-uuid> && claude"
```

Painful enough that the tool isn't usable for daily work. M5 wraps that into one command: `reeve attach <card>`.

To make detach/reattach useful (so a flaky SSH connection or a Ctrl-B D doesn't kill the claude session), the daemon manages a tmux session inside each container. Claude starts inside tmux on `mv → Implementing` and survives detaches. This is the parent design's vision (see §"Live attach escape hatch" and §"Process model").

## Summary

On `mv → Implementing`, after the container is alive and the listener bound, the daemon issues `docker exec -u vscode <container> tmux new-session -d -s reeve -- bash -lc 'cd /workspaces/<id> && (claude || bash)'`. Tmux session creation failures are warn-not-bail — the column transition succeeds even if tmux is missing in the container, so the user can self-diagnose.

A new RPC `attach { id }` resolves the card → container, validates the card is in `Implementing`, and returns the container name + session name. The `reeve attach <id>` CLI subcommand makes that call, then process-replaces itself via Unix `exec` into `docker exec -it <container> tmux attach -t reeve`. The user's terminal becomes tmux directly; no wrapper process.

The `(claude || bash)` fallback inside tmux is defensive — if the claude install glitched (we saw this during the M3.5 smoke), the session lands in bash instead of erroring out, and the user can fix it via `reeve attach`.

M5 deliberately does *not* yet:

- Add TUI mutations or TUI live-attach (the `a` keypress from the parent design). M5 ships CLI-only.
- Auto-restart claude on user-initiated `/exit`. If the user exits, they meant it.
- Add card-id prefix matching or fuzzy lookup in `reeve attach <prefix>`. Full UUIDs only.
- Fall back from `tmux attach` to bare `docker exec ... bash` in the CLI when the session is missing. The tmux error is clear enough; auto-fallback is M6 polish.
- Address the M3.5/M4 known-issues list.
- Stream the agent's output to a host-side event timeline (parent design's structured event stream is a separate concern).
- Spawn one tmux session per claude conversation, support tmux windows, or any multi-session-per-container model. One session, name `reeve`.

## Architecture

### New modules

None. M5 is additive across existing modules.

### Modified modules

- **`crates/reeve-protocol/src/lib.rs`** — three new types:
  - `AttachParams { id: String }`
  - `AttachResult { container_name: String, session_name: String, in_container_workspace: String }`
  - No new `RpcError` codes; reuses `NOT_FOUND`, `INVALID_ARGUMENT`, `INTERNAL`.

- **`crates/reeved/src/container.rs`** — `ContainerOps` trait gains one method:
  ```rust
  /// Start a detached tmux session inside an existing container that runs
  /// `command` in a new shell. Used by `ensure_running` to launch the
  /// per-card claude session. Errors propagate from the underlying
  /// `docker exec` invocation.
  async fn exec_in_tmux(
      &self,
      container_name: &str,
      session_name: &str,
      command: &str,
  ) -> Result<()>;
  ```
  `DockerOps` impl shells `docker exec -u vscode <name> tmux new-session -d -s <session> -- bash -lc <command>`. The `-u vscode` is fixed (matches the M3.5 smoke discovery that postCreate runs as vscode and that's where claude lives). If a user changes their devcontainer's `remoteUser`, they'll need to adapt — flagged for M6 follow-up if it bites.

- **`crates/reeved/src/effects.rs`** — `ensure_running` gains a tail step after `listeners.spawn`:
  ```rust
  // M5: start the per-card claude session inside the container's tmux.
  // Defensive `|| bash` so a broken claude install doesn't lock the user out.
  let cmd = format!("cd /workspaces/{card_id} && (claude || bash)");
  if let Err(e) = self
      .state
      .containers
      .exec_in_tmux(&container_name, "reeve", &cmd)
      .await
  {
      tracing::warn!(
          "tmux session not started in {container_name}: {e:#} — \
           `reeve attach` will return a 'session not found' error; \
           ensure tmux is installed in the container",
      );
  }
  ```
  Warn-not-bail. The column transition succeeds; mv returns Ok. The user discovers the issue when they try to attach.

- **`crates/reeved/src/handlers.rs`** — new `handle_attach`:
  ```rust
  async fn handle_attach(state: &DaemonState, params: AttachParams) -> Result<Value, RpcError> {
      // 1. Card lookup (DB-direct, M3.5 Task 12 pattern).
      // 2. Column check: Implementing only.
      // 3. Container discovery by `reeve.card_id` label.
      // 4. Return AttachResult.
  }
  ```
  Plus dispatcher case `"attach"`.

- **`crates/reeve/src/cli.rs`** — new `Command::Attach { id: String }` variant + `attach(id)` handler. The handler:
  1. Calls `attach` RPC; on `Err`, prints `"daemon error: {e}"` and exits 1.
  2. On `Ok(AttachResult)`, prints one info line: `"attaching to <container>:reeve (ctrl-b d to detach, claude keeps running)"`.
  3. Uses `std::os::unix::process::CommandExt::exec` to process-replace the CLI with `docker exec -it <container> tmux attach -t reeve`. From this point, the `reeve` process is gone; the user's shell IS docker exec.

### Tmux session lifecycle

- **Name.** Fixed string `reeve`. One session per container; no cross-container collision because each container is its own tmux universe.
- **Creation.** Inside `ensure_running`, after container discovery and listener bind. Synchronous w.r.t. the mv RPC — by the time `mv → Implementing` returns to the caller, the tmux session exists (or warn-logged its absence).
- **Destruction.** Implicit. `ensure_stopped` does `docker rm -f <container>`, which removes the container and everything inside it including tmux.
- **Daemon restart.** No effect. Tmux is bound to container PID 1, not the daemon. `container::reconcile_on_startup` adopts existing containers; the tmux session inside them is still alive; `reeve attach` works the same.
- **Adopt path in `ensure_running`.** When a card moves to Implementing and an existing container with the right label is found, the adopt branch returns early WITHOUT calling `exec_in_tmux`. This preserves any tmux state from before the daemon restart. The first-ever start triggers tmux setup; adoption never does.
- **Single-card-multi-attach.** Tmux's native multi-attach works out of the box. Two terminals running `reeve attach <id>` join the same session and see the same screen. Standard tmux ergonomics.

### CLI flow (data flow diagram)

```
$ reeve attach <card-id>
   │
   ▼
client::call("attach", { id })   ─── via $XDG_RUNTIME_DIR/reeved.sock
   │
   ▼
handle_attach
   │
   ├─ db::get_card(id)             ─── None → NOT_FOUND
   ├─ check column == Implementing ─── else  → INVALID_ARGUMENT
   ├─ containers.discover_container_name(id) ── None → INTERNAL
   │
   ▼
AttachResult { container_name, session_name: "reeve",
               in_container_workspace: "/workspaces/<id>" }
   │
   ▼
CLI prints info line
   │
   ▼
exec("docker", ["exec", "-it", <container>, "tmux", "attach", "-t", "reeve"])
   │
   ▼
[reeve process is gone; terminal IS docker exec IS tmux]
```

The `exec` is `std::os::unix::process::CommandExt::exec` — Unix-only. Reeve is already Linux-only (devcontainer CLI and docker semantics throughout assume it); this isn't a new portability constraint.

### Error semantics

| Failure | Where | Behavior |
|---|---|---|
| Card id not found | `handle_attach` DB lookup | `RpcError NOT_FOUND` → CLI prints "daemon error: card '<id>' not found", exit 1 |
| Card not in Implementing | `handle_attach` column check | `RpcError INVALID_ARGUMENT` → CLI prints "card is in <column>; attach only works in Implementing", exit 1 |
| No container row for the card (data drift) | `handle_attach` discovery | `RpcError INTERNAL "container for card '<id>' not running (try mv <id> implementing again)"`, exit 1 |
| Daemon not running | CLI `client::call` | Existing error path: "could not connect to daemon socket ...", exit 1 |
| Tmux missing inside container | `ensure_running` startup | `tracing::warn!` only — column transition succeeds. User runs `reeve attach`, hits tmux's "session not found" error, knows to add tmux to devcontainer features |
| Claude install broken | tmux session post-init | `(claude \|\| bash)` falls back to bash. User attaches and lands in a shell at `/workspaces/<id>`, can manually run install script or rebuild |
| ANTHROPIC_API_KEY missing | claude startup | claude prints its own auth error inside tmux, exits → bash takes over via `\|\|`. User sees the message on attach |
| User ctrl-b d | normal | docker exec exits 0, CLI exits 0, claude keeps running. Reattach with same command |
| User `/exit`s claude | normal | Tmux session exits (no more windows). Container itself stays alive (PID 1 is devcontainer's shell). Reattach hits "session not found" — user does mv done → implementing to rebuild |

### Out of scope (do not implement in M5)

- TUI live attach (`a` keypress from parent design)
- TUI mutations (kanban write operations from the TUI)
- Card-id prefix matching for `reeve attach <prefix>`
- Auto-fallback from `tmux attach` to bare `docker exec ... bash` when session is missing
- Restart-claude-on-/exit
- Structured agent event stream (parent design's "event timeline")
- Multi-window tmux layouts per session
- Configurable tmux session name
- M3.5/M4 known-issues fixes (orphan workspace, shallow clones, mv-into-existing, zombie daemons)
- Heavy-slot scheduler, service categories
- `reeve diff <card>`, `reeve commit <card>`, any other self-scoped agent ops beyond what M2 already wired (`mv-self`, `status-self`)

### Documentation updates

- **`README.md`** — Development section adds tmux to required tools (technically required in the container, not the host; clarify in the prose).
- **The `witan-household` template (out-of-tree)** — its `devcontainer.json` should ship with tmux installed by default. Either via `ghcr.io/devcontainers/features/common-utils:2` with `installTools: ['tmux']` OR via postCreate `apt-get install -y tmux`. We'll PR the template; M5 itself documents the requirement.

## Testing

### Unit (handlers.rs)

- `attach_returns_container_and_session_for_implementing_card` — fresh state, household + card in Implementing, container row exists → `AttachResult.container_name` matches; `session_name == "reeve"`; `in_container_workspace == "/workspaces/<id>"`.
- `attach_rejects_non_implementing_card` — card in Backlog → `INVALID_ARGUMENT`.
- `attach_returns_not_found_for_unknown_card` — empty DB → `NOT_FOUND`.
- `attach_errors_when_no_container_row` — card in Implementing but no container row → `INTERNAL` with "not running" message.

Uses the existing test scaffolding: `fresh_state`, `insert_test_household`, in-memory sqlite. Add a `ConfigurableContainers` mock (or extend the existing `NoopOps` with a configurable `discover_container_name` response) so step 3's docker-discovery path is mockable.

### Unit (effects.rs)

- `start_tmux_session_in_container_warns_on_failure_does_not_bail` — extend the existing `ensure_running` test scaffolding (`ConfigurableGit` was added in M4 Task 7; analogous `ConfigurableContainers` here). With a `ContainerOps` mock that errors `exec_in_tmux`, assert `ensure_running` still returns `Ok(())` and the warn was emitted (capture via `tracing-subscriber`'s test mode if convenient; otherwise smoke-test by asserting no panic).

### Unit (container.rs)

- `exec_in_tmux` is a thin shell-out. Mock-level test: extend `MockGitOps`-equivalent for containers (the existing test patterns) to record the argv shape.
- Real-docker integration test under the existing docker-gated module: pull `ubuntu:24.04`, `apt-get install -y tmux`, `docker run -d ...`, call `exec_in_tmux(name, "test-session", "sleep 30")`, then `docker exec <name> tmux ls` and assert the `test-session` line is present.

### E2E (tests/e2e.rs)

`m5_attach_drops_into_running_tmux_session` (docker-gated, REEVE_TESTS_STRICT-aware):

1. Build the test template with tmux included (extend `build_test_template` to add tmux installation to the devcontainer features OR run `apt-get install -y tmux` in postCreate).
2. `household new` → `reeve new` → `reeve mv <id> implementing`.
3. Discover the container.
4. `docker exec <container> tmux ls` — assert output contains `reeve:` (the session was started by the daemon).
5. Call `attach` RPC directly via `client::call` (skip CLI's `exec`-replacement — can't test through a real PTY in a subprocess test). Assert `AttachResult.container_name`, `session_name == "reeve"`, `in_container_workspace == format!("/workspaces/{card_id}")`.
6. Send a marker line into the tmux session and read it back:
   ```rust
   docker exec <container> tmux send-keys -t reeve "echo MARK_$$" Enter
   docker exec <container> tmux capture-pane -t reeve -p
   ```
   Assert the captured pane contains `MARK_`. Confirms the session is a real shell, not just a name.
7. `reeve mv <id> done` → container teardown → tmux session goes with it.

No PTY-level e2e for `reeve attach` itself. The daemon-side wiring is tested above; the CLI's `exec`-replacement is glue we validate via manual smoke.

### Manual smoke

Using the M3.5 smoke setup (or a fresh household with tmux in its devcontainer):

1. `reeve daemon start`
2. `reeve new --household reeve-self --brief "m5 smoke"` → capture `$CARD`
3. `reeve mv $CARD implementing` → wait for container
4. `reeve attach $CARD` → expect Claude Code TUI to appear in the terminal
5. Type a message, observe a response
6. ctrl-b d → expect shell prompt back
7. `reeve attach $CARD` again → expect same conversation, history intact
8. `reeve mv $CARD done` → container down

## Verification

A merged M5 must:

1. Pass all M1-M4 tests untouched.
2. Pass the new unit, integration, and Docker-gated e2e tests.
3. `cargo test --workspace` clean (with `REEVE_TESTS_STRICT=1`); `cargo fmt --check` clean; `cargo clippy --workspace -- -D warnings` clean.
4. Manual smoke (steps 1-8 above) all pass.

If the manual smoke surfaces a snag (broken claude install in the container, tmux missing, an `exec` quirk we didn't predict), file a follow-up rather than block the milestone, but flag in the PR description.

## Open items deferred to writing-plans

- Exact shape of `ConfigurableContainers` test mock vs extending `NoopOps`. The implementation will pick whichever fits cleanly with the existing handler/effects test patterns.
- Whether `attach` RPC should also return the workspace path inside the container (already in `AttachResult.in_container_workspace`) OR have the CLI compute it from the card id (it's deterministic — `/workspaces/<id>`). Including in the response keeps the contract explicit; computing in the CLI is one fewer DB round-trip. Lean: include in response. Already specified above.
- Whether to add a `--no-attach` companion flag (or separate `reeve session start`) for users who want to spawn the session without joining. Skip for M5 — `mv → implementing` already starts the session.
