# Reeve M3 — Per-card clones + `devcontainer up`

**Status:** Design (2026-05-13)
**Predecessor:** M2 container lifecycle (merged to `main` as `b3cd69d`, 2026-05-12)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

## Context

M2 wired the container lifecycle: `→ Implementing` spawns a Docker container per card, mounts the daemon's per-card socket directory, and lets the in-container agent call back via `mv-self` / `status-self`. The container ran a household-configured `image` + `cmd` against an empty workspace — the agent had no code to work on.

M3 fills the workspace. When a card moves to `Implementing`, the daemon clones each declared host repo into the card's workspace via `git clone --reference --dissociate` (object-share at clone, then sever the dependency), then spawns the container via `devcontainer up --workspace-folder <workspace> --config <devcontainer.json>` against the household's devcontainer.json. The agent now sees real code at `/workspaces/<card-id>/<repo>/` inside the container and can edit, build, and commit it.

M3 deliberately does *not* yet:

- continuously fetch card branches into `refs/reeve/<card-id>/*` on the host (deferred to M4).
- provide `reeve push <id>` to promote a card's branch to GitHub (M4).
- clone the Lorekeeper KB alongside code repos (M4 or later).
- isolate one container per repo (parent design's v2 refinement — M3 keeps one container per card).
- use `devcontainer down` for teardown (stays on `docker stop && docker rm` per M2).
- enforce `--user agent` (deferred M2 item).
- archive or prune card workspaces after `→ Done` (workspace persists for inspection; a future `reeve archive <id>` handles cleanup).

The point of M3 is to land the **clone topology** and the **`devcontainer up` invocation path** end-to-end. Once that wiring is sound, M4 adds the host-side ref namespace and `reeve push`.

## Summary

The household profile loses its M2-era `image`/`cmd` stopgaps and gains a `devcontainer` path plus a list of `[[repos]]`. On `→ Implementing`, the daemon prechecks every declared host repo exists as a git repository and that the devcontainer.json is readable; if either fails, the column move is rolled back. On success, the daemon creates the card workspace at `~/reeve/cards/<card-id>/`, clones each declared repo into it via `git clone --reference <host> --dissociate <host> <workspace>/<repo-name>/`, then spawns the container via `devcontainer up`. The container's actual name is discovered post-spawn from a `docker ps --filter label=reeve.card_id=<id>` query and stored in the `containers.container_name` column (M2's deterministic `reeve-<id>` is gone — devcontainer CLI chooses the name). All downstream operations (`docker inspect`, `docker stop`, `docker rm`, reconcile) keep using the discovered name and need no logical change.

## Architecture

### New modules

- `crates/reeved/src/clones.rs` — clone topology. Owns `card_workspace_path`, `card_repo_path`, `clone_repo`, `clone_all_repos`, and a `GitOps` trait for testability.

### Modified modules

- `crates/reeved/src/household.rs` — replaces the `HouseholdProfile { image, cmd }` shape with `HouseholdProfile { devcontainer: PathBuf, repos: Vec<RepoSpec> }`. New `RepoSpec { name: String, path: PathBuf }`. Both `devcontainer` and `repos[].path` are tilde-expanded at load time. Zero `[[repos]]` is allowed (degenerate case).
- `crates/reeved/src/container.rs` — replaces `spawn_command` with `devcontainer_up_command` that builds the `devcontainer` CLI argv. Adds `discover_container_name(ops, card_id) -> Option<String>` that queries `docker ps --filter label=reeve.card_id=<id> --format '{{.Names}}'`.
- `crates/reeved/src/effects.rs::ensure_running` — extended flow: create workspace dir → clone all repos → record `Spawning` → run `devcontainer up` → discover container name → write it to the row → mark `Running` → spawn listener.
- `crates/reeved/src/handlers.rs` — host-side precheck in `handle_mv` / `handle_mv_self` extended from M2's "household exists" to also check the devcontainer.json file is readable and every declared repo path contains a `.git` directory.
- `crates/reeve-protocol/src/paths` — adds `cards_workspace_dir()` (respects `$REEVE_CARDS_DIR`, defaults to `$HOME/reeve/cards`) and `card_workspace_path(card_id)`. Shared between the daemon (where it creates the dirs) and any future tooling that wants to find a card's workspace.
- `crates/reeved/Cargo.toml` — no new deps (subprocess-only, no new git crate).
- `Cargo.toml` (workspace) — no changes.

### Cards directory and workspace layout

Card workspaces live under `$REEVE_CARDS_DIR` (default `$HOME/reeve/cards`). The override is the test seam, analogous to `$XDG_RUNTIME_DIR` for sockets. Per-card layout:

```
$REEVE_CARDS_DIR/                    (e.g., ~/reeve/cards/)
└── <card-id>/                        ← workspace root (devcontainer --workspace-folder)
    ├── <repo-name-1>/                ← full clone of host repo 1
    │   └── .git/                     ← dissociated; no objects/info/alternates
    ├── <repo-name-2>/
    └── ...
```

