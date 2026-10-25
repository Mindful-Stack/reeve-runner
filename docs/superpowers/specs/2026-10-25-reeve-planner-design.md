# Reeve planner design

**Status:** Design (2026-10-25)

## Summary

Reeve becomes an agent-first planning tool that replaces Jira and Jira Product Discovery for the Grantigo team, built as the open-source successor to the current Reeve in the Witan family. It models work as initiative → epic → story, makes decision-taking a first-class phase by mirroring decision records from the repositories, computes readiness from facts instead of a hand-set label, and exposes a CLI that Lorekeeper's skills drive so that planning is work an agent does, not work an agent skips. Agent execution loops are a later milestone; the planner ships first, with the contract that loops will need pinned at the interface level only.

The planner lives in a fresh `reeve` repository. The existing repository is renamed `reeve-runner`, keeps running unchanged as the local execution tool, and becomes one pluggable executor later. The two are coupled only by the planner's HTTP API and CLI.

This document is the parent design. Each milestone gets its own spec and plan. The research behind it is in `docs/superpowers/research/2026-10-25-agent-first-planning-research.md`.

## Context

The team plans in Jira Product Discovery, whose roadmap view holds about thirty initiatives in Now, Next, Upcoming, Later and Won't do buckets, and delivers in a Scrum project with epics and stories across the five columns To Do, In Progress, Ready for review, In testing, Done. The initiative template already has a section for decisions required before building. It stays empty, because nothing checks it, and decisions end up being taken by whichever developer writes the Lorekeeper spec at implementation time. The team describes its disease as under-planning: discovery and refinement do not happen, developers work isolated areas toward individual sprint goals, and the bus factor is high.

The current Reeve is a single-user local daemon that orchestrates parallel Claude Code sessions, one card per devcontainer, with no planning hierarchy, no decisions, no users, and no scheduler. Its design deferred the Jira part explicitly. Mothra, an earlier goal-centric Jira replacement in Elixir, never shipped a roadmap or a prioritisation model while spending its effort on seats, roles and theming; its reusable lessons are separate refinement and delivery state machines, readiness enforced by rules, and blockers as records.

Research across agent-first trackers, orchestration loops, decision processes and open-source candidates found: the industry shape is a tracker as state machine plus a separate supervisor that claims work; readiness is computed, never set; records belong in the repo with the tracker pointing at them; no open-source project ships a three-level hierarchy, a roadmap and an agent API under a permissive licence.

## Goals

- Replace Jira and Jira Product Discovery entirely for the team: strategic planning view, delivery board, hierarchy, migration of existing data.
- Make decisions a first-class phase: every initiative and epic lists the decisions it requires, with a decider and a deadline, and readiness follows from their status.
- Make planning something agents do: Lorekeeper skills create epics, stories and proposed decisions through the CLI, and read packaged context from it.
- Compute readiness so that a ready story is pickable by anyone, human or agent, which is the structural fix for the bus factor.
- Design the planning model so that execution loops are additive later.

## Non-goals for this design

- Building the execution loop, the scheduler or the oversight views. Milestone two.
- Goals and metrics as entities. Later, if ever, following Mothra's sprint-as-outcome idea.
- A wiki or document store. RFCs and docs live in git under the decision in this document; the planner renders decision mirrors and initiative briefs only.
- Sprints, comments on items, a timeline view, dashboards, a refinement view. All deliberately absent from milestone one and additive.
- A hosted multi-organisation service.

## Milestones

**Milestone one: migrate off Jira.** Planning server, SvelteKit client, CLI and TUI, Jira import, decision sync, and the Lorekeeper integration with initiative, epic and story modes. Humans are the only actors. Today's Reeve keeps running on the maintainer's machine with a thin bridge: a card can be created from a story key and moving the card updates the story's delivery state through the CLI.

**Milestone two: loops.** Agent principals, the executor contract, sessions and activities, oversight views, PR tracking and budgets, possibly as human-triggered delegation first and autonomous pickup second. The executor behind the contract is chosen then: the Reeve runner, Claude Code background agents, or a hosted sandbox.

This document fully specifies milestone one and specifies milestone two at the interface level only.

## Repositories and naming

- `reeve` (new): the planner. Server, web client, CLI and TUI, OpenAPI document. Seeds carried over from the current repo: the card crate as the start of the planning crate, the protocol crate's type discipline, the architecture rules, the lore knowledge base, and the CLI and TUI crate's event, action and render structure.
- `reeve-runner` (the current repository, renamed): frozen in maintenance mode with the bridge above, gutted into a real executor at milestone two or retired.
- Both are members of one Witan household so that cross-repo changes use the stack convention. The existing `reeve-household` directory is the home, once its manifest is migrated to the current schema.
- The runner never links a planner crate. Coupling is the public HTTP API and the CLI only.

## Domain model

Crates follow bounded contexts. Within a context, foreign keys are fine; across contexts, references are by id or key only.

### Planning context

