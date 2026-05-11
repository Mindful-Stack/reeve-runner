# Reeve M2 — Container Lifecycle

**Status:** Design (2026-05-11)
**Predecessor:** M1 walking skeleton (merged to `main` as `f2e1ed3`, 2026-05-11)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

## Context

M1 delivered the walking skeleton: a `reeved` daemon, a `reeve` user binary, JSON-line RPC over a Unix socket, SQLite-backed card metadata, six-column kanban model, and a read-only TUI. The daemon manages card state only — it has no concept of containers, agents, git, or services.

M2 takes the first step from "metadata management" toward "card runs work somewhere": when a card transitions to `Implementing`, the daemon spawns a Docker container for it; when the card reaches `Done`, the container is torn down. The container has the daemon's per-card socket and `~/.claude/` read-only state mounted in, and can call back to the daemon over that socket via self-scoped RPC methods.

M2 deliberately does *not* yet:

- clone repos into per-card workspaces (M3),
- continuously fetch into namespaced refs (M3),
- integrate Lorekeeper (M3 or later),
- start Claude Code inside the container (depends on tmux/pty integration; separate slice),
- integrate the heavy-slot scheduler or per-card services (later milestones),
- wire the TUI to perform mutations (separate slice),
- implement `devcontainer up` against a household-supplied `devcontainer.json` (M3 — coupled to repo clones).

The point of M2 is to prove the lifecycle, mount, identity, and reattach wiring against a minimal `docker run` invocation. Once that plumbing is sound, M3 swaps `docker run` for `devcontainer up` against a real workspace.

## Summary

When the user moves a card to `Implementing`, `reeved` runs `docker run -d --name reeve-<card-id> …` with a household-configured image and command, mounting the daemon's per-card Unix socket plus a read-only slice of `~/.claude/`. The container can talk back to the daemon over the mounted socket and the daemon knows which card is calling because each card has its own dedicated socket file. When the card reaches `Done`, the daemon tears the container down. On daemon restart, surviving containers are reconciled with SQLite state and adopted.

## Architecture

### New modules

- `crates/reeved/src/container.rs` — Docker subprocess wrapper. Owns `spawn_for_card`, `teardown_for_card`, `teardown_all`, `list_reeve_containers`, and the reconcile routine.
- `crates/reeved/src/transitions.rs` — Column-change side effects. One entry point: `on_column_change(old, new, card, ctx)`.
- `crates/reeved/src/household.rs` — TOML household-profile loader. Caches profiles in memory after daemon start.

### Modified modules

- `crates/reeve-protocol/src/lib.rs` — adds `MvSelfParams`, `StatusSelfParams`, `StatusSelfResult`; adds `FORBIDDEN: 403` error code.
- `crates/reeved/src/main.rs` — on startup: load households, run container reconcile, start per-card socket listeners for any `Running` containers. On shutdown: containers are left running (see *Daemon shutdown semantics*).
- `crates/reeved/src/server.rs` — gains per-card socket listener machinery (`spawn_card_listener`, `stop_card_listener`). Each accepted connection carries a `CallerContext { card_id: Option<String> }`. Host socket → `None`; per-card socket → `Some(card_id)`.
- `crates/reeved/src/handlers.rs` — `dispatch` now takes `CallerContext`. `handle_mv` invokes `transitions::on_column_change` on a successful column change. Adds `handle_mv_self`, `handle_status_self` (rejected with `FORBIDDEN` if `caller.card_id.is_none()`).
- `crates/reeved/src/db.rs` — adds the `containers` table plus CRUD helpers.
- `crates/reeve/src/cli.rs` — adds `mv-self` and `status-self` subcommands. Auto-detects the in-container socket path.

### Per-card socket identity (key design choice)

The daemon already binds the host socket at `$XDG_RUNTIME_DIR/reeved.sock`. For each running card, the daemon binds a second socket at `$XDG_RUNTIME_DIR/reeve/cards/<card-id>.sock` (mode `0600`, parent dir created `0700`). Only that per-card socket is mounted into the card's container, at `/var/run/reeved.sock`. Bind-time unlinks any existing file at the socket path — leftover socket files survive daemon shutdown (since containers do too), so the daemon always claims the path defensively.

The daemon resolves caller identity from which listener received the connection. The host socket dispatches with `caller.card_id = None`; a per-card socket dispatches with `caller.card_id = Some("<id>")`. This makes self-scoped calls unambiguous and removes any need for `SO_PEERCRED`, cgroup walking, or container introspection. As a side effect, self-scoped methods do not (and cannot) accept a card-id argument from the caller — the daemon supplies it.

### RPC additions

Two new methods:

| Method | Params | Result | Allowed callers |
|---|---|---|---|
| `mv-self` | `{ column: Column }` | `{ card: Card }` | per-card socket only |
| `status-self` | `{}` | `{ card: Card, container_status: Option<ContainerStatus> }` | per-card socket only |

