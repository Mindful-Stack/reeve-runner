# Reeve — TUI orchestrator for parallel Claude Code sessions

**Status:** Design (2026-04-29, revised 2026-04-30)

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

The system organises work by **household** — a named bundle of repos, services, a knowledge-base, and a devcontainer config, all configured per dev environment. A household is the unit a card belongs to.

Reeve **hard-requires Lorekeeper** for v1. Every card loads Lorekeeper as a CC plugin, and the household's knowledge-base repo is cloned alongside code repos so the agent can read and propose updates to it.

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
- **Tmux** sessions persist independent of the daemon.
- On daemon restart, it reads the DB, reattaches to surviving tmux sessions and containers, and resumes event streaming. `kill reeved` mid-day → cards keep running; only un-streamed events are lost.

## Lifecycle (kanban columns + transition events)

| Column | What happens | Resources |
|---|---|---|
| **Backlog** | Brief only | None |
| **Planning** | Brainstorm + write-plan in a lightweight CC session running on the host (see *Planning trust model*) | A CC process — no container yet |
| **Implementing** | Plan executes in devcontainer; light mode (compile + unit tests only) | Devcontainer up; per-card services running |
| **Testing** | Full integration; heavy lease acquired | Holds heavy lease; shared-exclusive services started |
| **Review** | Agent paused; commits ready; fork-chat available | Container kept warm |
| **Done** | Archived | Container torn down |

### Column transitions trigger orchestration

- `→ Implementing`: daemon reads the approved plan to determine which of the household's repos are involved, clones each (and the household's knowledge-base repo) into `~/cards/<id>/<repo>/` via `git clone --reference <host> --dissociate` (object-share at clone, then sever the dependency to survive host-side `git gc` or rebase-drops), and spins up the devcontainer.
- `→ Testing`: card requests heavy lease (queues if taken); daemon starts the household's `shared-exclusive` services when granted.
- `→ Review`: agent pauses, alert fires. Triggered by an agent-side signal (a `reeve mv-self review` call from inside the container) or by manual user action.
- `→ Done`: tear down container; branches stay on host (in the namespaced refspace, see *Local-as-remote*) for the user to push or discard.

Light cards may move `Implementing → Review` directly, skipping Testing.

### Reviewability is not gated by column

Diffs are always available regardless of column. The daemon fetches card branches into the host's main repo continuously. "Needs review" is a *signal* the agent sets when the work feels complete — not a gate on the user's ability to inspect.

### Planning trust model

Planning runs a CC session on the *host* (no container), with read access to the household's declared repo paths intended as the scope. v1 documents this scope as a convention but **does not technically enforce it** — the agent has full host filesystem access during Planning. Production-quality enforcement (firejail / bwrap with a generated profile) is a v2 hardening item.

Justification for the asymmetry with Implementing/Testing: Planning is short, user-supervised, and produces markdown only. The integration risk is much lower than Implementing's autonomous code edits.

## TUI layout (default frontend for v1)

### Default view: kanban board

Six columns horizontal. Each card shows: title, repos involved (chips), status icon, age. Column headers carry counts and an attention badge.

### Navigation

- Vim keys (`h/j/k/l`) and arrow keys both supported
- `Space` opens a contextual quick menu; top item is always "Move to next column" — `Space Space` is the rapid kanban-flow move
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

Pressing `a` detaches the TUI and drops the user into the card's actual CC session pty (managed by the daemon as a tmux session). Standard tmux detach returns to the TUI. This preserves the full Claude Code experience.

**Tmux as the pty backend gives free remote access:** SSH into the host, `tmux ls` shows all running cards, attach to any.

### Visual indicators

Card border colors signal state: red (error/blocked), yellow (needs review), dim grey (paused), default (healthy/running).

### Alerts

Mako desktop notifications + TUI badges, on these events:

- Agent asked a question (blocking)
- Agent errored
- Card auto-moved to Review (phase complete)
- Heavy lease granted (for a card the user queued)

Terminal bell dropped. Idle-timeout dropped from v1 (YAGNI; daemon supports the config knob, default off).

## CLI mode

Same binary, subcommand dispatches to the daemon over the same socket:

| Command | Purpose |
|---|---|
| `reeve ls` | list cards (column, status, age) |
| `reeve new --household foo --brief "..."` | create card |
| `reeve mv <id> <column>` | move card |
| `reeve attach <id>` | drop into the card's tmux session |
| `reeve diff <id>` | print diff to stdout |
| `reeve logs <id>` | tail event timeline |
| `reeve push <id>` | fast-forward `refs/heads/<branch>` from the namespaced ref, then `git push origin <branch>` |
| `reeve daemon start\|stop\|status` | daemon management |

Scripting hook: future Jira integration becomes `reeve new --jira PROJ-123 --brief "$(jira issue PROJ-123)"`.

### In-container authorization

The daemon socket is mounted into the agent's container, so the agent can call `reeve` directly. By default, the agent's calls are **self-scoped only**: `reeve mv-self`, `reeve diff-self`, `reeve status-self`, `reeve commit-self`, `reeve push-self`. Cross-card operations (`reeve diff <other-card>`, `reeve attach <other-card>`, etc.) are denied unless the household profile sets `allow_cross_card = true`. The daemon authenticates each socket call by inspecting the calling container's identity.

## Devcontainers + local-as-remote

### Devcontainer per card

- Each household supplies a `devcontainer.json` referenced from its profile (Microsoft open spec). Daemon stays generic — no imposed base image. v1 simplification: one container per card spans all of the household's repos (per-repo containers is a possible v2 refinement).
- Daemon spawns the container on `→ Implementing` via `devcontainer up`. Container name: `reeve-<card-id>`.
- Container runs rootless or userns-mapped — no host root.
- v1 supports Docker; Podman is a planned follow-up using the same devcontainer.json.

### Mounts (explicit allowlist; everything from `~/.claude/` is read-only)

| Host path | Container path | Mode | Purpose |
|---|---|---|---|
| `~/cards/<card-id>/<repo>/` | `/workspaces/<repo>/` | rw | Per-repo clones — agent's working tree (incl. KB clone) |
| `~/.claude/skills/` | `~/.claude/skills/` | ro | Skills (incl. Lorekeeper plugin) |
| `~/.claude/settings.json` | `~/.claude/settings.json` | ro | Settings (hooks suppressed inside containers via container env layer) |
| `~/.claude/CLAUDE.md` | `~/.claude/CLAUDE.md` | ro | Global instructions |
| daemon socket | `/var/run/reeved.sock` | rw | Agent calls `reeve` (self-scoped by default) |

**Deliberately NOT mounted:** host home, SSH keys, GitHub credentials, `~/.claude/.credentials.json`, anything writable from `~/.claude/`. The container authenticates its CC instance via `ANTHROPIC_API_KEY` injected by the daemon as a runtime env var. The agent has no path to push to a real `origin` and cannot mutate the host's CC config (closing both the credential-exfiltration and hook-injection vectors).

### Git topology — full clones with dissociation

A naive worktree bind-mount breaks: the worktree's `.git` is a *file* pointing outside the mount. Instead: full clone per card, object-share at clone time, then dissociate.

```
Host:
  ~/Source/foo/                     ← user's normal repo, untouched
  ~/cards/<card-id>/foo/            ← full clone (working tree for the container)
                /.git/objects/      ← copied via --reference --dissociate
```

Daemon runs `git clone --reference ~/Source/foo --dissociate ~/cards/<card-id>/foo/`. `--reference` shares objects at clone time (cheap); `--dissociate` then copies the needed objects into the clone, **severing the long-lived dependency**. The card survives host-side `git gc --prune=now` or rebase-drops cleanly.

### Local-as-remote (namespaced refs)

Daemon fetches from card clones into the host's main repo continuously, into a private namespace to avoid colliding with the user's own working branches:

```bash
# Daemon runs in ~/Source/foo:
git fetch ~/cards/<card-id>/foo/ '+refs/heads/*:refs/reeve/<card-id>/*'
```

The branches land at `refs/reeve/<card-id>/<branch>`. GitKraken / lazygit / Sublime Merge still see them; the user's `refs/heads/` namespace is never touched, so collisions with user-side rebases or checked-out branches are impossible.

Promotion to GitHub is an explicit human action: `reeve push <id>` fast-forwards `refs/heads/<branch>` from the namespaced ref, then runs `git push origin <branch>` to GitHub. Nothing leaves the machine until then.

### Sandbox properties

What `--dangerously-skip-permissions` is safe against in this design:

- ✓ Agent cannot touch host files outside the explicit mount allowlist
- ✓ Agent cannot reach real `origin` (no credentials in container)
- ✓ Agent cannot mutate host's CC config (`~/.claude/` mounted read-only; credentials never mounted)
- ✓ Agent cannot escape to root on host (rootless / userns)
- ✓ Container CPU/memory caps prevent runaway loops
- ✓ Agent cannot interfere with other cards (in-container `reeve` calls self-scoped by default)
- ✗ Agent *can* corrupt the card's own clone — but branches are isolated; discarding the card recovers cleanly

## Household profile + heavy-slot scheduling

### Per-household profile

Each household has a config at `~/.config/reeve/households/<name>.toml`:

```toml
name = "myarchy"

devcontainer = "~/Source/myarchy-ops/.devcontainer/devcontainer.json"

[knowledge_base]
path = "~/Source/myarchy-kb"
# Cloned into each card alongside code repos. Agent may read and write.

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

[security]
# Default: in-container reeve calls are self-scoped.
allow_cross_card = false
```

### Service categories

| Category | Daemon role | Lifecycle |
|---|---|---|
| `shared` | Reference-counted; running whenever any active card in the household exists | Started when first card hits `Implementing`; stopped when last leaves |
| `shared-exclusive` | Lease-gated; only one card at a time | Started when heavy lease acquired; torn down (or idle-timer'd) on release |
| (`per-card`) | Daemon ignores — agent runs its own copies inside its container | n/a |

### Adopt-in-place rule

**The daemon never stops a service it didn't start.** On daemon startup (and on each card transition that would normally start a service), the daemon runs the declared healthcheck. If the healthcheck passes, the service is marked **externally-owned** for the lifetime of that daemon session and is exempt from the reference-counted lifecycle. This avoids fighting the user's manual workflow when they run services outside Reeve.

### Heavy-slot scheduler

- One "heavy" lease per household (parallel households → parallel heavy cards).
- Card declares `needs_heavy = true` in metadata. Default false. Set at card creation or updated during the Planning phase by user or agent.
- On `→ Testing`: card requests the lease.
  - **Free** → granted; daemon starts (or adopts) `shared-exclusive` services.
  - **Taken** → card queues; status shows "queued for heavy"; column header shows queue depth.
- On `→ Review` (or earlier explicit release): lease released → next queued card promoted.

### v1 YAGNI cuts

- One lease type only ("heavy"). Generic multi-lease system can come later.
- `max_active_cards` enforced via simple counter; per-card CPU/memory caps later.
- Lease acquired automatically on column transition; no manual "claim" button.
- Plain git only (no git-town for stacked PRs); revisit if cross-card stacking becomes a workflow.
- One devcontainer per card, spanning all the household's repos; per-repo containers is a v2 refinement.

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

- **Household** — dropdown of profiles in `~/.config/reeve/households/`
- **Brief** — free-form textarea
- **`needs_heavy`** — checkbox (default false)

Repos involved and the detailed plan emerge during the Planning column's brainstorm + write-plan phase, *not* at creation time. At creation the user often doesn't know yet which repos will be touched.

## Recovery semantics

- **Daemon kill mid-day:** tmux + containers persist; cards keep running. On restart the daemon reattaches; only un-streamed events are lost.
- **Agent error:** card flips to red, paused. User attaches via `a`, fixes or instructs, agent continues. No automatic retry in v1.
- **TUI close:** daemon and cards keep running. Reopen any time.

## Pair: Reeve + Lorekeeper

Reeve hard-requires Lorekeeper for v1. Concretely:

- Each household profile declares a `knowledge_base.path` pointing to a Lorekeeper-shaped knowledge-base repo on the host.
- On `→ Implementing`, the daemon clones the KB repo into the card's working directory alongside the code repos (same `--reference --dissociate` mechanism).
- The Lorekeeper plugin is loaded in the container's CC session via the read-only mount of `~/.claude/skills/`.
- The daemon refuses to start if Lorekeeper isn't installed in `~/.claude/skills/`.
- The agent reads and writes the KB clone freely; updates flow back to the host through the same namespaced fetch as code commits and are reviewed/pushed via `reeve push <id>`.

The two systems share user-facing conventions (naming, terminology, configuration paths) and may merge into a unified household-management system later — Reeve as the running estate, Lorekeeper as its archive.

## v1 scope summary

### In scope

- Daemon (`reeved`) with SQLite state and IPC over Unix socket
- TUI (`reeve`) with kanban board, drill-in, diff viewer, fork operations
- CLI mode (same binary, subcommand dispatch); in-container self-scoped authorization
- Six-column lifecycle with column-transition orchestration
- Devcontainer per card (Docker backend) with `git clone --reference --dissociate` for poly-repo support
- Continuous fetch into namespaced refs (`refs/reeve/<card-id>/*`) on the host repo
- Per-household profile with three service categories and adopt-in-place lifecycle
- Heavy-slot scheduler (one lease per household)
- Mako notifications + TUI badges + card border colors for alerts
- Fork-the-chat (commit-scoped, read-only) and branch-the-card (full session fork)
- Hard-required Lorekeeper integration (KB clone + plugin loaded into containers)

### Deferred (post-v1)

- Jira / external ticket-system integration
- Podman backend (same devcontainer.json should work)
- Native GUI (Tauri / iced / egui) — second frontend over the same daemon IPC
- Multi-lease generic resource scheduler
- Per-card CPU/memory caps
- Idle-timeout enforcement for cards
- Automatic retry on agent error
- True per-card heavy-stack isolation
- git-town integration for stacked PRs
- Firejail/bwrap-enforced sandbox for Planning column
- Per-repo devcontainers (v1 uses one container per card spanning all repos)
- **Cross-household unified view** — the v1 TUI scopes the kanban board to a single household at a time. A future view layer should let the user see cards across *all* registered households in one kanban (or split-by-household sub-boards on one screen), so a user working on multiple unrelated projects in parallel can manage them without flipping context. Daemon already returns cards regardless of household; this is primarily a TUI presentation change with a possible `reeve ls --all-households` CLI addition.
- **Sub-card backlog (`task` concept)** — a first-class lightweight item below `card`, for bugs / chores / nits that don't warrant their own container-spawning kanban entry. Sketch: `reeve task add/ls/done/attach`, `reeve new --tasks t1,t2,...` to seed a card from tasks, a TUI tasks pane on the backlog column. Until this lands, sub-card items go in `docs/superpowers/known-issues.md` as a markdown workaround. Trigger to revisit: when the markdown gets unwieldy (~20+ items) or when a related capability (e.g. structurally surfacing `/lore:review` findings) makes the task concept naturally useful. Worth a real brainstorm — task↔card cardinality and TUI surface have several plausible shapes.
- **Reeve ↔ Lorekeeper integration (Reeve-side triggers)** — close the loop between cards and the household's lore. Concretely: on `→ Planning`, Reeve infers affected domains from the card brief (grep against lore frontmatter `tags`) and seeds the planning CC session with `/lore:prime <domain>` so the planning agent has context. On `→ Done`, Reeve runs an agent step that diffs the card's transcript + commits against existing lore and proposes new-learning candidates via `/lore:learn`. Crucially: all triggering lives Reeve-side; Lorekeeper stays a pure CC plugin and doesn't know Reeve exists (keeps the dependency one-way). The capability requires Lorekeeper's cultivation features to be mature enough for programmatic invocation; revisit once `/lore:iterate-domain` and the cultivation suite have shipped on the Lorekeeper side.

## Open implementation questions

Deliberately deferred to the writing-plans phase:

- IPC protocol details (JSON-RPC 2.0 vs simpler line-protocol; transport framing)
- SQLite schema
- Whether to depend on the `devcontainer` CLI or talk to Docker directly
- Specific Rust crates: TUI (Ratatui), tmux integration, Claude Agent SDK binding (Python/TS subprocess vs alternative)
- Notification mechanism (libnotify direct vs shelling out to `notify-send`)
- Per-card directory layout details
- Strategy for the read-only constraint on fork-the-chat sub-sessions (filesystem ACLs vs agent-side enforcement)
- How container identity is asserted to the daemon for self-scoped CLI authorization (cgroup inspection / SO_PEERCRED / unique socket path per card)
- How `~/.claude/settings.json` hooks are suppressed inside containers (env var, separate settings file, or settings layer)
- Where `ANTHROPIC_API_KEY` is sourced from when injecting into the container (host env, daemon config, secrets store)