**Initiative.** Key, title, household, brief with sections vision, capabilities (must, nice, out of scope) and known knowns as Markdown, horizon bucket (Now, Next, Upcoming, Later, Won't do), target month, owner, refinement phase (draft or discovery) with a discovery-complete timestamp, labels. Progress is computed from epics on read.

**Epic.** Key, title, initiative (nullable for migration, flagged in the readiness view), brief with goal, scope and out of scope, refinement phase and discovery-complete timestamp, labels. Decided and refined are derived.

**Story.** Key, title, kind (story, bug, chore), optional epic, description, acceptance criteria as an ordered list of text items with a done flag, optional estimate, owner (user), optional delegate (user in milestone one; agent principal from milestone two), labels, typed dependency edges (blocks, relates) to other stories, linked PRs (url, repo, status), delivery state, external references (for example the Jira key), row version. Readiness is never stored.

**Readiness computation** for a story returns ready or a list of reasons from: epic decided if there is an epic; every decision linked to the story accepted; at least one acceptance criterion; no open story that blocks it. For an epic, decided means discovery complete and every linked decision accepted; refined means decided plus at least one story and criteria on every story. For an initiative, decided means the same as for an epic.

### Decision context

**Decision.** Record id (for example `kb/ADR-0022` or `portal/ADR-0007`), kind (architecture, process, tooling, product), status (proposed, accepted, rejected, deprecated, superseded, missing), home (repo and path), title, description, decider, decide-by date, body as Markdown, source commit, last synced at. Links to initiatives, epics and stories with the role required-by. Status and body are written only by sync.

### Identity context

**User.** Verified email as identity, display name, linked provider identities (GitHub, Google), role (admin, member, viewer), personal tokens.

**Agent principal** (milestone two). Name, token, the executor that owns it.

### Household context

**Household.** Name, key prefix, key sequence, manifest reference (repos, ADR homes, knowledge base). Mirrors the household manifest; the server does not clone repositories.

### Session (milestone two)

One per story and executor attempt: state (pending, active, awaiting input, complete, error, stale), executor, started and last-activity timestamps, reported cost and duration, typed activities (thought, action, elicitation, response, error) with timestamps and text.

### Activity log

One append-only table: actor (user, agent, sync, import), entity (type and key), action, payload, timestamp. Feeds the activity feed, the audit view, and the server-sent event stream.

## Lifecycles and gates

**Refinement** on initiatives and epics. Stored: phase draft or discovery, plus the discovery-complete timestamp set by the owner. Derived: decided, refined. Discovery is where humans and agents research and produce proposed decisions, each with a decider and a decide-by date. Overdue proposed decisions surface on the planning view and the decisions view.

**Delivery** on stories: todo, in progress, in review, testing, done. Send-back edges from in review and testing to in progress. Actors: humans, the agent working the story through its session (milestone two), loops (milestone two). The single gate is todo → in progress, which requires ready. A human may override with a reason, written to the activity log. An agent has no override and may not be delegated a story that is not ready. The override count per period is a first-class query so the gate can be tightened on evidence.

**Session** (milestone two): pending, active, awaiting input, complete, error, stale. Stale is computed from silence beyond a configurable threshold. Moving the story out of in progress terminates any live session.

**Decision lifecycle** is the ADR one: proposed with decider and decide-by, then accepted or rejected, later deprecated or superseded; missing when the file disappears. Written only by sync.

**Intake.** Any new story without an epic is untriaged (a nullable triaged-at timestamp). Triage applies the placement tree: broken behaviour becomes a bug under the owning epic or none; a product change finds the covering epic, else the covering initiative and a new epic, else a new initiative in Later; internal work becomes a chore. In milestone one a triage skill proposes and a human confirms.

## Topology

**Server.** One Rust binary on axum and sqlx. Crates: `reeve-protocol` (API types; the OpenAPI document is generated from them), `reeve-planning`, `reeve-decision`, `reeve-identity`, `reeve-household`, `reeve-server` (composition root, serves the API and the static client), `reeve` (CLI and TUI). Live updates are one server-sent events stream of activity-log entries with resume by last event id.

**Client.** SvelteKit built with the static adapter, served by the server binary. Types generated from the OpenAPI document; a drift fails the build on either side.

**Hosting.** One server container plus one PostgreSQL container on the existing UpCloud Docker-host pattern in Stockholm, nginx and TLS in front, backups and deploys through the existing scripts and digest pipeline.

**Tenancy.** Self-hosted, single-tenant. No organisation column. Household is the only grouping; one instance can hold several households.

**Identity.** OpenID Connect with GitHub and Google as configured providers; a user is a verified email; roles admin, member, viewer derived from manifest-declared GitHub teams when a GitHub identity is linked, otherwise set by an admin on the admin page. No password store. The CLI uses a personal token issued and revocable from the web client. Authorization by role only in milestone one.

**Keys.** One sequence per household with a short prefix, for example `GRA-412` for any level; the kind is shown beside the key. Imported Jira keys are external references that the API and CLI resolve. Artifact paths and branch names use the key.

**CLI.** `reeve` with subcommand groups `initiative`, `epic`, `story`, `decision`, `household`, `auth`, `import`, and `tui`. Every command supports `--json`. Mutations are idempotent. Exit codes map to the typed error codes. In milestone one an agent runs under the user's token, so every write is attributed to a person.

## Lorekeeper integration and CLI contract

**Direction and detection.** Lorekeeper calls Reeve, never the reverse. The household manifest gains an optional `reeve` section with the instance URL. Skills check for the section and for `reeve` on the path; absent either, they behave as today. This is a cross-tool change with witan-household's schema and Lorekeeper's lint.

**Three planning modes.** The brainstorming skill gains a level:

- Initiative mode: `reeve initiative show <key> --json`, discovery with the architect agent's survey plus research, output is updated known knowns and open questions (`reeve initiative edit`), proposed decisions drafted through recording-decisions with `required_by: [<key>]`, and proposed epics (`reeve epic create --initiative <key> --json`). No spec.
- Epic mode: epic-level discovery the same way, then refinement: stories with acceptance criteria (`reeve story create --epic <key> --criteria ...`). No spec.
- Story mode: today's spec → writing-plans → implementation, invoked against a ready story, artifacts named by the story key.

**Decision sync.** `reeve decision sync` runs from a household checkout, walks every ADR home the manifest declares (local dirs per repo and the shared dir in the knowledge base), parses front matter and body, and upserts mirrors stamped with the source commit. Three new front-matter fields: `kind`, `decide_by`, `required_by`. The `rfc` field is retired. Sync runs as a CI step on merge to main in each repo and by hand. Sync marks a record missing when its file disappears; it never deletes.

**Context packaging.** `reeve story context <key>` emits one Markdown bundle: the initiative's vision, the epic brief, every linked decision with status, kind and path, the criteria, and the dependencies. Lore prime consumes it at story start.

**Readiness as a query.** `reeve story ready <key> --json` returns the boolean and reasons. `reeve story start <key>` moves a ready story to in progress, prints reasons on refusal, and accepts `--override "<reason>"` for humans only. `reeve story ls --ready --json` lists pickable work.

**Triage.** A triage skill runs the placement tree against `reeve story ls --untriaged --json` and proposes; a human confirms with `reeve story triage <key> --epic <key>` or in the web client.

**Contract rules.** `--json` everywhere; idempotent mutations; typed exit codes; no capability in a future MCP that the CLI lacks.

## Surfaces

**Web.** In priority order: the strategic planning view (initiatives in five horizon buckets with target month, refinement badge, overdue-decision count and computed progress; drag between buckets); the decisions view (every mirrored record with kind, status, decider, deadline and required-by items, overdue first, body rendered read-only with a link to the file and the commit stamp); the delivery board (stories by delivery state with filters for household, epic, owner, kind, labels, and a ready indicator); initiative and epic pages (brief with section editing, decisions, children, activity); the story drawer, addressable by URL from any view (fields, criteria checklist, decisions, dependencies, PRs, activity, override prompt); the triage inbox; the admin page (users and roles, households, tokens).

**TUI.** A thin board client: delivery board, story drawer with readiness reasons, start and move, triage confirmation, live over the event stream. Embedded sessions return with the runner in milestone two.

**Deliberately absent in milestone one.** Timeline, refinement view, goals, dashboards, comments on items, sprints.

## Jira migration

`reeve import jira --site <url> --token-from-env JIRA_TOKEN --household <name>` reads both projects through the Jira Cloud API. Imports are keyed by Jira key and re-runnable, allowing a parallel-run period before Jira is set read-only.

- Product Discovery ideas → initiatives. Roadmap bucket → horizon; target date → target month; description split by its template headings into vision, capabilities, known knowns, open questions; "ADRs required" text kept inside open questions; delivery links → initiative-to-epic relation.
- Delivery epics → epics, parented from delivery links, orphaned and flagged otherwise.
- Stories, bugs, tasks → stories with kind story, bug, chore. Status mapped column for column. Assignee → owner by email. Labels preserved. Jira key → external reference. Rich text → Markdown. Comments → activity entries with original author and date.
- Not imported: Confluence, sprints, attachments beyond a link.

After import every initiative and epic is in draft and no decision is linked, so almost nothing is ready. That is the true baseline; the override with reason keeps work moving while discovery catches up.

## Loops interface (milestone two, interface level only)

- Agent principal: identity with a token, distinct from a user; may be a story's delegate; the owner stays human.
- Claim: `POST /stories/{key}/claim` by an executor succeeds only if the story is ready, in todo, delegated to that agent or an agent pool, and has no live session; atomically creates a pending session and moves delivery to in progress under the row version.
- Session and activities: executor posts state transitions and typed activities; server marks stale after a silence threshold; moving the story out of in progress is the stop signal, observed by the executor over the event stream.
- PRs: executor registers PRs with status; GitHub webhooks may update status later.
- Pluggable executor: anything with an agent token implementing claim, session and activities.
- Scheduling, concurrency caps and budgets live in the executor; the session carries reported cost and duration for oversight.
- Milestone-one obligations that make this additive: the delegate field, the activity log, row versions, and no stored readiness.

## Error handling

- One error envelope with typed codes: not found, not ready (with reasons), conflict (with current version), forbidden, validation, unauthenticated. The CLI maps codes to exit statuses.
- A readiness refusal is a normal response, not an error.
- Writes carry a row version; a stale write returns conflict.
- Sync and import collect per-item failures, finish what they can, and report at the end. Sync never deletes; it marks missing.
- The event stream resumes from the last event id on reconnect.

## Testing

1. Domain tests, pure and fast: readiness computation, the three state machines, placement rules, front-matter parsing against copies of real ADRs from the Grantigo knowledge base.
2. Repository tests against a real PostgreSQL started per test run, one set per context.
3. API tests through the axum router with a test database; an OpenAPI snapshot so a wire change is a deliberate diff; generated client types checked in CI.
4. CLI end-to-end against a running server with a recorded Jira export as the import fixture and a fixture household with ADR files as the sync fixture.
5. Web: a Playwright smoke across the planning view, the decisions view and the board.

## Inherited choices and their verdicts

| Existing Reeve choice | Verdict |
|---|---|
| Bounded context per crate, consumer-defined ports, private adapters, isolation lint | Keep (reeve/ADR-0005) |
| SQLite through rusqlite, async-over-sync | Drop; PostgreSQL through sqlx (reeve/ADR-0002) |
| No cross-context foreign keys, handler owns the clock | Keep (reeve/ADR-0005) |
| Unix-socket JSON lines protocol | Drop; HTTP API with generated OpenAPI (reeve/ADR-0001) |
| Per-card socket as agent identity | Keep in the runner |
| Column transition runs the workspace lifecycle synchronously | Change; server never provisions, executors report sessions (kb/ADR-0008) |
| Single column enum mirroring the agent's phase | Change; three state machines (kb/ADR-0004) |
| Per-card lock across side effects | Change; row version |
| One story, one workspace, one devcontainer | Keep in the runner |
| Reference clones, mirrored refs, no credentials in the container, host-side gh bridge | Keep (kb/ADR-0008 R5) |
| Push and PR creation human-only | Change; executor-mediated, policy-gated |
| Household manifest as the project unit | Keep |
| Hard Lorekeeper dependency, Lorekeeper unaware of Reeve | Invert (kb/ADR-0006) |
| Plugin-agnostic session prompt; artifacts named by UUID | Keep the first; artifacts by key (kb/ADR-0007) |
| Brief of 1–200 characters as the whole description | Drop |
| `needs_heavy`, `parent_id` branch lineage | Drop |
| No event log, TUI polls | Change; activity log and server-sent events |
| Ratatui TUI with embedded sessions | Keep as an HTTP client |
| Vocabulary card, column, brief | Change; initiative, epic, story, decision, state (kb/ADR-0002) |

## Open questions deferred past this design

- Executor choice for milestone two: Reeve runner, Claude Code background agents, or a hosted sandbox.
- Whether the strict gate (whole epic refined before any story starts) replaces the loose one; decided on override counts after a quarter.
- Whether a refinement view, comments on items, or sprints return; each is additive.
- Whether an MCP server wrapping the CLI is wanted.
- Whether the `reeve-household` directory or a fresh household becomes the shared ADR home for the kb records below; the household manifest there predates the schema and needs migration first.

## Decisions

**Binding records followed:** none. Neither repository has an ADR home today; creating `docs/adr/` in the planner and `adrs/` in the household knowledge base is the first task of milestone one.

**Proposed records.** All thirteen are high tier, so each gets its own ADR-only PR and implementation waits until it is accepted. Numbers are per home and provisional until the homes exist. Full drafted text follows so a later session can apply each record without this conversation.

### kb/ADR-0001

**Title:** ADR-0001: The planner and any executor live in separate repositories, coupled only by the planner's HTTP API and CLI

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0001
title: "ADR-0001: The planner and any executor live in separate repositories, coupled only by the planner's HTTP API and CLI"
description: "Reeve the planner is its own repository and every executor, including the existing Reeve runner, is a separate repository that talks to it only through the public HTTP API and CLI, because a pluggable executor contract must not privilege one implementation."
tags: [adr, reeve, repository, boundary, executor]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0008]
implements: []
rfc:
---

