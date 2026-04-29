# Reeve — TUI orchestrator for parallel Claude Code sessions

**Status:** Design (2026-04-29)

## Context

The user develops in a poly-repo setup where features routinely span 2-3 repos that share local infrastructure (a database, an Azure Service Bus emulator, occasional additional services). The current single-Claude-Code-session workflow has six gaps:

1. **Forking the chat** — currently approximated with `/btw` or `/resume`.
2. **Reviewing diffs as commits arrive** — manual context-switch to GitKraken / Sublime Merge; no in-flow path to ask the agent about a specific commit.
3. **Working on multiple features simultaneously** — git worktrees alone don't cope with the poly-repo setup.
4. **Safe `--dangerously-skip-permissions` runs** — no isolated environment.
5. **Tracking multiple parallel CC instances** — no overview, no alerts.
6. **Jira intake** — *deferred; out of scope for v1*.

Existing tools were surveyed (vibe-kanban, slay.zone, Crystal, Sculptor, Cursor background agents, Nimbalyst, emdash, cmux, OpenHands, Goose, Devin). vibe-kanban is the only one with native poly-repo support, but its TS+Rust hybrid would trap a Rust-learning rebuild in TypeScript for any frontend work. Build is the chosen path.

## Summary

**Reeve** is a Rust-based, daemon-driven kanban orchestrator for parallel Claude Code sessions. Each "card" represents a feature, owns its own per-repo clones, and runs inside a devcontainer for safe `--dangerously-skip-permissions` execution. The user reviews commits as they land, can fork the chat at the commit or whole-card level, and decides when to push to GitHub.

The system pairs with **Lorekeeper**, a separate agent plugin sourcing standards and domain knowledge from a shared knowledge base. Reeve runs the operations; Lorekeeper holds the lore. They may merge later; for now they share user-facing conventions and develop independently.

## Architecture

### Process model

```
reeved                          (the daemon, always running)
 ├─ tmux server                 (host-shared; daemon spawns sessions in it)
 │   ├─ card-001 ─┐
 │   ├─ card-002 ─┤── each running CC inside its devcontainer
 │   └─ ...       ─┘
 ├─ docker containers           (one per running card)
 └─ shared services             (DB, service bus, ...)

reeve                           (user-facing binary)
 ├─ TUI (default mode)
 └─ CLI (subcommand mode)
```

`reeved` (the daemon) owns: card state (SQLite), per-repo clones, devcontainer lifecycle, tmux sessions, agent processes, lease scheduling, shared service lifecycle, IPC, event distribution.

`reeve` (the user binary) is a thin frontend. With no args it launches the TUI; with a subcommand it dispatches to the daemon via Unix socket. Both modes use the same JSON-RPC protocol over the same socket.

Multiple frontends can connect simultaneously — TUI on the laptop, a second TUI over SSH, a future GUI. They all see the same source of truth.

### Persistence and crash recovery

- **SQLite** for structured state: cards, columns, plans, repo associations, lease history, parent-child relationships.
- **Filesystem** for working state: card clones live under `~/cards/<card-id>/`.
- **Tmux** sessions persist independent of the daemon (tmux is its own process).
- On daemon restart, it reads the DB, reattaches to surviving tmux sessions and containers, and resumes event streaming. `kill reeved` mid-day → cards keep running; only un-streamed events are lost.

## Lifecycle (kanban columns + transition events)

| Column | What happens | Resources |
|---|---|---|
| **Backlog** | Brief only | None |
| **Planning** | Brainstorm + write-plan in a lightweight CC session running on the host | A CC process — no container yet |
| **Implementing** | Plan executes in devcontainer; light mode (compile + unit tests only) | Devcontainer up; per-card services running |
| **Testing** | Full integration; heavy lease acquired | Holds heavy lease; shared-exclusive services started |
| **Review** | Agent paused; commits ready; fork-chat available | Container kept warm |
| **Done** | Archived | Container torn down |