Inside the container, this lands at `/workspaces/<card-id>/<repo>/` by devcontainer convention. The agent navigates per-repo by `cd`.

The cards directory is intentionally **user-visible** — external git tools (GitKraken, lazygit, sublime-merge) point at it to inspect agent work. It is NOT under `~/.local/share/` for that reason.

### New household schema

```toml
devcontainer = "~/Source/myarchy-ops/.devcontainer/devcontainer.json"

[[repos]]
name = "backend"
path = "~/Source/myarchy-backend"

[[repos]]
name = "frontend"
path = "~/Source/myarchy-frontend"
```

`devcontainer` is **required**. `[[repos]]` is optional (zero is valid). `~` expansion is applied to both `devcontainer` and `repos[].path` at TOML load time. Unknown fields are still tolerated (M4 will add `knowledge_base`, `services`, `limits` non-breakingly).

The M2 `image` / `cmd` fields are removed. This is a breaking change to any M2-era profile; pre-v1 status makes this acceptable, and the M2 e2e tests are rewritten to use a real devcontainer.json (see *Testing*).

### Clone mechanics

For each `RepoSpec { name, path }`:

```bash
git clone --reference <repo.path> --dissociate <repo.path> $REEVE_CARDS_DIR/<card-id>/<repo.name>/
```

- `--reference` shares objects from the host at clone time (cheap).
- `--dissociate` then copies the needed objects into the new clone, severing the long-lived dependency. The card survives host-side `git gc --prune=now` or rebase-drops cleanly.
- Source and destination of the `--reference` flag both point at the same host path. (The `--reference` arg is a hint to git about which existing repo to borrow objects from; the trailing path is the clone source URL.)

The daemon shells out to `git` via `tokio::process::Command`. If `git clone` fails for any repo, the daemon removes the partial `<card-id>/` workspace and returns the error. The card column is **not** rolled back at this stage because the column update happens after the precheck and before the spawn — by the time clones run, we've already committed to `Implementing`. Failed clones bubble up as `INTERNAL` errors on the `mv` response; the card stays in `Implementing` with an empty workspace and no container, surfaceable later via `status-self`. (Same `Implementing+Failed` shape M2 already documents as a known follow-up.)

### `devcontainer up` invocation

```
devcontainer up \
  --workspace-folder $REEVE_CARDS_DIR/<card-id>/ \
  --config <household.devcontainer> \
  --id-label reeve.card_id=<card-id> \
  --mount type=bind,source=$XDG_RUNTIME_DIR/reeve/cards,target=/var/run/reeve-cards \
  --mount type=bind,source=$HOME/.claude/skills,target=/home/agent/.claude/skills,readonly \
  --mount type=bind,source=$HOME/.claude/settings.json,target=/home/agent/.claude/settings.json,readonly \
  --mount type=bind,source=$HOME/.claude/CLAUDE.md,target=/home/agent/.claude/CLAUDE.md,readonly \
  --remote-env REEVE_CARD_ID=<card-id> \
  --remote-env ANTHROPIC_API_KEY=<key>
```

Notes:

- The `--mount` injections for `~/.claude/*` honor the M2 contract: each is silently skipped (warn-logged) if the host source path doesn't exist. The argv builder filters absent paths before assembling the command, same shape as M2's `spawn_command`.
- `--remote-env ANTHROPIC_API_KEY=<key>` is omitted when the daemon's env doesn't have the key.
- The household's devcontainer.json declares `image`, `runArgs`, `remoteUser`, `postCreateCommand`, and any household-specific mounts — the daemon does not parse it.
- `--id-label reeve.card_id=<card-id>` is the load-bearing tag for name discovery and for cross-daemon-restart reconcile.
- `devcontainer` is a runtime dependency. The daemon shells out assuming it's in `$PATH`. Documented as a setup requirement.

### Container-name discovery

After `devcontainer up` succeeds:

```bash
docker ps --filter label=reeve.card_id=<card-id> --format '{{.Names}}'
```

The single-line stdout is the container name. The daemon stores it in `containers.container_name`. All M2 callers (`DockerOps::teardown`, `DockerOps::inspect_status`, reconcile) keep working unchanged — they take an opaque name.