# ADR-0001: The planner and any executor live in separate repositories

## Status
Proposed 2026-10-25.

## Context
Reeve today is one Rust workspace in one repository: a daemon that owns a kanban board and also provisions per-card clones, devcontainers and agent sessions. The successor splits those roles. A planning server with a web client and a CLI replaces Jira for the team; execution of stories by agents is a later milestone, and the executor behind it may be the existing runner code, Claude Code's own background agents, or a hosted sandbox service. A single Cargo workspace would build separate binaries for server and runner, so deployment artefacts are not the issue. The issue is whether the planner's API is designed as a public contract or as an internal one that one co-located executor can lean on.

## Facts relied on
- F1 The current repository's six crates mix board logic (reeve-card) with provisioning (reeve-workspace) under one composition root — source: `Cargo.toml`, `lore/knowledge/general/architecture.md`.
- F2 Published orchestrators converge on a tracker-as-state-machine plus a separate supervisor that polls and claims work — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §5.
- F3 Lorekeeper's stack skill already handles coordinated changes across two repositories — source: `/home/daniel/Source/lorekeeper/skills/stack`.

## Considered options
- **One Cargo workspace, two binaries** — atomic changes across the server-runner boundary with one CI run.
- **Separate repositories in a Witan household** — the API must be good enough for a third-party executor because no executor can link a planner crate.
- **Planner only, executors always external services** — least code, but gives up the self-hosted sandbox model the runner already provides.

## Decision
Daniel Thyselius keeps the planner in a fresh repository named `reeve` and renames the existing repository to `reeve-runner`, because an executor contract that is pluggable in theory must be pluggable in practice, and co-locating one executor with the planner would let it bypass the contract.

- **R1** The runner repository MUST NOT depend on any planner crate; it MAY depend on the planner's published OpenAPI document and CLI.
- **R2** Every capability an executor needs MUST be reachable through the public HTTP API; the server MUST NOT expose executor-only private endpoints.
- **R3** Both repositories MUST be members of one Witan household so cross-repo changes use the household's stack convention.

## Consequences
- A change that touches both the API and the runner needs two PRs and a stacked merge, which is slower than one workspace.
- The planner's API is forced to be complete enough for an executor the team did not write, which is the point.
- The existing repository keeps its history under a new name; GitHub redirects the old name.

## Assumptions and invalidation triggers
- *Assumes more than one executor implementation is plausible.* Trigger: two years pass with the Reeve runner as the only executor and the cross-repo tax dominates ⇒ supersede with a merge.
- *Assumes GitHub repository renames redirect.* Trigger: external links break after the rename ⇒ note and republish links.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0008 for the executor contract itself.
```

### kb/ADR-0002

**Title:** ADR-0002: Work is modelled as initiative, epic and story, where a story is one pull request and an epic holds decisions and the breakdown

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0002
title: "ADR-0002: Work is modelled as initiative, epic and story, where a story is one pull request and an epic holds decisions and the breakdown"
description: "Reeve models work as initiative → epic → story with kinds story, bug and chore, no feature level, a story being what ships as one PR and an epic owning decisions and the story breakdown, because three levels with one artifact each is the simplest model that forces product planning upfront while keeping design just-in-time."
tags: [adr, reeve, domain, hierarchy, vocabulary]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "lorekeeper:skills/brainstorming/**", "lorekeeper:skills/writing-plans/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0004]
implements: []
rfc:
---

# ADR-0002: Work is modelled as initiative, epic and story

## Status
Proposed 2026-10-25.

## Context
The team plans in Jira Product Discovery ideas (initiatives) and delivers in a Scrum project with epics and stories. Developers plan implementation with Lorekeeper, where one brainstorming session yields one spec and one plan, and the brainstorming skill states that a spec must be focused enough for a single plan. Today one Jira epic holds several specs, and specs have grown fat because they carry product decisions that nothing above them captured. Reeve's existing Card is one spec's worth of work in one container ending in one PR per repository. The team is small, several developers are juniors, and under-planning, not over-planning, is the observed failure.

## Facts relied on
- F1 The brainstorming skill decomposes anything too large for one plan into sub-projects with one spec each — source: `/home/daniel/.claude/plugins/cache/witan/lore/3.1.1/skills/brainstorming/SKILL.md`.
- F2 A Reeve Card is "one feature's worth of work" 1:1 with a workspace and container — source: `lore/knowledge/domain/card.md`, `lore/knowledge/domain/workspace.md`.
- F3 Linear, Jira and Plane each stop at three hierarchy levels plus sub-items — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §4.
- F4 The Jira delivery board on 2026-10-25 held stories, bugs and tech-debt tasks under epics such as "Selma v.2", with no level between epic and story — source: https://grantigo-team.atlassian.net/jira/software/projects/SCRUM/boards/1.

## Considered options
- **Four levels: initiative, epic, feature as spec, story as PR slice** — no re-cut of existing epics and smaller PRs.
- **Three levels, epic is the spec, story is a plan slice** — the crispest gate, since a decided epic yields ready stories mechanically.
- **Three levels, story is the spec and the PR, epic holds decisions and breakdown** — simplest rule set and no change of grain for Reeve or Lorekeeper.

## Decision
Daniel Thyselius models work as initiative, epic and story, with a story being what ships as one pull request and an epic owning the decisions and the story breakdown, because that forces product planning to the epic while leaving implementation design just-in-time, and it is the only option that changes neither Reeve's nor Lorekeeper's unit of work.

- **R1** A story MUST be deliverable as one pull request per affected repository; a story that cannot be is split, not nested.
- **R2** Stories MUST carry a kind of story, bug or chore; the planner MUST NOT introduce a fourth hierarchy level.
- **R3** Decisions and acceptance criteria MUST be attached at the epic and story levels; a spec MUST NOT be the place where an epic's decisions are first recorded.
- **R4** The vocabulary initiative, epic, story and decision MUST be used across the API, database, CLI and UI; card is a UI word for any item on a board only.

## Consequences
- Stories are larger than Jira user stories were; a small story needs criteria but no spec, and a large one still ships as one story, using stacked PRs if required.
- Parallel agent sessions inside one spec are not expressible; if that becomes routine, an epic-level spec that generates stories is the additive extension, not a new level.
- Existing fat specs are not migrated; their product content belongs to epics going forward.

## Assumptions and invalidation triggers
- *Assumes one agent session per story is the right execution grain.* Trigger: a sustained need to run several sessions on one story's spec ⇒ supersede with the epic-level spec extension.
- *Assumes the team stays small enough that three levels suffice.* Trigger: initiatives routinely hold more than roughly twenty epics ⇒ revisit.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0003 readiness; kb/ADR-0004 state machines.
```