### Column transitions trigger orchestration

- `→ Implementing`: daemon reads the approved plan to determine which repos are involved, clones them under `~/cards/<id>/<repo>/` with `git clone --reference` for object sharing, spins up the devcontainer.
- `→ Testing`: card requests heavy lease (queues if taken); daemon starts the project's `shared-exclusive` services when granted.
- `→ Review`: agent pauses, alert fires. Triggered by an agent-side signal (a `reeve mv-self review` call from inside the container, or a daemon-watched state file) or by manual user action.
- `→ Done`: tear down container; branches stay on host for the user to push or discard.

Light cards may move `Implementing → Review` directly, skipping Testing.

### Reviewability is not gated by column

Diffs are always available regardless of column. The daemon fetches card branches into the host's main repo continuously. "Needs review" is a *signal* the agent sets when the work feels complete — not a gate on the user's ability to inspect.

## TUI layout (default frontend for v1)

### Default view: kanban board

Six columns horizontal. Each card shows: title, repos involved (chips), status icon, age. Column headers carry counts and an attention badge.

### Navigation

- Vim keys (`h/j/k/l`) and arrow keys both supported
- `Space` opens a contextual quick menu; top item is always "Move to next column" — so `Space Space` is the rapid kanban-flow move
- `n` — new card • `Enter` — drill in • `q` — quit • `?` — help

### Drill-in view: split pane

- **Left:** event timeline — structured stream of "plan step started", "commit landed", "agent question", "error", etc. Scrollable.
- **Right:** context tabs:

| Tab | Content |
|---|---|
| Plan | The card's spec/plan markdown |
| Diff | Card-branch vs base, scoped per-repo |
| Commits | List with messages; press `f` to fork-the-chat on a commit |
| Chat | Read-only mirror of the agent's reasoning |

### Live attach escape hatch

Pressing `a` detaches the TUI and drops the user into the card's actual CC session pty (managed by the daemon as a tmux session). Standard tmux detach returns to the TUI. This preserves the full Claude Code experience for moments when the abridged in-TUI view is insufficient.

**Tmux as the pty backend gives free remote access:** SSH into the host, `tmux ls` shows all running cards, attach to any.

### Visual indicators

Card border colors signal state: red (error/blocked), yellow (needs review), dim grey (paused), default (healthy/running).

### Alerts

Mako desktop notifications + TUI badges, on these events:

- Agent asked a question (blocking)
- Agent errored
- Card auto-moved to Review (phase complete)
- Heavy lease granted (for a card the user queued)

Terminal bell dropped (vestigial). Idle-timeout dropped from v1 (YAGNI; daemon supports the config knob, default off).

## CLI mode

Same binary, subcommand dispatches to the daemon over the same socket:

| Command | Purpose |
|---|---|
| `reeve ls` | list cards (column, status, age) |
| `reeve new --project foo --repos a,b --brief "..."` | create card |
| `reeve mv <id> <column>` | move card |
| `reeve attach <id>` | drop into the card's tmux session |
| `reeve diff <id>` | print diff to stdout |
| `reeve logs <id>` | tail event timeline |
| `reeve push <id>` | run `git push origin <branch>` in the host repo |
| `reeve daemon start\|stop\|status` | daemon management |

Scripting hook: future Jira integration becomes `reeve new --jira PROJ-123 --brief "$(jira issue PROJ-123)"`. A git pre-commit hook could call `reeve status` to flag a heavy-lease conflict.

## Devcontainers + local-as-remote

### Devcontainer per card

- Each project supplies its own `.devcontainer/devcontainer.json` (Microsoft open spec). Daemon stays generic — no imposed base image.
- Daemon spawns the container on `→ Implementing` via `devcontainer up`. Container name: `reeve-<card-id>`.
- Container runs rootless or userns-mapped — no host root.
- v1 supports Docker; Podman is a planned follow-up using the same devcontainer.json.

### Mounts