`devcontainer up` may return success a beat before the container appears in `docker ps` (the CLI awaits the dial-tone, not docker's metadata commit). The discovery routine polls with a 2-second deadline using exponential backoff (100ms, 200ms, 400ms, 400ms, …) — typically 4-5 poll attempts within the window. If the query is still empty at the deadline, the daemon marks the row `Failed` and returns `INTERNAL "container not discoverable after devcontainer up"`. Multiple matches are impossible in practice (the label is unique per spawn).

The alternative — parsing `devcontainer up --log-format json`'s stdout for `containerId`, then running `docker inspect --format '{{.Name}}' <id>` — is left as a writing-plans-phase consideration. The `docker ps` filter approach is simpler and equally robust given the 2-second poll.

### Precheck (extended)

Replaces the M2 household-only precheck. On `→ Implementing` (host or self), the daemon checks **before** mutating the column:

1. Household profile exists in the loaded map.
2. `household.devcontainer` resolves to a readable file.
3. Every `household.repos[].path` exists and contains a `.git` entry (file or directory — supports submodule worktrees).

Any failure returns `RpcError { code: INTERNAL, message: "<reason>" }` and the column move is rolled back (i.e., the update never happens).

### Reconcile (unchanged)

M2's `reconcile_on_startup` already works on opaque `container_name`s. It needs no change for M3. The pre-existing M2 caveat — `Spawning` rows whose daemon crashed mid-spawn are marked `Failed` — applies symmetrically to M3.

### Workspace cleanup

The card workspace persists across teardown. `→ Done` removes the container (M2 behavior) but leaves `~/reeve/cards/<card-id>/` on disk so the user can inspect history with external tools, replay commits, etc. A future `reeve archive <id>` command will prune; not part of M3.

## Open items deferred to writing-plans

- Whether to expand `~` via the `dirs` crate or by hand (probably by hand to avoid a new dep).
- How to surface `Implementing+Failed` cards to the user (M2 follow-up; remains so).
- Parallel vs. serial clones (`clone_all_repos` is a sketch — small-repo households want parallel; large-repo households want serial to avoid network/disk contention). Default to serial for predictability in M3.

## Testing

### Unit tests (colocated)

- `clones::card_workspace_path` respects `$REEVE_CARDS_DIR`.
- `clones::clone_repo` builds the correct git argv (pure, via a `GitOps` trait mock).
- `clones::clone_all_repos` cleans up partial workspaces on failure.
- `container::devcontainer_up_command` builds the correct argv for a representative input (mount filtering, env injection, label tagging).
- `container::discover_container_name` parses single-line, empty, and multi-line stdout.
- `household::load` parses the new schema, rejects missing `devcontainer`, accepts zero `[[repos]]`, tilde-expands both fields, tolerates unknown fields.

### Integration tests (colocated, real git)

A `clones::integration_tests` module that uses `tempfile::tempdir` for both source and dest:

- Initialize a host repo with one commit. Clone via `clones::clone_repo`. Assert: dest exists, has `.git`, `objects/info/alternates` is absent (dissociated), `git log` shows the commit.
- Initialize two host repos. Call `clone_all_repos`. Assert both clones present.
- Clone with non-git source path → error; partial workspace removed.

These tests don't gate on Docker — they require `git` in `$PATH`, which is a strict subset of any reasonable dev environment.

### E2E (Docker-gated, rewritten)

The M2 `m2_container_lifecycle_happy_path` and `m2_reconcile_after_daemon_sigkill` tests are rewritten:

- Test setup writes a minimal devcontainer.json into the temp dir (`{"image": "debian:trixie-slim", "remoteUser": "agent"}`).
- Test setup `git init`s a host repo with one commit.
- Household TOML points `devcontainer` at the temp devcontainer.json and adds one `[[repos]]` entry pointing at the host repo.
- Test sets `REEVE_CARDS_DIR` to a temp path.
- Flow: `mv implementing` → assert clone exists at `<REEVE_CARDS_DIR>/<id>/<repo-name>/.git`, container running (name discovered from label), per-card socket exists, `docker cp` reeve into the container, `mv-self review` works, `mv done` tears down. Workspace persists on disk after Done.
- Reconcile test: same setup, then SIGKILL daemon, restart, confirm container survives, listener re-binds, in-container `mv-self` works.

The test image stays `debian:trixie-slim` (glibc compatibility).

## Out of scope (do not implement in M3)

- Anything in the parent design's M4+ deferred list.
- `refs/reeve/<id>/*` continuous fetch.
- `reeve push <id>`.
- Lorekeeper KB integration.
- Per-repo containers.
- `devcontainer down` for teardown.
- `--user agent` enforcement.
- Workspace pruning after `→ Done`.
- Concurrent-card host-mount isolation (the M2 follow-up — separate work).
- Replacing M2's subprocess wrapper with a Docker SDK crate (e.g., `bollard`).

## Verification

A merged M3 must:

1. Pass all M1 e2e tests untouched.
2. Pass the new unit, integration, and Docker-gated e2e tests (rewritten).
3. Pass the existing `--user agent` / `Stopped` enum / `db` plumbing follow-ups (M2's deferred items) **unaffected** — M3 should not regress them.
4. Manually: with `git`, `docker`, and `devcontainer` (`@devcontainers/cli`) installed, a user writes a household TOML pointing at a real host repo + a real devcontainer.json, runs `reeve new --household <name> --brief "<x>"`, `reeve mv <id> implementing`, observes `~/reeve/cards/<id>/<repo>/` populated with the clone (severed: `cat .git/objects/info/alternates` returns no such file), the container running (`docker ps | grep reeve.card_id`), and the agent able to `cd /workspaces/<id>/<repo>/ && git log` inside the container.