### kb/ADR-0003

**Title:** ADR-0003: Readiness is computed from facts, never stored, and gates only the start of work with a human-only override

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0003
title: "ADR-0003: Readiness is computed from facts, never stored, and gates only the start of work with a human-only override"
description: "A story's readiness is computed on read from its epic's decisions, its own decisions, its acceptance criteria and its blockers, never stored, and it gates the todo-to-in-progress transition with a reasoned human override and no agent override, because a stored flag rots and an unmeasured gate cannot be tightened honestly."
tags: [adr, reeve, readiness, gate, domain]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "lorekeeper:skills/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0005]
related: [kb/ADR-0004]
implements: []
rfc:
---

# ADR-0003: Readiness is computed, never stored, and gates the start of work

## Status
Proposed 2026-10-25.

## Context
The team's initiative template has a section for decisions required before building, and it stays empty, because nothing checks it. Published practice computes readiness from facts: Beads' ready query is "open with no open blockers", Mothra's refinement state machine required subtasks and no blockers, Shape Up's "solved" and Kubernetes' "implementable" encode decided-ness as a state. Nobody found phrases the gate as "every linked decision accepted", which is the gap this record fills. Definition-of-ready practices fail when they harden into a barrier the team routes around, and the team has stated that under-planning is its disease, so a gate that is bypassed silently teaches nothing.

## Facts relied on
- F1 Readiness in Beads is a query, not a field — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §4.
- F2 Reeve's `needs_heavy` flag became orphaned metadata because its enforcement point was never built — source: `docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md` §Future work.
- F3 Atlassian measured agent-authored-for tickets as agent-ready 83% of the time versus 6% for traditional ones — source: research digest §6.

## Considered options
- **Stored readiness label set by humans** — zero computation and familiar from Jira labels.
- **Computed readiness, strict gate: the whole epic refined before any story starts** — maximises upfront planning.
- **Computed readiness, loose gate on this story's facts, human override with reason, no agent override** — enforces the gate where it is cheap and measures bypasses.

## Decision
Daniel Thyselius computes a story's readiness on every read as: its epic is decided if it has one, its own linked decisions are accepted, it has acceptance criteria, and no blocking dependency is open; the computation returns reasons on failure, gates the todo-to-in-progress transition, allows a human override only with a recorded reason, and allows no override for an agent, because a computed value cannot rot and a recorded override lets the gate be tightened on evidence.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons.
- **R2** A human MAY start a non-ready story only by supplying a reason, which MUST be written to the activity log.
- **R3** An agent principal MUST NOT claim or start a non-ready story under any circumstance.
- **R4** Epic decided MUST mean discovery declared complete by its owner and every linked decision accepted; epic refined MUST additionally require at least one story and criteria on every story.

## Consequences
- Immediately after Jira import nearly nothing is ready, which is true, and overrides will be frequent at first.
- Readiness queries touch decisions, dependencies and criteria on each read; indexes on those links are mandatory.
- Tightening to the strict gate later is a one-line rule change plus a policy decision backed by override counts.

## Assumptions and invalidation triggers
- *Assumes decision status in the planner is current.* Trigger: sync lag routinely exceeds a day ⇒ add a staleness reason to the computation.
- *Assumes the loose gate is enough to change behaviour.* Trigger: override rate stays above half of starts after one quarter ⇒ supersede with the strict gate.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0004, kb/ADR-0005.
```

### kb/ADR-0004

**Title:** ADR-0004: Refinement, delivery and session are three separate state machines, and pull requests are linked records rather than a state

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0004
title: "ADR-0004: Refinement, delivery and session are three separate state machines, and pull requests are linked records rather than a state"
description: "Reeve keeps refinement (stored phase plus derived decided and refined), delivery (todo, in progress, in review, testing, done) and session (pending, active, awaiting input, complete, error, stale) as separate state machines with PRs as linked records, because one column enum was only honest when the agent was the sole actor."
tags: [adr, reeve, state-machine, domain, protocol]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002]
related: [kb/ADR-0003, kb/ADR-0008]
implements: []
rfc:
---

# ADR-0004: Three separate state machines, PRs as linked records

## Status
Proposed 2026-10-25.

## Context
Reeve's single column enum, shared by wire, database and CLI, was a deliberate simplification: the column was a projection of the one agent's self-reported phase, and a PR column made shipping an explicit act. The successor has humans, agents and a computed readiness acting on the same story, plus a planning side with its own discovery and decision lifecycle. Mothra's lesson was that keeping refinement and delivery as separate machines kept the board from lying. Linear's agent sessions carry their own states and typed activities independent of the issue's workflow state.

## Facts relied on
- F1 Reeve's columns are "a projection of the agent's self-reported state" — source: `lore/knowledge/domain/card.md`.
- F2 Mothra separated a refinement machine from a workflow machine and gated sprint entry on the former — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.
- F3 Linear agent sessions use pending, active, awaitingInput, complete, error and stale — source: research digest §4.
- F4 The Jira delivery board uses To Do, In Progress, Ready for review, In testing, Done — source: https://grantigo-team.atlassian.net/jira/software/projects/SCRUM/boards/1 on 2026-10-25.

## Considered options
- **One enum covering planning, delivery and agent phase** — simplest wire type, as Reeve has today.
- **Delivery states plus a PR column, session folded into delivery** — keeps shipping explicit on the board.
- **Three machines, PRs as linked records with their own status** — each actor writes only its own machine.

## Decision
Daniel Thyselius separates refinement, delivery and session into three state machines and represents pull requests as linked records on a story, because humans, agents and computation each need a machine they alone write, and a PR has a lifecycle of its own that a column cannot carry.

- **R1** Refinement MUST store only the phase (draft or discovery) and the discovery-complete timestamp; decided and refined MUST be derived.
- **R2** Delivery MUST be todo, in progress, in review, testing, done, with send-back edges from in review and testing to in progress; no other delivery state MAY be added without superseding this record.
- **R3** Session state MUST be written only by executors through the executor contract; the planning context MUST NOT read session internals to compute delivery.
- **R4** Moving a story out of in progress MUST terminate any live session for it.
- **R5** A PR MUST be a linked record with its own status; "shipped" MUST NOT be a delivery state.

## Consequences
- Three machines mean three sets of transition tests and a UI that composes them, which is more surface than one enum.
- The explicit "ship it" act moves from a board column into the executor contract, where loops need it anyway.
- The TUI's existing column rendering is rewritten around delivery states only.

## Assumptions and invalidation triggers
- *Assumes five delivery states cover the team's flow.* Trigger: the team needs a sixth state for a quarter ⇒ supersede R2.
- *Assumes session semantics match Linear's for executors in general.* Trigger: an executor cannot map its lifecycle onto the six states ⇒ amend the session machine via kb/ADR-0008.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0003, kb/ADR-0008.
```