| Host path | Container path | Purpose |
|---|---|---|
| `~/cards/<card-id>/<repo>/` | `/workspaces/<repo>/` | Per-repo clones (the agent's working tree) |
| `~/.claude/` | `~/.claude/` | Skills + CC auth — full superpowers inside |
| daemon socket | `/var/run/reeved.sock` | Agent can call `reeve` CLI from inside |

**Deliberately NOT mounted:** host home, SSH keys, GitHub credentials. The agent has no path to push to a real `origin`.

### Git topology — full clones, not worktrees

A naive worktree bind-mount breaks: the worktree's `.git` is a *file* pointing outside the mount, and git operations fail. The cleaner solution is a full clone per card:

```
Host:
  ~/Source/foo/                     ← user's normal repo, untouched
  ~/cards/<card-id>/foo/            ← full clone, container's working tree
                /.git/objects/      ← deduplicated via --reference (cheap on disk)
```

Daemon clones with `--reference ~/Source/foo` so objects are shared. The container bind-mounts the card's clone — fully self-contained, **no path back to the user's main repo**.

### Local-as-remote

Daemon fetches from card clones into the host's main repo continuously (every commit, by default):

```bash
# Daemon runs in ~/Source/foo:
git fetch ~/cards/<card-id>/foo/ <card-branch>:<card-branch>
```

The branch then lives in the user's normal repo. GitKraken / Sublime Merge / lazygit open it like any branch. Promotion to GitHub is an explicit human action: `reeve push <id>` (or the user runs `git push origin <branch>` directly). Nothing leaves the machine until then.

### Sandbox properties

What `--dangerously-skip-permissions` is safe against in this design:

- ✓ Agent cannot touch host files outside the mounts
- ✓ Agent cannot reach real `origin` (no creds present)
- ✓ Agent cannot escape to root on host (rootless / userns)
- ✓ Container CPU/memory caps prevent runaway loops
- ✗ Agent *can* corrupt the card's own clone — but branches are isolated; discarding the card recovers cleanly

## Service profile + heavy-slot scheduling

### Per-project profile

Each project has a config at `~/.config/reeve/projects/<name>.toml`:

```toml
name = "myarchy"

[[repos]]
name = "backend"
path = "~/Source/myarchy-backend"

[[repos]]
name = "frontend"
path = "~/Source/myarchy-frontend"

# Only declare services the DAEMON manages.
# Per-card services don't appear here — agent handles them in its container.

[services.db]
type = "shared"
start = "docker compose -f ~/Source/myarchy-infra/db.yml up -d"
stop  = "docker compose -f ~/Source/myarchy-infra/db.yml down"
healthcheck = "pg_isready -h localhost -p 5432"

[services.bus]
type = "shared-exclusive"
start = "docker compose -f ~/Source/myarchy-infra/bus.yml up -d"
stop  = "docker compose -f ~/Source/myarchy-infra/bus.yml down"

[limits]
max_active_cards = 4
```

### Service categories

| Category | Daemon role | Lifecycle |
|---|---|---|
| `shared` | Reference-counted; running whenever any active card in the project exists | Started when first card hits `Implementing`; stopped when last leaves |
| `shared-exclusive` | Lease-gated; only one card at a time | Started when heavy lease acquired; torn down (or idle-timer'd) on release |
| (`per-card`) | Daemon ignores — agent runs its own copies inside its container | n/a |

### Heavy-slot scheduler

- One "heavy" lease per project (parallel projects → parallel heavy cards).
- Card declares `needs_heavy = true` in its metadata. Default false. Set at card creation or updated during the Planning phase by user or agent.
- On `→ Testing`: card requests the lease.
  - **Free** → granted; daemon starts `shared-exclusive` services.
  - **Taken** → card queues; status shows "queued for heavy"; column header shows queue depth.
- On `→ Review` (or earlier explicit release): lease released → next queued card promoted.

### v1 YAGNI cuts

- One lease type only ("heavy"). Generic multi-lease system can come later.
- `max_active_cards` enforced via simple counter; per-card CPU/memory caps later.
- Lease acquired automatically on column transition; no manual "claim" button.

## Fork operations

Two distinct fork shapes serving different needs:

### `f` on a commit — commit-scoped sub-session

- Spawns a CC session in a sub-pane of the focused card's drill-in view.
- Context seeded: the parent card's plan + the specific commit's diff.
- *Read-only* with respect to the codebase — fork-chat can read and answer, cannot commit or modify files.
- Lives until the pane closes; no persistent card record.
- Use case: "why did you choose this approach?", "is this commit really needed?"

### `b` on a card — whole-session branch

- Creates a sibling card in Backlog, pre-seeded with the parent's full transcript, plan, repo selection, and container config.
- Fully independent thereafter: own clone, own container, own commits.
- Tracked as `parent_id = <parent>` in the DB for traceability.
- Use case: "what if we tried a different approach?", `/btw`-style branching, deep-dive without disrupting the parent.

## Card creation

TUI `n` opens a creation form:

- **Project** — dropdown of profiles in `~/.config/reeve/projects/`
- **Brief** — free-form textarea
- **`needs_heavy`** — checkbox (default false)

Repos involved and the detailed plan emerge during the Planning column's brainstorm + write-plan phase, *not* at creation time. This is deliberate: at creation the user often doesn't know yet which repos will be touched.

## Recovery semantics

- **Daemon kill mid-day:** tmux + containers persist; cards keep running. On restart the daemon reattaches; only un-streamed events are lost.
- **Agent error:** card flips to red, paused. User attaches via `a`, fixes or instructs, agent continues. No automatic retry in v1.
- **TUI close:** daemon and cards keep running. Reopen any time.

## v1 scope summary

### In scope

- Daemon (`reeved`) with SQLite state and IPC over Unix socket
- TUI (`reeve`) with kanban board, drill-in, diff viewer, fork operations
- CLI mode (same binary, subcommand dispatch)
- Six-column lifecycle with column-transition orchestration
- Devcontainer per card (Docker backend) with `git clone --reference` for poly-repo support
- Continuous fetch of card branches into the host main repo (local-as-remote)
- Per-project service profile with three service categories
- Heavy-slot scheduler (one lease per project)
- Mako notifications + TUI badges + card border colors for alerts
- Fork-the-chat (commit-scoped, read-only) and branch-the-card (full session fork)

### Deferred (post-v1)

- Jira / external ticket-system integration
- Podman backend (same devcontainer.json should work)
- Native GUI (Tauri / iced / egui) — second frontend over the same daemon IPC
- Multi-lease generic resource scheduler
- Per-card CPU/memory caps
- Idle-timeout enforcement for cards
- Automatic retry on agent error
- True per-card heavy-stack isolation (each card has its own full stack)

## Open implementation questions

Deliberately deferred to the writing-plans phase:

- IPC protocol details (JSON-RPC 2.0 vs simpler line-protocol; transport framing)
- SQLite schema
- Whether to depend on the `devcontainer` CLI or talk to Docker directly
- Specific Rust crates: TUI (Ratatui), tmux integration, Claude Agent SDK binding (Python/TS subprocess vs alternative)
- Notification mechanism (libnotify direct vs shelling out to `notify-send`)
- Per-card directory layout details
- Strategy for the read-only constraint on fork-the-chat sub-sessions (filesystem ACLs vs agent-side enforcement)

## Pair: Reeve + Lorekeeper

Reeve runs operations; Lorekeeper supplies standards and domain knowledge from a shared knowledge base. Inside any Reeve card, the agent can query Lorekeeper for the established way to do something. The two systems share user-facing conventions (naming, terminology, configuration paths) but develop independently for now. A merge into a unified household — Reeve handling the running estate, Lorekeeper its archive — is a plausible future direction.
