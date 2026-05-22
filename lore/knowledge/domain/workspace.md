---
title: Workspace — per-card materialization (clones + container + listener)
description: The Workspace bounded context. Provisions a per-card on-disk working tree (poly-repo clones via --reference --dissociate), spawns and tears down the devcontainer that hosts the agent, and runs the background fetch worker that mirrors card-branch refs into the host meta-repo.
tags: [domain, ddd, core, workspace, container]
---

# Workspace

## Purpose

Workspace is the ephemeral, per-card materialization of work. While [[domain/card]] is the persistent kanban metadata, Workspace is the *thing the agent actually lives in*: a directory of git clones at `~/reeve/cards/<card-id>/`, a Docker container running the agent's environment, a per-card Unix socket the agent uses to talk back to the daemon, and a tmux session inside the container running Claude.

Workspaces are created on `Card → Implementing` and torn down on `Card → Done` (or defensively on a backwards move out of an active column). One Workspace per Card; the `card_id` is the Workspace's primary key.

This context also owns the **fetch worker** (background loop that mirrors card-branch refs into the host meta-repo), the **clone topology** (`--reference --dissociate` for cheap-then-detached clones), and the **container lifecycle** (devcontainer up, adopt-if-running, teardown).

The Workspace aggregate *implements* the cross-aggregate `WorkspaceLifecycle` port defined by [[domain/card]] — it is the supplier; Card is the consumer.

## Key entities

- **Workspace** — root aggregate. Identity is `card_id` (1:1 with a Card). Holds `container: Option<Container>` (None when not provisioned), `socket_path`.
- **Container** — sub-entity. Holds `name` (docker-assigned), `status` (ContainerStatus enum), `started_at`. Constructed via `Container::new(...)`; status mutated via `with_status`.
- **`ContainerStatus`** — value enum: `Spawning`, `Running`, `Stopped`, `Failed`. Wire-form via `as_wire_str()`/`from_wire_str()` like Column.
- **`RepoOnDisk`** — value enum: `Workspace`, `Sibling`, `Inline`, `Missing`. Derived at `household.show` time by inspecting each manifest entry against the meta-repo on disk.
- **`FfOutcome`** — value enum from the fast-forward helper: `Updated`, `NotFastForward`, `NoChange`. Surfaced in push results.
- **`LiveWorkspaceLifecycle`** — the daemon-side concrete that implements [[domain/card]]'s `WorkspaceLifecycle` trait. Composes `WorkspaceRepo`, `HouseholdRepo`, `GitOps`, `ContainerOps`, and a `ListenerHandle` (port abstracting over the per-card socket registry).
- **`FetchDeps`** — port the fetch worker consumes. Implemented in `reeved` by `DaemonState`. Lets `reeve-workspace::fetch::run` operate without depending on reeved's internals.

## Ubiquitous language