### kb/ADR-0005

**Title:** ADR-0005: Decision records stay in repositories and the knowledge base; the planner mirrors them from front matter and retires the RFC artifact

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0005
title: "ADR-0005: Decision records stay in repositories and the knowledge base; the planner mirrors them from front matter and retires the RFC artifact"
description: "The planner never owns a decision: it mirrors records from each ADR home by syncing front matter plus body, writes status only from sync, adds kind, decide_by and required_by fields, and retires the RFC artifact in favour of the initiative brief and a generalised decision record, because one source of truth must be the versioned file."
tags: [adr, reeve, decisions, sync, lorekeeper, rfc]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-decision/**", "reeve:crates/reeve/src/commands/decision*", "lorekeeper:skills/recording-decisions/**", "lorekeeper:references/adr-template.md", "lorekeeper:scripts/adr-lint.js"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0006]
implements: []
rfc:
---

# ADR-0005: Decision records stay in the repositories; the planner mirrors them

## Status
Proposed 2026-10-25.

## Context
The team keeps architecture decision records beside the code they govern and shared ones in the knowledge base, with front matter carrying id, status, decided_by, reversibility, scope and relations, and an unused `rfc` field. RFCs were written separately, often with security, UX or product people, and covered both broad product questions and non-architectural hard-to-reverse choices such as tooling. Practitioners keep records in the repo and pitches or decision logs in a wiki layer, with the tracker holding links. No surveyed tool links a ticket to a record natively; teams paste URLs. Non-technical team members need to read decisions without opening GitHub.

## Facts relied on
- F1 Shared ADRs use front matter with id, status, decided_by, reversibility, blast_radius, scope, supersedes, related and `rfc` — source: `/home/daniel/Source/grantigo/knowledge-base/knowledge/adrs/0022-portal-reads.md`.
- F2 Each household repo may declare its own ADR directory and the KB a shared one — source: `/home/daniel/Source/grantigo/household.json` (`adr.localDir`, `adr.sharedDir`, `repos[].adrDir`).
- F3 Rust's RFC merge procedure writes the tracking issue into the RFC header as the canonical cross-link — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §6.

## Considered options
- **Decisions authored and stored in the planner** — one place for non-technical readers and comments.
- **Decisions in repositories only, the planner holds URLs** — zero duplication, but readiness cannot be computed without reading git.
- **Decisions in repositories, mirrored into the planner by a sync that owns status** — computed readiness with the file still the single truth.

## Decision
Daniel Thyselius keeps every decision record in its repository or knowledge-base home and has the planner mirror id, title, status, kind, decider, deadline, body and links through a CLI sync, with status writable only by sync, because readiness needs the data in the planner while the versioned file must remain the only truth.

- **R1** The planner MUST NOT allow editing a decision's status or body in its UI or API; sync MUST be the sole writer.
- **R2** Records MUST carry `required_by` listing the item keys that need them, `kind` of architecture, process, tooling or product, and `decide_by` for the proposed phase; the `rfc` field MUST be removed from the template.
- **R3** Sync MUST walk every ADR home the household manifest declares, MUST stamp each mirror with the source commit, and MUST mark a record missing rather than delete it when its file disappears.
- **R4** Broad product reasoning MUST live in the initiative brief; a hard-to-reverse non-architectural choice MUST be a decision record of the appropriate kind; no separate RFC artifact MAY be introduced.

## Consequences
- Non-technical readers see rendered mirrors that can lag the file until the next sync; the commit stamp makes the lag visible.
- Three front-matter fields and the template change are a cross-tool change to Lorekeeper and its lint.
- Discussion of a decision by non-engineers happens in the planner, and an agent or engineer writes the record PR; the file never carries the discussion.

## Assumptions and invalidation triggers
- *Assumes a CI step can run sync on merge to main in every household repo.* Trigger: a repo cannot run the step ⇒ sync runs on a schedule and staleness becomes a readiness reason.
- *Assumes non-technical participation is satisfied by reading mirrors and discussing in the planner.* Trigger: a demonstrated need to author records outside git ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0003, kb/ADR-0006.
```

### kb/ADR-0006

**Title:** ADR-0006: Lorekeeper drives Reeve through the reeve CLI when a household declares an instance; Reeve never depends on Lorekeeper

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0006
title: "ADR-0006: Lorekeeper drives Reeve through the reeve CLI when a household declares an instance; Reeve never depends on Lorekeeper"
description: "Lorekeeper skills call the reeve CLI when the household manifest declares a Reeve instance and the CLI is on the path, and Reeve neither links nor requires Lorekeeper, because the planner is the product and the plugin is one client of it; this inverts the earlier rule that Lorekeeper must not know Reeve exists."
tags: [adr, reeve, lorekeeper, integration, cli, manifest]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["lorekeeper:skills/**", "lorekeeper:scripts/**", "witan-household:schema/**", "reeve:crates/reeve/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0005, kb/ADR-0002]
implements: []
rfc:
---

# ADR-0006: Lorekeeper drives Reeve through the CLI

## Status
Proposed 2026-10-25.

## Context
Reeve's original design hard-required Lorekeeper and stated that Lorekeeper must not know Reeve exists, because Reeve was an execution loop meant to sit beside an existing planning tool. The successor is the planning tool. Initiative-first work means the brainstorming, writing-plans and recording-decisions skills must create epics and stories, attach decisions and read context from the planner. Lorekeeper already drives external tools by shelling out to CLIs such as gh. A Rust CLI with JSON output needs no shared code with a TypeScript plugin.

## Facts relied on
- F1 The 2026-04-29 design deferred Jira intake and stated Lorekeeper stays a pure plugin unaware of Reeve — source: `docs/superpowers/specs/2026-04-29-reeve-design.md` §Deferred.
- F2 Lorekeeper skills are Markdown procedures that invoke shell commands and agents — source: `/home/daniel/Source/lorekeeper/skills/*/SKILL.md`.
- F3 The household manifest is the cross-tool contract shared by Reeve, Lorekeeper and witan-household — source: `CLAUDE.md` §Manifest schema migration.

## Considered options
- **Lorekeeper unaware of Reeve; humans copy keys between tools** — no cross-tool coupling.
- **Reeve exposes an MCP server that Lorekeeper connects to** — richer tool surface for agents.
- **Reeve exposes a CLI with JSON output; Lorekeeper calls it when the manifest declares an instance** — one surface for humans, agents and skills, degrading gracefully when absent.

## Decision
Daniel Thyselius makes Lorekeeper depend on Reeve through the `reeve` CLI, gated on a `reeve` section in the household manifest and the CLI being on the path, and keeps Reeve free of any Lorekeeper dependency, because the planner is the product every client talks to and a CLI is the surface the plugin already knows how to drive.

- **R1** The household manifest schema MUST gain an optional `reeve` section with the instance URL; its absence MUST leave every Lorekeeper skill behaving as before.
- **R2** Every `reeve` command used by a skill MUST support `--json`, MUST be idempotent on repeat, and MUST exit non-zero with a typed code on refusal.
- **R3** Reeve MUST NOT require Lorekeeper to be installed to perform any planning function.
- **R4** An MCP server, if ever added, MUST wrap the same commands and MUST NOT expose capabilities the CLI lacks.

## Consequences
- The manifest schema change ripples through witan-household's validator and Lorekeeper's lint, a three-tool change.
- Skills gain a conditional branch, which is more test surface for Lorekeeper.
- Agents act under the user's token in milestone one, so every write is attributed to a person.

## Assumptions and invalidation triggers
- *Assumes a shell CLI is an adequate agent surface.* Trigger: a skill needs streaming or long-lived state the CLI cannot provide ⇒ add MCP under R4.
- *Assumes one instance per household.* Trigger: a household needs two instances ⇒ amend the manifest section.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0005, kb/ADR-0002.
```