Calling a self-scoped method on the host socket returns `RpcError { code: 403, message: "self-scoped methods require per-card socket" }`. Calling any non-self-scoped method on a per-card socket is allowed in M2 (no host-scoped lockdown yet) — the security model around what containers can do *to other cards* lands when fork operations arrive.

### Container lifecycle

Triggered exclusively by column transitions:

| Old column | New column | Action |
|---|---|---|
| any (≠ `Implementing`) | `Implementing` | spawn container if no `Running` row exists |
| `Implementing` / `Testing` / `Review` | `Done` | teardown container if a `Running` row exists |
| `Implementing` | `Testing` / `Review` | no-op (container kept warm) |
| `Backlog` ↔ `Planning` | any | no-op |
| anything else | `Backlog` / `Planning` | no-op (container should not exist) |

If the daemon encounters an inconsistent state (e.g., transitioning `Backlog → Done` while a `Running` row exists from a stale crash), it teardowns defensively and logs at `warn`.

### Spawning a container

```
docker run -d \
  --name reeve-<card-id> \
  --label reeve.card_id=<card-id> \
  -v <per-card-socket-path>:/var/run/reeved.sock \
  -v $HOME/.claude/skills:/home/agent/.claude/skills:ro \
  -v $HOME/.claude/settings.json:/home/agent/.claude/settings.json:ro \
  -v $HOME/.claude/CLAUDE.md:/home/agent/.claude/CLAUDE.md:ro \
  -e ANTHROPIC_API_KEY=<from daemon env> \
  -e REEVE_CARD_ID=<card-id> \
  <household.image> \
  <household.cmd...>
```

Notes:

- `docker run` runs without `--rm`, so the container persists after the entrypoint exits. The daemon does explicit `docker rm reeve-<card-id>` on teardown so it can read exit codes if it needs to.
- The in-container username/home is fixed to `agent` for M2. Mount targets assume that. Households configure their image to have a user named `agent`. (Configurable in a later milestone.)
- Read-only mounts that don't exist on the host are silently skipped (warn-logged). This avoids the daemon refusing to start because the user hasn't created `~/.claude/CLAUDE.md`.
- `ANTHROPIC_API_KEY` is read from the daemon's process environment at spawn time. If unset, the env var is omitted from `docker run`. Households whose `cmd` needs the key will surface their own error.

### Teardown

`docker stop --time 5 reeve-<card-id>` (SIGTERM with a 5-second grace), then `docker rm reeve-<card-id>`. Per-card socket listener is stopped, the socket file is removed, and the SQLite `containers` row is updated to `Stopped`.

### Daemon shutdown semantics

On `reeved` shutdown, **containers are left running**. Rationale: the parent design specifies "Daemon kill mid-day → cards keep running". Per-card socket files remain on disk but are stale; in-container `reeve` calls will fail with connection-refused until the daemon restarts.

A user who wants to tear everything down can call `reeve mv <id> done` for each card before stopping the daemon, or (post-M2) we can add a `reeved --teardown-all-on-exit` flag if it turns out useful.

### Daemon startup reconcile

On startup, before opening the host socket:

1. Load household profiles from `~/.config/reeve/households/*.toml` into the in-memory map.
2. Read all rows from `containers` where `status = 'Running'`.
3. For each, check whether the container actually exists via `docker inspect --format '{{.State.Status}}' reeve-<card-id>`.
   - **`running`** → adopt: unlink any stale per-card socket file on disk (containers persist across daemon stop, so the previous socket file is still there); spawn the per-card socket listener afresh; leave row as `Running`.
   - **`exited` / not found** → orphan: update row to `Failed`; log at `warn`. Card column is left untouched (the user can see "container failed" via `status` in a later milestone). Per-card socket file is removed if present.
4. Begin accepting host-socket connections.

### Household profile (minimal)

Path: `~/.config/reeve/households/<name>.toml`. Minimal M2 schema:

```toml
image = "alpine:latest"
cmd = ["sleep", "infinity"]
```

Loaded once at daemon start; not hot-reloaded in M2. A missing profile when a card moves to `Implementing` results in `RpcError { code: 500, message: "household '<name>' not configured" }` and the column move is rolled back.

The full household schema from the parent design (knowledge_base, repos, services, limits, security) is intentionally **not** parsed yet. The parser is permissive — unknown fields are ignored — so M3 and later can add fields without breaking M2 profiles.

### `containers` table

```sql
CREATE TABLE containers (
  card_id TEXT PRIMARY KEY REFERENCES cards(id) ON DELETE CASCADE,
  container_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('Spawning','Running','Stopped','Failed')),
  started_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
```

State transitions written by the daemon:

- `INSERT … Spawning` immediately before `docker run`
- `UPDATE … Running` on `docker run` success
- `UPDATE … Stopped` after successful teardown
- `UPDATE … Failed` if `docker run` errors or reconcile finds the container gone