- *Workspace* — the per-card materialization (this aggregate). Lowercased "workspace" is sometimes used informally; always means the per-card thing. (The witan-household manifest formerly used `workspace` as a field name; it's now `meta_repo`. See [[domain/household]].)
- *Card workspace path* — the on-disk directory at `~/reeve/cards/<card-id>/`. The agent's working tree. One repo per clone, side by side.
- *Container* — the Docker container Reeve spawns to host the agent for one card. Tagged with `label=reeve.card_id=<id>` for discovery.
- *Adopt* — at startup, find a surviving container by its label and re-attach to it without re-cloning or re-spawning. Survives daemon restarts.
- *Tmux session* — the per-card tmux session inside the container, running `(claude || bash)`. Created on fresh-spawn (NOT on adopt); attached to via `reeve attach`.
- *Fetch worker* — a tokio task spawned at daemon startup; polls every N seconds; for each card whose container row is `Running` or `Spawning`, runs `git fetch <card-workspace-clone> '+refs/heads/*:refs/reeve/<card-id>/*'` on each host-side repo declared in the manifest that has its own `.git/` directory. Includes the meta-repo (which has its own `.git/`); skips inline entries (which don't). See "Selection by container state" below.
- *Namespaced refs* — refs at `refs/reeve/<card-id>/<branch>` in the host's meta-repo. The user's normal `refs/heads/` namespace is never touched.
- *Push* — promote a card's namespaced refs to GitHub. `reeve push <card-id>` fast-forwards `refs/heads/<branch>` from the namespaced ref then `git push origin <branch>`. Per-branch outcome reported (`Pushed` / `NotFastForward` / `PushFailed`).
- *Provision* — the full "create the workspace" sequence: clone repos, check devcontainer.json, spawn container, discover its name, write container row, spawn listener, start tmux. Triggered by `ensure_running`.
- *Teardown* — the inverse: stop listener, find container row, ensure docker container is gone (or already absent), remove the container row. Triggered by `ensure_stopped`.

## Invariants

These are enforced in `reeve-workspace/src/lifecycle.rs::LiveWorkspaceLifecycle` and pinned by tests + the M5 e2e suite.

- **One container per card (upsert semantics).** The containers table has `card_id` as PRIMARY KEY. `upsert_container` is the only write path; concurrent provision attempts merge rather than duplicate.
- **No FK to cards.** `containers.card_id` does not `REFERENCES cards(id)`. Schema independence is the rule. The "does the card exist?" check lives at the application layer (we wouldn't have been called by the policy if the card didn't exist). Removing the FK was an explicit decision; see [[general/architecture]] §"Schema independence".
- **Idempotent provisioning.** `clone_all_for_card` checks if `<workspace-path>/.git` already exists; if so, the workspace is already cloned and the call is a no-op. The previous destructive cleanup (which wiped the workspace dir on rerun) was dropped in `9150ba3` — it caused agent work to be lost on retry.
- **Adopt-existing-running-container.** If `docker ps --filter label=reeve.card_id=<id>` returns a name, and `docker inspect --format {{.State.Status}}` returns `running`, the existing container is adopted: write the row, spawn the listener, return. Skip the clone+spawn step entirely. The adopt path does NOT call `exec_in_tmux` (preserves any pre-existing tmux state).
- **`exec_in_tmux` only on fresh-spawn.** The M5 contract: when we spawn a brand-new container, we then `docker exec <name> tmux new-session -d -s reeve 'cd /workspaces/<id> && (claude || bash)'`. Warn-not-bail: if tmux isn't installed, we log and continue (the user gets a clearer error when `reeve attach` fails). This is the *only* invariant that was regressed and re-added during the refactor (commit `55a5752`); the post-Card-γ unit test in `lifecycle.rs::tests` pins it.
- **Listener spawn after container is up.** `state.listeners.spawn(card_id, socket)` is called only after `discover_with_deadline` succeeds and the container row is written. The order matters: the per-card socket file at `~/reeve/cards/<id>.sock` only becomes useful once the container is running and has the cards socket dir bind-mounted.
- **Fetch worker selects by container state, not card column.** `FetchDeps::list_running_cards` queries `WorkspaceRepo::list_by_status(Running)` plus `list_by_status(Spawning)` and joins each to its card via `CardRepo::get`. The fetch follows the container, not the column. Consequence: a card in Implementing without a running container (e.g. mid-teardown, or after a reconcile-orphan) gets *no* fetches; a container row that outlives its card's column transition (briefly, before `ensure_stopped` removes it) *does* get fetched until the row is removed.
- **Fetch iterates the meta-repo and every sibling that has `.git/` on disk.** The walk uses `reeve_household::iter_card_repos(&household, &manifest)`, which yields the meta-repo entry first (host_path = `household.meta_repo_path`), then each manifest sibling (host_path = `<meta_repo_path>/<sibling-name>`). Each entry is gated by `host_path.join(".git").exists()` — meta-repo always passes (it has `.git/`); manifest siblings pass iff they're "Sibling" on-disk classification, not "Inline" (no `.git/`) or "Missing" (no dir). The card-side source path for each fetch: meta-repo → `<card-workspace>/`; siblings → `<card-workspace>/<sibling-name>/`. Fixed in `59e38f9` after observing fetch errors on inline-KB households (the .git gate is what makes the inline case a no-op).

## RPC surface (2 methods)

| Method | Handler | Description |
|---|---|---|
| `attach` | `handle_attach` | Look up the running container for a card; return its name + the tmux session name. `reeve attach` CLI then `docker exec`s the user into the session. Read-only — does not spawn anything. |
| `push` | `handle_push` | Iterate the household's repos; for each branch in `refs/reeve/<card-id>/`, fast-forward `refs/heads/<branch>` then `git push origin <branch>`. Per-branch result. |

(Provisioning and teardown are not RPCs — they're side effects of [[domain/card]] column transitions, invoked via the `WorkspaceLifecycle` port.)

## Components inside `reeve-workspace`

- `domain.rs` — `Workspace`, `Container` entities. Pure: no `rusqlite`, no `tokio::process`, no docker types.
- `repo.rs` — `WorkspaceRepo` trait + `pub(crate) SqliteWorkspaceRepo` adapter + `make_workspace_repo` factory + `init_schema(containers table)`.
- `lifecycle.rs` — `LiveWorkspaceLifecycle` impl of `reeve_card::WorkspaceLifecycle`; the orchestration that ties the rest together. Also defines the `ListenerOps` port (implemented in reeved's transport layer).
- `git.rs` — `GitOps` trait + `pub(crate) RealGitOps` adapter + `make_git_ops` factory. Operations: `clone(reference + dissociate)`, `clone_plain`, `fetch`, `fast_forward`, `push`. All shell out via `tokio::process::Command`.
- `clones.rs` — `clone_all_for_card(ops, household, manifest, workspace_path)`: the per-card clone workflow. Calls `GitOps::clone` for the meta-repo, then for each sibling (with `--reference` pointing at the host's sibling clone). Inline entries arrive with the meta-repo clone.
- `container.rs` — `ContainerOps` trait + `pub(crate) DockerOps` adapter + `make_container_ops` factory + `devcontainer_up_command(...)` pure argv builder + `reconcile_one` pure decision function.
- `fetch.rs` — `FetchDeps` trait (consumer-defined; impl'd by reeved::DaemonState) + `pub async fn run(deps, shutdown)` background worker.
- `handler.rs` — `handle_attach`, `handle_push`. Take port references; no `DaemonState` import.

## Reconcile pass (startup)

On daemon start, `reeved/src/reconcile.rs::reconcile_on_startup` (still in reeved because it depends on DaemonState/ListenerRegistry) does the following:

1. List all container rows with status `Running` or `Spawning`.
2. For each row, `docker inspect <name>` to find the actual state.
3. **Adopt** (docker says `running`): re-spawn the listener, mark the row Running. The agent's workflow is unbroken.
4. **Orphan** (docker says anything else, or container is gone): mark the row `Failed`, unlink the stale per-card socket file.

This is what makes Reeve survive daemon restarts mid-card. Tested via `reconcile_tests` and the M5 e2e suite.

## Integration points

- **Inbound from [[domain/card]]** via the `WorkspaceLifecycle` port (`ensure_running`, `ensure_stopped`). `reeve-workspace` depends on `reeve-card` (inverted-dep). See [[frameworks/rust/inverted-dependency-ports]].
- **Inbound from [[domain/household]]** via `HouseholdRepo::get` (look up `meta_repo_path` for a card's household). `reeve-workspace` also depends on `reeve-household` for this.
- **Inbound RPCs:** `attach` and `push` dispatched via `reeved/src/dispatch.rs`.
- **Outbound:** `tokio::process` to docker and git. No HTTP, no network beyond what docker/git themselves do.
- **Persistence:** `WorkspaceRepo` owns the containers table. No FKs.
- **Filesystem:** `~/reeve/cards/<id>/` for per-card workspaces; `~/.local/share/reeve/runtime/cards/<id>.sock` (or whatever XDG_RUNTIME_DIR puts there) for per-card sockets.

## Key workflows

1. **Provision (`ensure_running(card)`)**:
   1. Look up household via `HouseholdRepo::get(card.household)` → if missing, `bail!("household not configured")` (this is the only stage where we'd encounter a missing household — the precheck in Card prevents most cases, but a card could in theory have a stale household reference).
   2. Adopt-existing-running-container path (label-based discovery + status check). If hit, write row + spawn listener + return.
   3. Load and validate `household.json` (`load_household_manifest`).
   4. `clone_all_for_card(&git, &household, &manifest, &workspace_path)` — clones meta-repo, then each sibling via `--reference --dissociate`. Inline entries travel with the meta-repo clone.
   5. Check `devcontainer.json` exists at `<workspace-path>/.devcontainer/devcontainer.json` (every witan-household must ship one).
   6. Build argv via `devcontainer_up_command(...)`, spawn the container via `containers.spawn(argv)`.
   7. `discover_with_deadline(card_id)`: poll `docker ps --filter label=reeve.card_id=<id>` until a name comes back (2s deadline with exponential backoff).
   8. `workspaces.upsert_container(card_id, name, Running, now)`.
   9. `listeners.spawn(card_id, socket_path)`.
   10. `exec_in_tmux(name, "reeve", "cd /workspaces/<id> && (claude || bash)")`. Warn-not-bail.
2. **Teardown (`ensure_stopped(card_id)`)**:
   1. `listeners.stop(card_id)` (idempotent — no-op if already gone).
   2. `workspaces.get_container(card_id)` → if `None`, return Ok (no container ever existed for this card).
   3. `containers.inspect_status(name)` → if `None`, container is already gone; just remove the row.
   4. Else, `containers.teardown(name)` (docker stop + rm).
   5. `workspaces.remove_container(card_id)`.
3. **Fetch tick** (every `FETCH_INTERVAL` seconds; spawned at startup):
   1. `FetchDeps::list_running_cards()` → query `WorkspaceRepo::list_by_status(Running)` + `list_by_status(Spawning)`, then for each container row look up the card via `CardRepo::get` and emit `RunningCard { card_id, household }`. (Selection follows container state, not card column.)
   2. For each `RunningCard`, `FetchDeps::get_household(name)` to resolve the meta-repo path; `load_household_manifest` to parse `household.json`. Missing household or unparseable manifest: log a warning and continue with the next card.
   3. Walk `iter_card_repos(&household, &manifest)` — yields meta-repo first, then each manifest sibling. For each `(repo_name, host_path)`:
      - Skip if `host_path.join(".git")` doesn't exist (inline entry or missing on disk).
      - Compute the card-side source path: `<workspace>/` for the meta-repo entry, `<workspace>/<repo_name>/` for siblings.
      - `git.fetch(host_path, card_clone, card_id)` — pushes refs from the card clone into the host's repo at `refs/reeve/<card-id>/*`. Per-repo errors are logged and don't abort the tick.
   4. The worker is best-effort: no per-card errors propagate, only the next tick retries.
4. **Push (`push id=X`)**: see RPC table above; the body iterates `iter_card_repos` (workspace + manifest order), per-branch fast-forward, per-branch `git push origin <branch>`, returns a `PushResult { branches }`.

## See also

- [[domain/card]] — the consumer that drives Workspace via `WorkspaceLifecycle`.
- [[domain/household]] — the source of `meta_repo_path` and the `Manifest` we clone from.
- [[frameworks/rust/inverted-dependency-ports]] — why `WorkspaceLifecycle` lives in `reeve-card` and is *implemented* here.
- [[frameworks/rust/repo-port-adapter]] — the `WorkspaceRepo` + `SqliteWorkspaceRepo` + factory shape.
- [[general/architecture]] — schema independence (no FK to cards), `pub(crate)` adapter discipline, factory functions.