### kb/ADR-0007

**Title:** ADR-0007: Every item takes a key from one sequence per household with a short prefix, and imported Jira keys are kept as external references

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0007
title: "ADR-0007: Every item takes a key from one sequence per household with a short prefix, and imported Jira keys are kept as external references"
description: "Initiatives, epics and stories share one key sequence per household with a short prefix, so an item is GRA-412 whatever its level, and Jira keys survive as external references, because keys outlive tools and appear in records, commits and conversations."
tags: [adr, reeve, keys, identity, migration]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "lorekeeper:references/adr-template.md"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002]
related: [kb/ADR-0005]
implements: []
rfc:
---

# ADR-0007: One key sequence per household with a prefix

## Status
Proposed 2026-10-25.

## Context
Keys are the one identifier that leaves the tool: they appear in branch names, commit messages, PR titles, decision records' `required_by` lists and conversations. Jira uses one sequence per project, so the team has PIB-10 for an idea and SCRUM-586 for a story, which tells a reader which project but not which level. Linear uses one sequence per team for every issue type. The household is the grouping unit in Reeve. Beads uses hash ids to avoid collisions under concurrent offline creation, which the planner does not need because it is a single server.

## Facts relied on
- F1 Jira Product Discovery and the delivery project use separate key spaces, PIB and SCRUM — source: https://grantigo-team.atlassian.net/jira/polaris/projects/PIB and the SCRUM board, observed 2026-10-25.
- F2 Decision records will reference items by key in `required_by` — source: kb/ADR-0005 R2.
- F3 Reeve artifacts are currently named by card UUID, which the planning-column spec called "uglier" — source: `docs/superpowers/specs/2026-05-26-planning-column-design.md` §Artifact paths.

## Considered options
- **Separate sequences per level, such as INI-, EPIC-, STORY-** — the level is readable from the key.
- **One sequence per household with a prefix** — a key is short, unambiguous and never changes if an item is re-levelled.
- **UUIDs only** — no coordination, but unusable in conversation and commits.

## Decision
Daniel Thyselius assigns every initiative, epic and story a key from one sequence per household with a short configurable prefix, and stores imported Jira keys as external references on the item, because a key must be short, stable and unique across the household and must keep resolving old links.

- **R1** Keys MUST be `<PREFIX>-<n>` with one monotonic sequence per household; numbers MUST NOT be reused.
- **R2** An item's key MUST NOT change when its level, epic or initiative changes.
- **R3** Imported items MUST keep their Jira key as an external reference that the API and CLI resolve.
- **R4** Artifact paths and branch names derived from an item MUST use its key, not its internal id.

## Consequences
- A key alone does not reveal the level; the UI and CLI show the kind beside it.
- Two households in one instance have different prefixes, so a bare number is ambiguous without one.
- Renaming a household's prefix is a migration of every derived artifact; it is chosen once.

## Assumptions and invalidation triggers
- *Assumes a single server allocates keys.* Trigger: offline or multi-server creation becomes necessary ⇒ supersede with a collision-free scheme.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0005.
```

### kb/ADR-0008

**Title:** ADR-0008: Executors are pluggable clients that claim ready stories and report sessions and activities; scheduling and budgets live in the executor

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0008
title: "ADR-0008: Executors are pluggable clients that claim ready stories and report sessions and activities; scheduling and budgets live in the executor"
description: "Any holder of an agent-principal token that implements claim, session state and typed activities over the HTTP API is an executor, with the claim guarded by readiness and row version, stop signalled by moving the story out of in progress, and scheduling and budgets kept in the executor, because the planner must not privilege one runtime."
tags: [adr, reeve, executor, loops, protocol, agents]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security]
scope: ["reeve:crates/reeve-protocol/**", "reeve:crates/reeve-identity/**", "reeve:crates/reeve-session/**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0001, kb/ADR-0003, kb/ADR-0004]
related: []
implements: []
rfc:
---

# ADR-0008: Executors are pluggable clients of the planner

## Status
Proposed 2026-10-25.

## Context
Autonomous loops are a later milestone, but the planning model shipped first must not block them. Published orchestrators share one shape: the tracker is the state machine, a supervisor polls, claims atomically, caps concurrency, times out stalls and backs off, and each task runs in an isolated workspace whose agent pushes a branch and never merges. Symphony keeps scheduling in the supervisor and stops a run when the ticket's state changes. Linear models the agent as a delegate with a typed session. The executor behind Reeve could be the existing runner, Claude Code background agents, or a hosted sandbox service, and that choice is best made with usage evidence.

## Facts relied on
- F1 Symphony polls on an interval, claims by state and labels, caps concurrency, requeues on stall, and stops a run when the ticket state changes — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §5.
- F2 Reeve's sandbox puts no credentials in the container and mediates GitHub through the host — source: `docs/superpowers/specs/2026-04-29-reeve-design.md` §Sandbox properties.
- F3 Reported failures of unattended loops are runaway cost and stuck retries, mitigated by iteration caps, stall timeouts and budgets — source: research digest §5.

## Considered options
- **Scheduler inside the planner that pushes work to registered executors** — one place for caps and budgets.
- **Executors poll and claim; scheduling and budgets inside each executor** — the planner stays a state machine and any runtime can participate.
- **No contract; the Reeve runner reads the database directly** — least API work, but binds the planner to one executor.

## Decision
Daniel Thyselius defines an executor as any client holding an agent-principal token that implements claim, session state and typed activities over the public HTTP API, with scheduling, concurrency caps and budgets inside the executor, because the planner must remain a state machine that any runtime can drive without the planner knowing how it runs.

- **R1** A claim MUST succeed only if the story is ready, in todo, delegated to that agent or an agent pool, and without a live session, and MUST atomically create a pending session and move delivery to in progress under the row version.
- **R2** Session state and activities MUST be written only through executor endpoints; the executor MUST observe the event stream and MUST tear down when the story leaves in progress.
- **R3** An agent principal MUST be distinct from a user; a story's owner MUST remain a human; agents MAY be delegates only.
- **R4** The planner MUST NOT enforce budgets or concurrency; the session MUST carry reported cost and duration so oversight can show them.
- **R5** Executors MUST NOT hold repository credentials inside the agent's sandbox; pushing and PR creation MUST pass through an executor-owned bridge.

## Consequences
- Two executors can race on the same story and one loses the claim, which is intended.
- The planner cannot stop a runaway spend by itself; the executor's budget is the only brake, and oversight shows reported cost.
- Milestone one ships the delegate field, activity log and row versions so that this contract is additive later.

## Assumptions and invalidation triggers
- *Assumes executors are trusted deployments operated by the team.* Trigger: third-party or multi-tenant executors ⇒ supersede with server-side limits.
- *Assumes polling is adequate.* Trigger: claim latency matters at scale ⇒ add push assignment without changing R1.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0001, kb/ADR-0003, kb/ADR-0004.
```

### reeve/ADR-0001

