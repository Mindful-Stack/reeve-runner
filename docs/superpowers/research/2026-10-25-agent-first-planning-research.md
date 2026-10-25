# Research digest: agent-first planning tool to replace Jira (2026-10-25)

Nine research agents ran during the first brainstorming session for a Jira/JPM
successor. This file keeps their reports (lightly trimmed) so the URLs and
findings survive the chat. It is point-in-time research, not living documentation.

Sections:
1. Reeve today (knowledge base + code)
2. ADR bind: what constrains a "Reeve v2" (nothing recorded; eight one-way gaps)
3. gp-mothra (Elixir goal-centric Jira replacement) findings
4. Agent-first issue trackers
5. Autonomous ticket-to-PR agent loops
6. Decision-centric processes (RFC/ADR/Shape Up/DoR)
7. Confluence replacements (RFCs + docs)
8. Forkable OSS starting points

---

## 1. Reeve today

End-to-end flow: `reeve new` inserts a Card in Backlog. Moving Inactive→Active
(`Backlog`/`Done` → `Planning`/`Implementing`/…) calls `WorkspaceLifecycle::ensure_running`:
clones meta-repo + siblings into `~/reeve/cards/<id>/` (`git clone --reference --dissociate`),
`devcontainer up`, container labelled `reeve.card_id`, per-card Unix socket, `tmux` running
`claude`. Teardown on Active→Inactive. A fetch worker mirrors container branches into host
repos as `refs/reeve/<card-id>/*`. One container/agent/conversation persists across all active
columns. SessionStart hook injects phases + artifact paths; the agent moves itself with
`reeve mv-self` over the per-card socket (self-scoped RPCs = trust boundary; no GitHub creds
in the container).

Columns on `main`: Backlog, Planning, Implementing, Testing, Review, Done. The 2026-05-27
review-and-PR spec reorders to Backlog → Planning → Implementing → Review (human diff review)
→ Testing (human manual test) → PR → Done, implemented on unmerged branch `feat/review-pr-flow`.
`mv pr` pushes and runs `gh pr create` on the host; `card_prs` table; PR→Done manual; no GitHub
polling ("Reeve tracks the user's understanding of card state, not the literal GitHub state").