`Spawning` rows that linger across a daemon restart are treated as `Failed` during reconcile (the daemon doesn't know whether the half-spawned container exists; it errs on the side of marking failed and lets the user retry).

### In-container `reeve` CLI

The `reeve` binary, when run with `mv-self` or `status-self`:

1. Looks for `/var/run/reeved.sock` first. If present, uses it.
2. Falls back to `$XDG_RUNTIME_DIR/reeved.sock` (host case — but `mv-self` / `status-self` will then be rejected with `FORBIDDEN`).

Host-side `reeve` subcommands (`ls`, `new`, `mv`, `daemon …`) continue to talk to `$XDG_RUNTIME_DIR/reeved.sock`. Self-scoped subcommands run on the host return `FORBIDDEN` — by design, this is what proves the auth boundary holds.

## Lifecycle module note

`reeved/src/lifecycle.rs` currently holds data-directory helpers. The M2 work introduces what is properly a *card* lifecycle (spawn/teardown). To avoid the naming clash, M2 names the new module `container.rs`, not `lifecycle.rs`. The existing `lifecycle.rs` stays put.

## Testing

The existing M1 e2e tests (32 passing) remain untouched. M2 adds tests at three levels.

### Unit tests (colocated)

- `container::spawn_command` — pure function that builds the `docker run` argv from a card and household. Test: assert exact argv for a given input.
- `transitions::on_column_change` — table-driven test for the transition matrix. Mocks the container handle with a trait, asserts which container ops were invoked.
- `household::load` — minimal TOML parses, missing required fields error cleanly, unknown fields are tolerated.

### Integration tests (in-process)

- Per-card socket listener: spin up `server::spawn_card_listener("test-card")`, connect a client, send a request, assert `caller.card_id == Some("test-card")` reached the handler.
- `mv-self` on host socket → `FORBIDDEN`. `mv-self` on per-card socket → success and the right card moved.

### E2E (Docker-gated)

A new test in `tests/e2e.rs`, gated by `is_docker_available()` (returns false → test prints a skip message and returns). Uses `alpine:latest` + `["sleep", "infinity"]`. Flow:

1. Spawn daemon under `tempfile::tempdir()` for `XDG_RUNTIME_DIR` and `XDG_DATA_HOME`.
2. Write a minimal household TOML into the temp config dir; daemon picks it up.
3. `reeve new` → card created in `Backlog`.
4. `reeve mv <id> implementing` → assert via `docker inspect` that `reeve-<id>` is `running`. Assert the per-card socket file exists.
5. `docker cp <reeve-binary> reeve-<id>:/usr/local/bin/reeve` → copy the just-built `reeve` binary into the container. (Production households install `reeve` in their image; the test side-steps that by copying at runtime.)
6. `docker exec reeve-<id> /usr/local/bin/reeve mv-self review` → assert the card is now in `Review` and the container is still running.
7. `reeve mv <id> done` → assert `docker inspect` returns "no such container".
8. Daemon stop via the existing `DaemonGuard` SIGTERM path.

A second e2e test exercises the reconcile path: spawn daemon, move card to `Implementing`, copy the `reeve` binary into the container as above, SIGKILL the daemon (bypasses graceful teardown), confirm container survives, restart daemon, confirm the per-card socket is re-created and `docker exec … /usr/local/bin/reeve mv-self review` works.

## Open items deferred to writing-plans

- Exact `docker` argv quoting for `cmd` arrays with spaces / special chars.
- Whether to depend on the `bollard` crate later (M2 stays on subprocess).
- Where the daemon writes container stdout/stderr (M2: discard; later: per-card log file).
- Error-recovery policy when `docker stop` times out (M2: log + force `docker rm -f`; reconsider later).
- Whether reconcile-found `exited` containers should auto-move the card to a "container failed" sentinel column (probably not — M2 just records `Failed`).

## Out of scope (do not implement in M2)

- Anything in the parent design's deferred list.
- Repo clones into `~/cards/<id>/<repo>/`.
- `refs/reeve/<id>/*` fetch.
- Lorekeeper / KB clone / plugin loading.
- `devcontainer up`.
- tmux sessions and `reeve attach`.
- Fork operations (`f`, `b`).
- TUI mutations.
- Heavy-slot scheduler and `shared-exclusive` services.
- `shared` services lifecycle.
- Per-card CPU/memory caps.
- Container-internal Claude Code integration.
- Podman backend.

## Verification

A merged M2 must:

1. Pass all M1 e2e tests unchanged.
2. Pass the new unit, integration, and Docker-gated e2e tests.
3. Manually: with `docker` installed, a user can write `~/.config/reeve/households/test.toml` with an `alpine` image, run `reeve new --household test --brief "smoke"`, `reeve mv <id> implementing`, see `docker ps | grep reeve-` show the container, `docker exec` into it, run `reeve mv-self done`, see the container removed.
4. SIGKILL the daemon mid-life, restart, confirm the container is still there and `docker exec reeve-<id> reeve status-self` returns a sensible card.