**Title:** ADR-0001: The planner is a Rust server on axum and sqlx serving a statically built SvelteKit client from the same binary

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0001
title: "ADR-0001: The planner is a Rust server on axum and sqlx serving a statically built SvelteKit client from the same binary"
description: "The planner server is Rust on axum and sqlx and serves a SvelteKit client built as static assets from the same binary, because Rust's compiler gives agents the fastest wrong-shape feedback, the TUI and runner are already Rust, and one container is the simplest thing to host."
tags: [adr, reeve, stack, rust, sveltekit]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: repo
sensitivity: []
scope: ["crates/**", "web/**"]
supersedes: []
superseded_by:
depends_on: []
related: [reeve/ADR-0002, reeve/ADR-0005]
implements: []
rfc:
---

# ADR-0001: Rust server with a statically built SvelteKit client

## Status
Proposed 2026-10-25.

## Context
The planner will be maintained by one person who writes Rust and will be worked on heavily by coding agents. The team's product backends are C# with SvelteKit clients and one Hono service, and a non-technical part of the team needs a browser interface. The existing Reeve code, including a ratatui TUI with embedded sessions, is Rust. The portal runs its client as a separate server-rendered container, which suits a public product more than an internal tool.

## Facts relied on
- F1 The portal and grant scraper backends are C# and the portal client is SvelteKit — source: `/home/daniel/Source/grantigo/portal/src/server/Grantidote.sln`, `/home/daniel/Source/grantigo/portal/src/client/package.json`.
- F2 Reeve's CLI and TUI depend only on the protocol crate — source: `lore/knowledge/general/architecture.md`.
- F3 Reeve's 2026-04-29 design rejected a TypeScript-plus-Rust hybrid for a single-developer local tool — source: `docs/superpowers/specs/2026-04-29-reeve-design.md` §Context.

## Considered options
- **C# server matching the portal backend** — the team's backend language and tooling.
- **TypeScript server sharing types with the Lorekeeper plugin** — one language for plugin, CLI and server.
- **Rust server on axum and sqlx with a static SvelteKit client served by the binary** — strictest compiler, reuse of Reeve's crates and TUI, one deployable.

## Decision
Daniel Thyselius builds the planner server in Rust on axum and sqlx and serves a SvelteKit client compiled to static assets from the same binary, because compile-time strictness is the fastest feedback loop for agents working the codebase and one binary is the simplest thing to host and reason about.

- **R1** The server MUST be a single Rust binary that serves both the API and the client assets; production MUST NOT run a Node process.
- **R2** The client MUST be built with a static adapter and MUST consume the API only through types generated from the server's OpenAPI document.
- **R3** The API's wire types MUST live in the protocol crate and the OpenAPI document MUST be generated from them, so a wire change is a diff in one place.

## Consequences
- The web client is TypeScript, so the repo has two languages regardless; the boundary is the generated types.
- No server-side rendering; first paint waits for the API, which is acceptable for an authenticated internal tool.
- Team members who write C# cannot easily contribute to the server; this was accepted knowingly.

## Assumptions and invalidation triggers
- *Assumes one maintainer for the server is acceptable.* Trigger: the planner becomes something others must maintain and no Rust capacity exists ⇒ supersede.
- *Assumes a static client suffices.* Trigger: a need for server rendering or edge auth ⇒ supersede R1.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0002, reeve/ADR-0005.
```

### reeve/ADR-0002

**Title:** ADR-0002: PostgreSQL is the planner's datastore, accessed through sqlx

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0002
title: "ADR-0002: PostgreSQL is the planner's datastore, accessed through sqlx"
description: "The planner stores everything in PostgreSQL through sqlx, replacing Reeve's SQLite-over-rusqlite, because the planner is a hosted multi-user service whose operations already run PostgreSQL and whose readiness queries join across several tables."
tags: [adr, reeve, storage, postgres]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: repo
sensitivity: []
scope: ["crates/*/src/repo.rs", "migrations/**"]
supersedes: []
superseded_by:
depends_on: [reeve/ADR-0001]
related: [reeve/ADR-0004]
implements: []
rfc:
---

# ADR-0002: PostgreSQL through sqlx

## Status
Proposed 2026-10-25.

## Context
Reeve stores its board in a single SQLite file read through rusqlite inside spawn_blocking, a pattern documented in its own knowledge base, which fits one daemon on one laptop. The planner is a hosted service with several concurrent users and, later, executors claiming work under optimistic concurrency. The team's services already run PostgreSQL in Docker with backups scripted on the UpCloud hosts. SQLite with a streaming backup would serve six users, so the choice is not forced by load.

## Facts relied on
- F1 Reeve uses rusqlite with the bundled feature and an async-over-sync pattern — source: `crates/reeved/Cargo.toml`, `lore/knowledge/frameworks/rust/`.
- F2 The portal runs with a local PostgreSQL compose file in production — source: `/home/daniel/Source/grantigo/portal/docker-compose.prod.local-postgres.yml`.
- F3 Readiness is computed on read across decisions, dependencies and criteria — source: kb/ADR-0003.

## Considered options
- **SQLite with a streaming backup** — one file, no second container, adequate for the team's size.
- **PostgreSQL through sqlx** — matches existing operations, native row versioning and concurrent writers, compile-time checked queries.

## Decision
Daniel Thyselius uses PostgreSQL accessed through sqlx as the planner's only datastore, because the planner is a hosted multi-user service, the team already operates PostgreSQL, and sqlx's compile-time query checking fits the compiler-first stack.

- **R1** Every context's repository adapter MUST target PostgreSQL through sqlx; no second storage engine MAY be introduced for planning data.
- **R2** Schema changes MUST be sqlx migrations committed with the code that needs them.
- **R3** Each bounded context MUST own its tables; cross-context references MUST be by key or id without foreign keys, per reeve/ADR-0005.

## Consequences
- Tests need a real PostgreSQL per run, which is slower than in-memory SQLite.
- The async-over-sync rusqlite pattern and its knowledge node are retired for the planner.
- A second container to operate, already the norm for the team's services.

## Assumptions and invalidation triggers
- *Assumes the UpCloud hosts keep running PostgreSQL in Docker.* Trigger: a move to a managed database or a different host model ⇒ note, R1 still holds.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0001, reeve/ADR-0005.
```

### reeve/ADR-0003

**Title:** ADR-0003: Users sign in through OpenID Connect with GitHub and Google, identified by verified email, with no password store

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0003
title: "ADR-0003: Users sign in through OpenID Connect with GitHub and Google, identified by verified email, with no password store"
description: "The planner authenticates humans through a generic OpenID Connect layer with GitHub and Google as providers, treats a verified email as the user identity, derives roles from GitHub team membership with admin override, and stores no passwords, because not every team member has GitHub and a password store is the account-management work that sank Mothra."
tags: [adr, reeve, identity, auth, oidc]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: repo
sensitivity: [security]
scope: ["crates/reeve-identity/**", "crates/reeve-server/src/auth/**", "web/src/routes/auth/**"]
supersedes: []
superseded_by:
depends_on: []
related: [reeve/ADR-0004]
implements: []
rfc:
---

# ADR-0003: OpenID Connect with GitHub and Google, no password store

## Status
Proposed 2026-10-25.

## Context
The household manifest declares GitHub teams including core-team, core-non-coders, interns and ai-researchers, and PR linking and role mapping benefit from a GitHub identity, but not every team member has a GitHub account. Everyone has a Google identity. Mothra built seat and role management and an Entra sign-in that was blocked on admin consent, and that work displaced the product's core. The CLI needs a credential that is not a browser session.

## Facts relied on
- F1 The household manifest maps GitHub teams to repository access, including a core-non-coders team — source: `/home/daniel/Source/grantigo/household.json`.
- F2 Mothra's deployment was blocked on Entra admin consent and its auth hook fell back to the first user — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.
- F3 Jira's login for the team offers Google, Microsoft, Apple and Slack alongside email — source: https://id.atlassian.com/login observed 2026-10-25.