Data model: Card {id, household, brief 1–200 chars, column, needs_heavy (orphaned), parent_id,
timestamps}; Workspace 1:1 with Card; Household {name, meta_repo_path, household.json}. No
cross-context FKs. Not modelled: epics/initiatives (sub-card `task` concept only sketched,
revisit trigger "~20 items"), decisions/ADRs (Lorekeeper's job), users/auth/multi-tenancy,
event timeline, transcript store.

Architecture choices that matter for v2: local single-user daemon; SQLite via rusqlite in
spawn_blocking; newline-JSON RPC over Unix socket (not strict JSON-RPC 2.0, no size cap);
no push/streaming (TUI polls 500 ms / 400 ms); sessions are tmux-in-container PTYs reached via
docker exec (a web UI needs a PTY-over-websocket bridge); bounded-context crates with
consumer-defined ports; hard Lorekeeper dependency; original design envisioned more frontends
over the same IPC.

Hard-won lessons (commit-referenced in KB): side effects before state commit; per-card locking;
idempotent provisioning instead of cleanup-on-retry; horizontal layering blocked parallel
agents (the DDD refactor was motivated by enabling parallel agent work); warn-not-bail
regressed when the TUI embedded sessions; expensive opaque ops need progress + confirm;
fetch follows container state not card column; keep orchestrator plugin-agnostic; don't add
gating fields before the gate exists (`needs_heavy`).

Deliberately left out of Reeve: Jira intake (deferred), any planning hierarchy, `card.new-self`,
readiness/auto-advance criteria, agent loops picking up ready work (never designed; every card
is started by a human `mv`), GitHub state sync/auto-merge, cross-household board, GUI,
Reeve→Lorekeeper triggers (Lorekeeper must not know Reeve exists).

Known issues: shallow-clone sources break `--reference`; failed `ensure_running` leaves orphan
workspace+container; zombie daemons after `daemon stop`; `card.md` lore still lists old column
order on `main`.

## 2. ADR bind for "Reeve v2"

No ADR home exists (`docs/adr` absent, `lore/knowledge/adrs` absent; `docs/superpowers/` is
design history, not records). Nothing binds, nothing is ruled out. One-way choices v2 must
make, none recorded:

1. Successor vs evolution of the six-crate Rust workspace.
2. Work-item model: flat Card (1:1 container) vs initiative→epic→story hierarchy; which level
   owns a container; new aggregate = new crate; references by string id (no cross-context FKs).
3. Vocabulary: `card.md` glossary rejects *issue* ("Jira-flavoured") and reserves *task*.
4. Readiness gate semantics: column, flag, or precondition on `mv → Implementing`; how a card
   cites a decision record; does *proposed* count as ready; gate as port on Card or in daemon.
5. Identity/authority of agent loops: a new actor that calls `mv`; needs caller context,
   per-card lock, lifecycle-before-commit; orchestration inside `reeved` or external.
6. Credential/GitHub boundary for agent-opened PRs: host-owned `gh` bridge vs credentials
   nearer the agent (security, one-way in practice).
7. Persistence/topology: local SQLite over Unix socket vs shared/remote planning state.
8. Direction of Reeve↔Lorekeeper dependency once a gate reads/validates ADRs.

Already-made-but-unrecorded choices v2 inherits: Rust + bounded-context crates + inverted
ports + pub(crate) adapters + CI isolation lint; SQLite per-context tables, no FKs, handler
owns clock; daemon + thin client over Unix-socket JSON lines, per-card sockets as agent
identity; Card = one Workspace = one devcontainer spanning all household repos; six-column
enum shared by wire/DB/CLI; `--reference --dissociate` clones + `refs/reeve/*` + human-only
push; household.json manifest + hard Lorekeeper dep; plugin-agnostic SessionStart prompt;
ratatui TUI with embedded PTYs; `--dangerously-skip-permissions` safe *because of* mount
allowlist + no creds + self-scoped RPCs + host-owned gh.

Design-doc assumptions this task invalidates: Jira as intake source not native hierarchy;
sub-card `task` deferral trigger fires; "no polling, no webhooks" assumes a human closes the
loop; "'Needs review' is a signal, not a gate" vs a real readiness gate; "phase guardrails come
from the plugin, not Reeve" vs a Reeve-native readiness check.

Architect recommendation: create the ADR homes and record the inherited choices as accepted
legacy decisions, then record gaps 1, 2, 5, 6, 7 as proposed before building.

## 3. gp-mothra findings

Purpose: "goal-oriented Jira replacement" — every Initiative ties to measurable Goals, Stories
carry goal links, Scrum ceremonies are flows not ticket states. Elixir/Phoenix LiveView/
Postgres DDD monolith. One-day hack (4 devs + 1 UX) hardened for a pilot. 118 commits
2026-05-26 → 2026-09-30; feature work stopped ~2026-08-25; pilot postponed, contract ended;
handover doc written.

Model: Goal tree (parent_goal_id, no cycles) with embedded Metric {target, current, unit,
direction} + leading/lagging classification + Timeframe; SMART score. Hierarchy Initiative →
Epic → Story → SubTask (no Task level). Initiative↔Goal many-to-many with primary; Story has
≤1 goal which must be the initiative's goal or a descendant. Story has two state machines:
refinement `scoping → ready_for_refinement (≥1 subtask) → ready_for_work (no open blockers)`
and workflow `backlog → in_progress → in_review → testing → done`; sprint entry requires
`ready_for_work`. Blockers are separate records (virtual Blocked column, can spawn fix SubTask).
Epic/Initiative rollups via PubSub projection GenServers. Spaces = tenant; roles owner>admin>
member>viewer.

No priority/rank field anywhere; priority expressed only through goals. Sprint-goals are real
Goal records; dragging a story into a sprint-goal zone assigns sprint + repoints goal;
sprint goal text auto-composed. Readiness = refinement state machine; no explicit decision
concept.

UI: dashboard, refinement kanban, backlog tree, goals split-pane, initiatives, sprint daily
board, demo view, planning, story drawer, global search, story-first create. **No roadmap or
timeline view.** No agent/AI integration; only a KB search endpoint (unauthenticated).

Gaps: EpicStatusProjection never rebuilds on startup; LiveView auth falls back to first user;
no git webhook (manual "simulate PR merged"); Entra admin consent blocks sign-in.

Carry forward: sprint as outcome contract; story goal under initiative goal rule; readiness
enforced by code not set by hand; blockers as records; separate refinement vs delivery state
machines; initiative active if any work unfinished; demoable flag; strict tenant scoping.

Mistakes: heavy DDD/events for a small tool (33 event modules, no store, 2 consumers, 13
bypassing call sites); async projections caused drift (compute rollups on read); 2,370-line
`work.ex`; hack-day process left security gaps (#99–#105); soft delete by convention; large
share of effort on seats/roles/theming/Azure while roadmap + prioritisation never shipped.

## 4. Agent-first issue trackers

- **Linear**: agents are app users (May 2025); assignable, @mentionable; assigning sets
  *delegate*, human stays owner. OAuth app + webhooks; each delegation opens an **Agent
  Session** (`pending/active/awaitingInput/complete/error/stale`); agent streams typed
  activities `thought/action/elicitation/response/error`; 10-second first-activity rule.
  `promptContext` bundles issue, comments, team guidance. Hosted MCP (OAuth 2.1). Linear's own
  agent triages and opens PRs (Apr 2026). Model Initiative > Project(+Milestones) > Issue >
  Sub-issue. https://linear.app/developers/agent-interaction
- **GitHub Issues + Copilot coding agent / Agent HQ**: assign issue to Copilot/Claude/Codex/
  custom agents (`.github/agents/*.md`) → `copilot/` branch + draft PR in Actions. Copilot
  ignores later issue comments (follow-ups in PR). Multi-vendor parallel assignment Feb 2026;
  assignable from Jira Mar 2026. Projects is a view layer, no agent automation found.
- **Jira (Rovo)**: "Agents in Jira" GA May 2026 — assignee picker lists Rovo + third-party
  agents; agents bindable to workflow transitions and board columns. Pitfall: agent visibility
  global, bypasses project Assignable User permission.
- **Plane** (AGPL): Plane Agents Sep 2026 — triggers: assignment, @mention, change, schedule;
  Triage/Spec Writer/Standup agents; custom agents with scoped objectives, permitted actions,
  escalation rules; credit-metered. Official MIT MCP at mcp.plane.so.
- **Shortcut**: Shortcut for Agents May 2025; Korey PM agent expands ideas into
  Stories/Sub-tasks/Epics with acceptance criteria.
- **Huly** (EPL-2.0): no first-party agent features; community MCP over WebSocket SDK.
- **Beads** (`bd`, MIT, Go, now Dolt-backed): dependency-aware graph replacing markdown TODOs;
  hash ids; hierarchy via dotted ids; types epic/task/bug/message; deps blocks/parent-child/
  relates-to/duplicates/supersedes; **`bd ready`** = open with no open blockers; `bd update
  --claim` atomic; memory-decay compaction. https://github.com/steveyegge/beads
- **Execution boards** (one card = one agent run in one worktree): Vibe Kanban (Apache-2.0,
  Rust+React; Bloop shut Apr 2026, community-maintained), Conductor (Mac, closed), Kilo Agent
  Manager (Apache-2.0 VS Code), Terragon (shut Jan 2026), Trekker, agent-board (DAG deps +
  leases), open-taskboard (claim leases).

Recurring patterns: (1) agent as principal with distinct identity, delegate slot keeps human
accountable; (2) assignment/mention as trigger, typed session as contract, progress streamed
onto the ticket; (3) readiness computed from a dependency graph, not inferred by the model;
(4) atomic claim/lease; (5) context packaging on the ticket (promptContext, guidance,
acceptance criteria, DoR); (6) isolation per card, PR as output, human diff review as gate;
(7) scoped tool permissions / worker-planner-reviewer profiles / escalation rules; (8) dual
surface: UI for humans, CLI/MCP with JSON for agents.

Pitfalls: one-shot handoff (Copilot ignores later comments); tool heaviness and breaking
upgrades (Beads daemon/hooks/Dolt migration); permission model mismatch (Jira); vendor churn
(Terragon, Vibe Kanban); vague specs multiply across parallel agents, stuck retry loops, file
collisions, untrustworthy agent tests; DORA 2025 ties higher AI adoption to ~9% more bugs.

## 5. Autonomous ticket-to-PR loops

- **Ralph Wiggum loop** (Huntley, Jul 2025): `while true; do claude -p "$(cat PROMPT.md)"; done`
  — fresh context per iteration, state in files + git. Anthropic plugin runs it via Stop hook
  with `--completion-promise` + `--max-iterations`; README: unsuitable for human-judgment tasks,
  best with automatic verification. Anthropic harness guidance: initializer agent (init.sh,
  JSON feature list, progress log) + per-session coding agent doing one feature end-to-end;
  failure modes: premature "done", unverified features, one-shotting to context exhaustion.
  https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- **OpenAI Symphony** (Apache-2.0, Elixir reference impl, ~28k stars):
  https://github.com/openai/symphony — SPEC.md: poll tracker every 30 s; eligible = active
  state + required_labels + unclaimed; sort priority then created_at; Linear states Todo → In
  Progress → Human Review / Rework → Merging; terminal states tear down. One directory per
  issue under workspace.root with `after_create` hook; workspaces persist. `codex app-server`
  over stdio; max_turns 20; max_concurrent_agents 10 + per-state caps. Stall timeout 5 min
  kills + requeues; exponential backoff capped 5 min; restart recovers from polling + existing
  workspaces. Review/merge not defined by orchestrator; agent moves ticket to handoff state;
  optional commit/push/pull/land skills; "proof of work" (CI status, review feedback, videos).
  Oversight: WORKFLOW.md (YAML + prompt) hot-reloads; changing ticket state stops a run;
  LiveView dashboard + /api/v1; structured logs keyed by issue/session.
- **Claude Code Agent View** (`claude agents`): supervisor daemon, background sessions
  surviving shell close; worktree per session under `.claude/worktrees/`; commits/pushes, never
  merges; UI groups Ready for review / Needs input / Working / Completed; one-line summaries;
  PR badges; peek-and-reply; `claude logs/attach/stop/respawn`; `--json`.
  https://code.claude.com/docs/en/agent-view
- **Agent teams** (experimental): lead + teammates, file-locked shared task list,
  self-claiming with deps, mailbox, plan-approval gating, TaskCompleted/TeammateIdle hooks;
  3–5 teammates recommended, one-file-one-owner.
- **Managed Agents** (beta 2026-04-01): hosted harness, cloud or self-hosted sandboxes,
  persistent sessions, SSE event stream, steer/interrupt, scheduled deployments.
- **Devin**: Linear assignment / `!plan` `!implement` `!review` labels / status automation;
  activity feed into Linear agent-session UI; Devin Review + Auto-Fix for review comments and
  failing checks; GitHub auto-merge toggle; Autoapprover beta; per-PR ACU limits.
- **Codex cloud**: per-repo environments, container sandbox, `@codex review` / auto review
  (P0/P1 by default), `@codex fix it`.
- **Cursor cloud agents**: VM per agent, triggers Slack/GitHub/Linear/API, shareable run page,
  human take-over of remote desktop and hand back, spend limit.
- **Jules**: VM clone, plan approval before edits, `jules` label trigger, pause mid-task,
  scheduled tasks. **OpenHands Cloud**: label/mention triggers, Agent Canvas, budgets.
  **Factory**: Ticket-to-PR, Review Droid, Droid Computers.
- **OSS orchestrators**: Vibe Kanban; claude-squad (Go, AGPL, tmux+worktree); Conductor;
  **Paperclip** (MIT, Node/React/Postgres, optional Rust runner): company-metaphor control
  plane, ticket checkout + execution locks, wake on assignment/schedule, budgets that pause,
  approval stages, audit log https://github.com/paperclipai/paperclip ; Agetor (Rust).

Recurring architecture: (a) tracker as state machine — columns are the queue, claiming is an
atomic transition/lock; (b) scheduler/supervisor daemon: poll, concurrency caps, stall
detection, backoff, recovery from persisted state; (c) per-task workspace (worktree local,
container/VM cloud), agent pushes a branch and never merges; (d) review layer: AI reviewer
that can request changes but not approve, then human gate, then CI/merge queue; auto-merge
only where explicitly enabled; (e) hooks as policy points.

Oversight UI converges on: "needs me" first, one-line activity summary, PR/CI badge, live
transcript/activity feed mirrored into the tracker, peek-and-reply or take-over, per-task cost
with thresholds/auto-pause, structured logs keyed by ticket+session.

Where it breaks: context loss (mabl: ~40% of failures from context drift across 75 repos,
cut to <5% with shared rule base + per-repo CLAUDE.md + path verification); merge conflicts
at integration (one-file-one-owner, merge queue re-running suite); runaway cost (reported
~$6k overnight; every mature tool has max-iterations, stall timeouts, hard-pause budgets);
review fatigue (METR: ~half of test-passing AI PRs would not be merged, mostly quality/style;
mabl: AI pre-review + circuit breakers — no auto-fix after two consecutive auto-fix commits,
never retry a reverted fix — + confidence gate so agents ask rather than guess).
https://www.mabl.com/blog/how-we-built-a-system-for-ai-agents-to-ship-real-code-across-75-repos
https://metr.org/notes/2026-03-10-many-swe-bench-passing-prs-would-not-be-merged-into-main/

## 6. Decision-centric processes

- Oxide RFD states `prediscussion → ideation → discussion → published → committed → abandoned`;
  discussion in a PR; bot stamps link. Web tool renders PR comments beside paragraphs; does
  not link to tickets. https://rfd.shared.oxide.computer/rfd/0001
- Rust: RFC merge procedure creates the tracking issue first, writes its number into the RFC
  header — canonical ticket↔decision cross-link.
  https://forge.rust-lang.org/lang/rfc-merge-procedure.html
- Kubernetes KEPs: `provisional` (goals agreed) vs `implementable` (design agreed); release
  tracking refuses work until implementable.
- ADRs: Nygard statuses proposed/accepted/deprecated/superseded; MADR `docs/decisions/NNNN`
  with decision-makers/consulted/informed + "Confirmation" section; GDS: decisions affecting one
  application live in its repo. https://adr.github.io/madr/
- Shape Up: shaped work is rough, solved, bounded — open questions resolved before building;
  fixed appetite, variable scope; no backlog, betting table. https://basecamp.com/shapeup/
- Amazon PR/FAQ & 6-pager; Loomio thread → proposal → recorded outcome; Confluence Decisions
  blueprint builds a decision log.
- Gating readiness: Definition of Ready "no unresolved questions"; Relias "Needs Detail / PO
  Blocked" vs "Ready for Development". Atlassian measured 300 tickets: agent-authored-for
  tickets 83% agent-ready vs 6% traditional; "every assumption we leave for the agent to fill
  in is a cost we pay later". https://www.atlassian.com/blog/jira/writing-tickets-for-ai-agents
  A startup's `ai-ready` label: ≤3 files, acceptance criteria, <3h, single repo, no gotchas;
  LLM triage applies it. https://aicoding.substack.com/p/ai-ready-tasks
  **Nobody found phrases the gate literally as "all referenced decisions Accepted"** — it is
  implicit (Shape Up "solved", KEP "implementable").
- Ticket↔decision tooling: Rust tracking issues, KEP tracking issues, Backstage ADR plugin
  (no Jira link), JPD Delivery tab (idea → create or link epic). Nothing links a ticket to an
  ADR natively; teams paste URLs.
- Triage/intake: Linear Triage inbox (accept/merge/decline/snooze); JPD idea → prioritise →
  new epic or link existing; route-by-type intake (bug → fix path; feature → triage; strategic
  → initiative review).
- Candidate lifecycle: Intake → Triage (bug? fits epic? spawn epic) → Discovery → Decision
  pending (RFC/pitch, time-box, named decider) → Decided (record accepted; tracking issue
  linked) → Ready to build (all linked decisions accepted + acceptance criteria + bounded
  scope) → Built (RFD committed; ADR confirmation). Any stage → Abandoned (kept).
- Artifact placement per practitioners: RFC/RFD and ADR in the **repo**; pitch/PR-FAQ/decision
  log in the **wiki**; the **tracker** holds the tracking issue with links out.
- What goes wrong: no decision step → graveyard (Kaplan-Moss "Against RFCs": default outcome
  is "no", rewards writing to exhaustion; fix: named decider) https://jacobian.org/2023/dec/1/against-rfcs/ ;
  consensus by exhaustion (Attentive: lightweight template, one-week time-box,
  Draft/In Review/Approved header, addendums) https://tech.attentive.com/articles/rfc-process-for-teams ;
  ADR rot (fitness functions, "revisit if", expiry tests); discoverability (Cosmos SDK);
  process bypass (KEP implemented while provisional); DoR as bureaucracy.

## 7. Confluence replacements

Obsidian: shared vault in git = perfect fidelity, agents read files; pain = merge conflicts
and `.obsidian/` clashes; no comments except PRs. Obsidian Sync shared vaults ≤20 users, every
collaborator needs a seat on top of commercial licence. Publish: owner-only subscription, no
comments. Quartz (MIT SSG). Official Obsidian CLI since Feb 2026 (v1.12); many MCP servers.

Self-hosted wikis: Outline (BSL 1.1, Postgres/ProseMirror, built-in MCP since 2026-02, very
active); Docmost (AGPL, TipTap, community MCP); Wiki.js (two-way git sync but 2.x maintenance
only); BookStack (MIT, REST, no MCP); AFFiNE (trust concerns); AppFlowy; SiYuan; SilverBullet
(MIT, plain .md on disk, v2 Rust server, small scope).

Docs-as-code: Material for MkDocs EOL 2026-11-05 (successor Zensical) — don't start new;
Backstage TechDocs only if you run Backstage; Docusaurus 3 (MIT, MDX) and Astro Starlight (MIT,
zero-JS, Pagefind, 0.x but active); Oxide rfd-api (MPL-2.0; AsciiDoc; one branch per RFD;
heavy to run — the *process* is the reusable part).

Notion-style open: Huly hosted shut down, repo frozen, 30+ containers — avoid; Plane Pages
(API writes HTML, no MD export endpoint; official MCP); Anytype (source-available, local-first,
not a git story). Trackers embedding docs: Linear Documents (GraphQL content is Markdown;
MCP does not list documents).

Three architectures: (1) **tracker renders .md from repos** — RFCs in `docs/rfcs/`, PRs are
review, Starlight/Quartz renders, tracker links; zero lock-in, non-engineers won't open PRs.
(2) **Separate wiki** (Outline/Docmost) — comments, WYSIWYG, MCP; DB-backed, Markdown is an
export, drift. (3) **Git vault + Obsidian** — storage of (1) with Obsidian editor; multi-user
git pain, no comments. Recommendation for the stated bias: (1), or (3) as (1)-with-Obsidian;
keep RFC review in PRs, render with Starlight, point tracker at rendered URLs; Outline only
if non-engineer comments dominate.

## 8. Forkable OSS starting points

| Project | License | Stack | Hierarchy / roadmap | Agent API | Fork difficulty |
|---|---|---|---|---|---|
| Plane | AGPL CE; epics/initiatives/work-item-types **Commercial-only** | Django+React monorepo | CE 2 levels + Timeline | REST+webhooks CE; MCP hosted/local | Large; 3rd level not in AGPL code |
| Huly | EPL-2.0 | TS/Svelte, 30+ services | project→issue→sub-issue | SDK only | Very hard |
| Taiga | MPL-2.0 | Django + AngularJS (EOL) | epic→story→task, no timeline | REST+webhooks | Dead frontend |
| Focalboard | — | Go+React | boards | REST | Unmaintained since 2023 |
| Vikunja | AGPL | Go single binary + Vue | project→subproject→task→subtask; Gantt | REST + HMAC webhooks | Medium; to-do semantics |
| OpenProject | GPLv3 | Rails+Angular | unlimited hierarchy, Gantt free | APIv3 + webhooks; MCP Enterprise-only | Huge legacy |
| Leantime | AGPL | PHP | goal→milestone→to-do | JSON-RPC; MCP paid plugin | Medium |
| Kaneo | MIT | Hono+React, Postgres | workspace→project→task→subtask + deps; Gantt; no epics | REST+CLI+MCP+webhooks | Small, ~1 yr old |
| Fizzy (37signals) | O'Saasy source-available | Rails+Hotwire, SQLite | flat board→column→card | REST+webhooks; community MCP | Small; licence blocks hosted resale |
| Beads | MIT | Go/Dolt | epic→task→subtask ids, typed deps; no UI | CLI+MCP | Ledger not tracker |
| GitHub Issues | SaaS | — | sub-issues, org issue types, blocked-by (GA 2025); Projects v2 roadmap | official MIT MCP (sub_issue_write, issue_dependency_*, projects_*) | n/a |
| GitLab | SaaS/self | Rails | epics Premium; nested + roadmap Ultimate | REST/GraphQL; MCP Premium+ | n/a |
| Rust/Elixir | peisear, beads_rust, kandesk (dead) | — | hobby-grade | — | nothing to fork |

Top fork candidates: Kaneo (MIT, compact, has REST+MCP+webhooks+Gantt, missing one layer);
Plane CE (polish, but paid hierarchy and huge monorepo); Vikunja/Fizzy as small clean bases.
Top integrate-via-API: GitHub Issues + Projects v2 (best agent surface; 2 nesting levels, no
initiative); OpenProject (unlimited hierarchy free; write own MCP); Plane Cloud (cleanest
data-model match; rented). Beads pairs with any as agent-local dependency ledger.

Case for building: no OSS project ships initiative→epic→story + roadmap + agent API under a
permissive licence. If hierarchy, transitions and decision model are the product, the domain
core is small (five aggregates, state machine, event log) and Reeve already has a Rust daemon
and protocol. Expensive part is the roadmap UI: expose MCP + webhooks first, mirror cards to
GitHub Issues for a free roadmap view, defer own timeline until the loop is proven.