## Considered options
- **GitHub OAuth only** — simplest, and the roles come for free; excludes members without GitHub.
- **Local accounts with passwords and invitations** — works for everyone; requires hashing, reset flows and invitation management.
- **Generic OpenID Connect with GitHub and Google, identity by verified email, roles from GitHub teams with admin override** — covers everyone without a password store.

## Decision
Daniel Thyselius authenticates humans through a generic OpenID Connect layer with GitHub and Google configured as providers, identifies a user by verified email so that both providers resolve to one user, derives roles from GitHub team membership where a GitHub identity is linked and otherwise lets an admin assign them, and stores no passwords, because that covers every team member with the least account management.

- **R1** The planner MUST NOT store password hashes or implement password reset; sign-in MUST go through a configured OpenID Connect provider.
- **R2** A user MUST be keyed by verified email; two providers presenting the same verified email MUST resolve to one user.
- **R3** Roles MUST be admin, member or viewer; a role MUST be derived from manifest-declared GitHub teams when a GitHub identity is linked and MUST be settable by an admin otherwise.
- **R4** The CLI MUST authenticate with a personal token issued from the web client, revocable there, and MUST NOT reuse the browser session.

## Consequences
- Adding a provider is configuration plus a callback; Microsoft can follow if needed.
- Email as identity trusts the provider's verification; an unverified email MUST be rejected at sign-in.
- Milestone one has no per-item access control; a viewer sees every household in the instance.

## Assumptions and invalidation triggers
- *Assumes every team member has a Google or GitHub identity.* Trigger: a member with neither ⇒ add a provider, never a password.
- *Assumes role by GitHub team is current enough.* Trigger: team membership changes faster than sign-ins refresh it ⇒ add a periodic refresh.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0004.
```

### reeve/ADR-0004

**Title:** ADR-0004: The planner is self-hosted and single-tenant, with the household as the only grouping

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0004
title: "ADR-0004: The planner is self-hosted and single-tenant, with the household as the only grouping"
description: "Reeve is deployed as one self-hosted instance per organisation with no organisation column and the household as the only grouping inside it, because a hosted multi-organisation service and its tenant scoping are effort the product does not need and a recurring source of bugs."
tags: [adr, reeve, deployment, tenancy, hosting]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: repo
sensitivity: [privacy]
scope: ["crates/**", "deploy/**"]
supersedes: []
superseded_by:
depends_on: []
related: [reeve/ADR-0003, reeve/ADR-0002]
implements: []
rfc:
---

# ADR-0004: Self-hosted, single-tenant, household as the only grouping

## Status
Proposed 2026-10-25.

## Context
The planner replaces Jira for one company and is open source as the successor to Reeve. Mothra modelled Spaces as a tenant boundary with roles per space and enforced tenant scoping by convention in every query, which produced bugs and consumed effort while the roadmap never shipped. The team's services run as Docker containers on UpCloud hosts in Stockholm so that data stays in Sweden. A household already groups a set of repositories and a knowledge base, and one instance can register several households.

## Facts relied on
- F1 Mothra's tenant scoping was by convention and soft delete was enforced only by convention, both noted as bug sources — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.
- F2 The team's infrastructure provisions Docker hosts in the se-sto1 zone so data stays in Sweden — source: `/home/daniel/Source/grantigo/infra/README.md`.
- F3 Reeve's household registry already models several households per daemon — source: `lore/knowledge/domain/household.md`.

## Considered options
- **Hosted multi-organisation service** — one deployment serves everyone; needs an organisation boundary in every table and query.
- **Self-hosted single-tenant instance, household as the only grouping** — no tenant scoping anywhere, deployment on the existing host pattern.

## Decision
Daniel Thyselius ships the planner only as a self-hosted single-tenant instance, with no organisation concept in the schema and the household as the sole grouping, because the product is a planning tool for a team that runs it and tenant scoping would be cost without a customer.

- **R1** No table MAY carry an organisation or tenant column; an instance MUST be assumed to belong to one organisation.
- **R2** Every initiative MUST belong to a household; households MUST be the only grouping the API exposes.
- **R3** The reference deployment MUST be one server container and one PostgreSQL container behind the team's existing reverse proxy, provisioned by the existing host scripts.

## Consequences
- Anyone else wanting Reeve runs their own instance; there is no hosted offering to operate or bill for.
- Access control is per instance and role, not per household, in milestone one.
- A second organisation sharing one instance is impossible by design.

## Assumptions and invalidation triggers
- *Assumes no hosted offering is wanted.* Trigger: a decision to offer Reeve as a service ⇒ supersede; the schema change is large and that is intended.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0002, reeve/ADR-0003.
```

### reeve/ADR-0005

**Title:** ADR-0005: The planner keeps one bounded context per crate, consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0005
title: "ADR-0005: The planner keeps one bounded context per crate, consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time"
description: "The planner carries Reeve's architecture rules forward as a recorded decision: one bounded context per crate, ports defined by the consuming crate, adapters private behind factories, a CI isolation lint, no foreign keys across contexts and the handler owning the clock, because these rules exist so that parallel agents can work the codebase without colliding."
tags: [adr, reeve, architecture, crates, ports]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: repo
sensitivity: []
scope: ["crates/**", "Cargo.toml", "scripts/check-context-isolation.sh"]
supersedes: []
superseded_by:
depends_on: [reeve/ADR-0001]
related: [reeve/ADR-0002]
implements: []
rfc:
---

# ADR-0005: Bounded context per crate and the rules that keep contexts apart

## Status
Proposed 2026-10-25.

## Context
Reeve's domain-driven refactor was motivated by a concrete failure: a single large handlers file meant two agents working unrelated contexts collided on the same file. The resulting rules are documented as standards prose in the knowledge base but never recorded as a decision, so a successor could drift from them without anyone noticing. The planner adds planning, decision, identity and session contexts on top of the household registry. Mothra went the other way, with heavy event modules and projections for a small tool, and paid for it in drift bugs.

## Facts relied on
- F1 The refactor's stated motivation was enabling parallel agent work on unrelated contexts — source: `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md` §Context.
- F2 The five rules and their enforcement by a grep lint are documented — source: `lore/knowledge/general/architecture.md`, `scripts/check-context-isolation.sh`.
- F3 Mothra's thirty-three event modules had two consumers and thirteen bypassing call sites — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.

## Considered options
- **One crate, modules per context** — less ceremony; nothing stops a cross-context import.
- **One crate per bounded context with consumer-defined ports, private adapters, isolation lint, no cross-context foreign keys, handler-owned time** — the compiler and CI enforce the boundaries.
- **Event-sourced contexts with projections** — full audit history; demonstrated to over-reach at this scale.

## Decision
Daniel Thyselius carries Reeve's architecture rules into the planner as binding rules, because they exist to let several agents work the codebase in parallel and that need is stronger, not weaker, in the successor.

- **R1** Each bounded context MUST be its own crate; the server MUST be the only composition root.
- **R2** A cross-context port MUST be defined in the consuming crate and implemented in the supplying crate; adapters MUST be crate-private and reachable only through factories returning trait objects.
- **R3** Tables MUST be owned by one context; foreign keys MUST NOT cross contexts; references MUST be by id or key.
- **R4** Handlers MUST own time by passing a single timestamp into repositories; repositories MUST NOT read the clock.
- **R5** CI MUST run the context-isolation lint; the activity log MUST be an append-only table, and the planner MUST NOT adopt event sourcing or asynchronous projections.

## Consequences
- More crates and more boilerplate than a single crate, accepted for parallelism.
- Rollups such as initiative progress are computed on read, which costs a query but cannot drift.
- Within the planning context, foreign keys between initiative, epic and story are permitted; the rule concerns context boundaries.

## Assumptions and invalidation triggers
- *Assumes compile-time boundaries are worth the ceremony for a one-maintainer repo worked by agents.* Trigger: the crate count exceeds what one person can hold and boundaries start being bypassed ⇒ revisit granularity, not the rules.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0001, reeve/ADR-0002.
```

