# Reeve planner design

**Status:** Design (2026-10-25, revised 2026-10-26 after twelve reviews)

## Summary

Reeve is a planning tool for small software teams that work with coding agents. Work lives in three levels, initiative → epic → story, each with a key. Any item can carry required decisions, each with a responsible person and a link to a decision record in git that Reeve checks for acceptance. A story is ready when its ancestors are through discovery, nothing it requires is undecided, it has acceptance criteria and nothing blocks it. Moving a story that is not ready out of todo needs a written reason from a person. A CLI with JSON output lets agents create, place and read that same structure, and a web board plus a strategic planning view lets everyone else see it.

Grantigo is the first user: Reeve replaces Jira and Jira Product Discovery for the team. It is the open-source successor to the current Reeve in the Witan family. The planner lives in a fresh `reeve` repository; the existing repository is renamed `reeve-runner` and becomes one pluggable executor in milestone two. Lorekeeper, the team's knowledge-base plugin, gains optional planning skills that use Reeve when it is present.

This document is the parent design. Each milestone gets its own spec and plan. The research behind it is in `docs/superpowers/research/2026-10-25-agent-first-planning-research.md`.

## Context

The team plans in Jira Product Discovery, whose roadmap view holds about thirty initiatives in Now, Next, Upcoming, Later and Won't do buckets, and delivers in a Scrum project with epics and stories across five columns. The initiative template has a section for decisions required before building. It stays empty, because nothing checks it, and decisions end up being taken by whichever developer writes the Lorekeeper spec at implementation time. The team describes its disease as under-planning: discovery and refinement do not happen, developers work isolated areas, and the bus factor is high.

The current Reeve is a single-user local daemon that orchestrates parallel Claude Code sessions, one card per devcontainer, with no planning hierarchy, no decisions and no users. Its design deferred the Jira part explicitly. Mothra, an earlier goal-centric Jira replacement, never shipped a roadmap while spending its effort on seats, roles and theming.

Research across agent-first trackers, orchestration loops, decision processes and open-source candidates found: the industry shape is a tracker as state machine plus a separate supervisor that claims work; readiness is computed, never set; records belong in the repo with the tracker pointing at them; no open-source project ships a three-level hierarchy, a roadmap and an agent API under a permissive licence, and none reads a decision record's status to decide whether work may start.

## Goals

- Replace Jira and Jira Product Discovery for the team: strategic planning view, delivery board, hierarchy, comments, PR links, and a one-time import.
- Make decisions a first-class phase: every initiative and epic lists the decisions it requires, each with a responsible person, and readiness follows from whether their records are accepted.
- Make planning something agents do: Lorekeeper skills create epics, stories and required decisions through the CLI and read packaged context from it.
- Compute readiness so that a ready story is pickable by anyone, human or agent, which is the structural fix for the bus factor.
- Keep the model small enough that execution loops are additive in milestone two.

**Milestone-one acceptance test.** From a fresh instance and a repository with an ADR directory, in five minutes on the CLI: create a story and a required decision on it; `story mv` to in progress is refused with the reason "no record"; an agent drafts the record and links it; readiness now reports "proposed, PR open"; the record is accepted and merged, the check runs, and the story appears in `story ls --ready`; `story context` hands the accepted decision to an agent. The web board shows the same story go from blocked to ready.

## Non-goals

- Execution loops, scheduling and oversight views. Milestone two.
- Goals and metrics as entities.
- A wiki, a document store or a catalogue of every decision record. Records stay in git and are read there.
- Notifications. A later epic.
- Sprints, a timeline, dashboards and reports, saved filters, scoring and voting, attachments beyond a link, a refinement view.
- A hosted multi-organisation service, per-item access control, a pluggable record format, mirroring to GitHub Projects.

## Milestones

**Milestone one: migrate off Jira.** The planning server, the SvelteKit client, the CLI, the Jira import, the record check, comments, PR links, backlog rank, the Lorekeeper planning skills, and the TUI last. Humans and automation acting for a human are the only actors. Build order: server and API, planning view and board, import, required decisions and the decisions view, Lorekeeper skills, TUI.

**Milestone two: loops.** Agent principals, delegation, sessions, the executor contract, oversight views, budgets. The executor behind the contract is chosen then: the Reeve runner, Claude Code background agents, or a hosted sandbox. The interface sketch at the end of this document is what milestone one must not block.

## Repositories, binaries and naming

- `reeve` (new): the planner. Server, web client, CLI and TUI, OpenAPI document. Seeds carried over from the current repo: the card crate as the start of the planning crate, the protocol crate's type discipline, the architecture rules, the lore knowledge base, and the CLI and TUI crate's event, action and render structure. Binaries: `reeve-server` and `reeve`.
- `reeve-runner` (the current repository, renamed): untouched until milestone two, when it is gutted into an executor or retired. Its CLI binary is renamed from `reeve` to `reeve-runner` so the two can coexist on one machine; its daemon stays `reeved`.
- `reeve --version` prints `reeve-planner <semver>`, which is how Lorekeeper tells the planner from anything else on the path.
- Both repositories are members of one Witan household. The existing `reeve-household` directory becomes that household once its manifest is migrated; its knowledge base holds the shared ADR home.
- The runner never links a planner crate. Coupling is the public HTTP API and the CLI only, and every CLI read has an HTTP endpoint.

## Domain model

Three bounded contexts, planning, identity and activity, each its own crate, plus the protocol crate, the server and the CLI. Within a context foreign keys are fine; across contexts references are by id or key.

An instance serves one team. Instance configuration holds the key prefix, the list of repositories with their ADR directories, the horizon bucket names, the allowed sign-in domains and addresses, and the admin emails.

### Planning context

**Initiative.** Key, title, brief as one Markdown body (the team's template headings, vision, capabilities, known knowns, open questions, are a convention, not fields), horizon bucket, target month, owner, discovery-complete mark (timestamp and person, or none). Progress is computed on read: stories completed over stories not dropped, shown only when the denominator is positive.

**Epic.** Key, title, initiative (nullable for migration, flagged on the epic page), brief as Markdown, discovery-complete mark.

**Story.** Key, title, kind (story, bug, chore), optional epic, description, acceptance criteria as an ordered list of items each with a done flag, owner, labels, rank within todo, blocks edges to other stories, linked PRs (repository, number, url, status draft, open, merged or closed), delivery state, resolution when done (completed or dropped), external reference (the Jira key), row version. Readiness is never stored. A story of kind story with no epic is untriaged.

**Required decision.** Attached to an initiative, epic or story: title, decider, optional due date, optional record reference (repository, path, record id), optional PR url for the proposal, last check result (accepted, not accepted with the record's status, proposed with PR open, not found, or unchecked), closed-at and reason when a person closes it. A required decision is a consequential choice that blocks work; clarifications belong in the brief or the criteria.

**Comment.** On a story or a required decision: author, Markdown body, timestamp, optional external source id for imported comments. Plain and unthreaded. Comments never change state.

**Readiness.** A story is ready when: every initiative and epic above it has its discovery mark and no open unsatisfied required decision; a story of kind story has an epic; every open required decision on the story itself is satisfied; it has at least one acceptance criterion; and no story that blocks it is unfinished, where unfinished means not done. A required decision is satisfied when its last check found the record accepted on the repository's default branch. Otherwise the reason is one of: no record yet; proposed, PR open; record not found; or the record's status, with "superseded by X, re-point" when that is the status. Closed required decisions are ignored. The computation returns the reasons when it fails. Cycles in blocks edges are not prevented; a cycle leaves every story in it blocked with a visible reason.

**Keys.** One sequence with the configured prefix, so an initiative, epic or story is `GRA-412` whatever its level, and the kind is shown beside the key. Imported Jira keys are kept as external references the API and CLI resolve. Artifact paths and branch names use the key.

### Identity context

**User.** Verified email as identity, display name, role (admin or member, from configuration), disabled flag. **Token.** Owner, kind (interactive or automation), expiry, revoked-at.

### Activity context

One append-only table: actor, via (interactive or automation), entity (type and key), action, payload, timestamp. Feeds the activity feed, the override filter and the event stream. The planning and identity contexts write to it through a port.

## Lifecycles and gates

**Discovery** on initiatives and epics is a mark: the owner declares discovery complete, which stores the timestamp and person; clearing it is logged. Discovery is where humans and agents research and create required decisions. The planning view and the decisions view show every open required decision with its age, and the due date when one was set.

**Delivery** on stories is free movement between todo, in progress, in review, testing and done, with two rules. Leaving todo for in progress, in review or testing requires ready, or an override by an interactive user with a written reason, logged; automation cannot override. Entering done requires a resolution, completed or dropped; dropped is the one ungated way out of todo. Moving a story to in progress sets the owner to the mover if it has none. The override filter on the activity feed is how the lead watches the gate; tightening it is a later decision backed by that count.

**Required decisions** are open until their record is accepted, or until a person closes them with a reason. Setting or changing the record reference resets the check result and runs the check for that one decision immediately. The record's own lifecycle, proposed, accepted, rejected, superseded, lives in the file and in Lorekeeper; the planner only reads it.

**Intake.** A story of kind story without an epic is untriaged and appears in the triage filter on the board. Triage applies the placement tree: broken behaviour becomes a bug under the owning epic or none; a product change finds the covering epic, else the covering initiative and a new epic, else a new initiative in the last bucket before Won't do; internal work becomes a chore. In milestone one the triage skill proposes and a person confirms by setting the epic.

**PR links.** A GitHub webhook receives pull-request events and links a PR whose branch name starts with a story key, keeping its status current. PR status is evidence on the card and the drawer; it never moves a story. A person moves stories.

## Topology

**Server.** One Rust binary on axum and sqlx against PostgreSQL. Crates: `reeve-protocol` (API types; the OpenAPI document is generated from them), `reeve-planning`, `reeve-identity`, `reeve-activity`, `reeve-server` (composition root; serves the API and the static client; receives the GitHub webhook), `reeve` (CLI and TUI). Live updates are one server-sent events stream of activity entries that the board and the TUI subscribe to.

**Client.** SvelteKit built with the static adapter, served by the server binary. Types generated from the OpenAPI document; a drift fails the build on either side.

**Hosting.** One server container plus one PostgreSQL container on the team's existing Docker-host pattern behind its reverse proxy, with backups and deploys through the existing scripts. A documented restore procedure, rehearsed once before cut-over, with a second named person able to run it.

**Tenancy.** Self-hosted, single-tenant, one team per instance. No organisation or household column anywhere.

**Identity.** Google sign-in through OpenID Connect. Sign-in is refused unless the verified email matches a configured domain or address. Emails listed as admins in configuration are admins; everyone else is a member. An admin can disable a user, which revokes their tokens. Every authenticated request checks the current role and disabled flag. Tokens are interactive or automation: an interactive token comes from `reeve auth login`; an automation token is issued from the user's token page and passed to the CLI through an environment variable, which is how Lorekeeper skills run. Automation tokens cannot override the gate or issue tokens, and their writes are logged as the owner via automation. GitHub sign-in is added when something needs it.

**CLI.** `reeve` with subcommand groups `initiative`, `epic`, `story`, `decision`, `comment`, `auth`, `import`, and `tui`. Every command supports `--json`. A command either succeeds, is refused with reasons, or fails, and the exit status says which. Create commands used by skills are idempotent on the key or title they are given.

## Lorekeeper integration

**Direction.** Lorekeeper's planning skills are an optional part of Lorekeeper that activates when a `reeve` on the path reports itself as the planner and the user is logged in. Absent that, every skill behaves as it does today. Reeve needs nothing from Lorekeeper to run.

**Skills.** The brainstorming skill keeps its hard gate and spec checklist and gains one first step that reads `reeve story context` when the planner is present. Two new skills reuse its exploration and architect-dispatch steps and produce no spec:

- `planning-initiative`: reads the initiative, runs discovery with the architect agent's survey plus research, updates the brief, creates required decisions with a decider, proposes epics, and for each required decision drafts a record through recording-decisions.
- `refining-epic`: epic-level discovery the same way, then stories with acceptance criteria.

Recording-decisions gains one step: when it drafts a record for a required decision, it runs `reeve decision link <id> --record <record-id> --repo <name> --path <path> --pr <url>` from the checkout where the path is known. Its triage is changed so that process, tooling and product decisions are drafted as records rather than declined. The triage skill runs the placement tree against `reeve story ls --untriaged --json` and proposes placements.

**Record check.** `reeve decision check` reads, for every open required decision with a reference, the file at its path on the repository's default branch and parses `status` and `superseded_by` from front matter. It runs every ten minutes, on every link, and on demand. A file that cannot be read yields not found, or "proposed, PR open" when a PR url is set. The few legacy records without front matter get front matter once.

**Context packaging.** `reeve story context <key>` emits one bundle, as Markdown or JSON: the initiative brief, the epic brief, every required decision with its check result and record path, the criteria, the blockers, and the comments.

**Readiness as a query.** `reeve story ready <key>` returns the answer and reasons. `reeve story mv <key> in-progress` applies the gate, prints reasons on refusal, and accepts `--override "<reason>"` for interactive tokens only. `reeve story ls --ready` lists pickable work.

**Ratification.** The planner never accepts, rejects or edits a record. Discussion on a required decision is its comment thread; a person hands the outcome to an interactive Lorekeeper session, which ratifies through its own flow, and the next check picks it up.

## Surfaces

**Web.** In build order: the strategic planning view (initiatives in the configured horizon buckets, each card showing title, owner, target month, discovery mark, open required decisions with the oldest's age, and progress; drag between buckets); the delivery board (five columns; each card shows key, title, owner, kind, epic, ready or the short blocking reason, and a PR icon; filters for mine, epic, kind, untriaged; drag to rank within todo); the decisions view (every open required decision with the item it blocks, decider, age, due date if any, and check result; default filter mine); initiative and epic pages (brief, required decisions with a create button, children; on the epic page the stories' criteria are editable inline, which is where refinement happens); the story drawer, addressable by URL from any view (fields, criteria, required decisions, blockers, PRs, comments, activity, override prompt); the activity feed with an overrides filter; the token page.

**TUI.** A thin board client, built last: the board, the drawer's readiness reasons, move with the override prompt, set epic. Live over the event stream. It does not show planning, decisions or briefs.

## Jira migration

`reeve import jira --site <url> --token-from-env JIRA_TOKEN` reads both projects through the Jira Cloud API and runs once: a rehearsal on the real instance, shown to the team, then a database reset and the cut-over run on the day Jira is set read-only. The importer only creates.

- Ideas → initiatives: bucket → horizon, target date → target month, description → brief as-is, delivery links → epics.
- Epics → epics, orphaned and flagged when no idea links them.
- Stories, bugs, tasks → stories with kind. Columns map one to one; Won't Do and Duplicate → done, dropped. Assignee → owner by email. Labels preserved. Checklists and "Acceptance criteria" sections in descriptions → criteria. Jira key → external reference. Comments → comments with original author and date, keyed by Jira comment id.
- Grandfathering: stories in progress, in review or testing at import, and the epics and initiatives above them, get the discovery mark with a legacy label, and their overrides are excluded from the count.
- Rollback within two weeks is re-enabling Jira writes and abandoning the instance.
- Not imported: Confluence, sprints, attachments beyond a link.

After import, most of the backlog is blocked for want of criteria or discovery. That is the true baseline, and refining-epic is the fastest way to work it down.

## Milestone two interface sketch

What milestone one must not block, and nothing more. An agent principal is a second kind of actor with its own token, owned by an executor; a story may be delegated to one. An executor claims a ready delegated story, which creates a session with a lease and a monotonic epoch and moves the story to in progress; it heartbeats, posts typed activities, and reads a per-session stop flag that the planner sets when the story leaves in progress or a person asks. Successive attempts use branch names suffixed with the attempt number. The planner enforces a per-principal cap on live sessions, lease expiry and token revocation; scheduling and budgets live in the executor. Milestone one keeps this additive by having an activity log, row versions, an owner distinct from any future delegate, and no stored readiness.

## Error handling and testing

A response is ok, refused with reasons, or an error with a code; writes carry a row version and a stale write is refused. The import and the check finish what they can and report failures at the end. Tests: pure domain tests for readiness, progress, the two delivery rules, placement and front-matter parsing against copies of real records; repository tests against a real PostgreSQL per run; API tests through the router including the webhook and token kinds, with an OpenAPI snapshot and generated client types checked in CI; a CLI end-to-end run of the acceptance test above against a fixture repository; a Playwright smoke across the planning view, the decisions view and the board.

## Inherited choices and their verdicts

| Existing Reeve choice | Verdict |
|---|---|
| Bounded context per crate, consumer-defined ports, private adapters, isolation lint | Keep, with three contexts (reeve/ADR-0005) |
| SQLite through rusqlite | Drop; PostgreSQL through sqlx (reeve/ADR-0002) |
| No cross-context foreign keys, handler owns the clock | Keep (reeve/ADR-0005) |
| Unix-socket JSON lines protocol | Drop; HTTP API with generated OpenAPI (reeve/ADR-0001) |
| Per-card socket as agent identity; one story, one devcontainer; reference clones; no credentials in the container | Keep in the runner, untouched until milestone two |
| Column transition runs the workspace lifecycle synchronously | Change; the planner never provisions |
| Single column enum mirroring the agent's phase | Change; discovery mark plus delivery state with two rules (kb/ADR-0002) |
| Per-card lock across side effects | Change; row version |
| Hard Lorekeeper dependency, Lorekeeper unaware of Reeve | Invert; Lorekeeper optionally uses Reeve (kb/ADR-0005) |
| Brief of 1–200 characters; `needs_heavy`; `parent_id` | Drop |
| No event log, TUI polls | Change; activity log and server-sent events |
| Ratatui TUI with embedded sessions | Keep as an HTTP client |
| Vocabulary card, column; CLI binary named `reeve` | Change; initiative, epic, story, decision; the runner's CLI becomes `reeve-runner` |

## Open questions deferred past this design

- Executor choice for milestone two.
- Whether the strict gate replaces the loose one, decided on override counts after a quarter.
- Notifications; whether a refinement view or sprints return; reports.
- GitHub sign-in; a pluggable record format; mirroring to GitHub Projects; an MCP wrapper for the CLI.

## Decisions

**Binding records followed:** none. Neither repository has an ADR home today; creating `docs/adr/` in the planner and `adrs/` in the household knowledge base is the first task of milestone one.

**Proposed records.** Ten, all high tier, so each gets its own ADR-only PR and implementation waits until it is accepted. Numbers are per home and provisional until the homes exist. The executor contract is not recorded here; it is drafted when milestone two is planned. Full drafted text follows so a later session can apply each record without this conversation.

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
related: []
implements: []
rfc:
---

# ADR-0001: The planner and any executor live in separate repositories, coupled only by the planner's HTTP API and CLI

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
- The milestone-two interface sketch in the planner design for the executor contract.
```

### kb/ADR-0002

**Title:** ADR-0002: Work is initiative, epic and story under one key sequence, with a discovery mark above and five delivery states under two rules below

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0002
title: "ADR-0002: Work is initiative, epic and story under one key sequence, with a discovery mark above and five delivery states under two rules below"
description: "Reeve models work as initiative → epic → story with kinds story, bug and chore, one key sequence per instance, a discovery-complete mark on initiatives and epics, and five freely movable delivery states constrained only by a readiness rule on leaving todo and a resolution rule on entering done, because three levels with one artifact each is the simplest model that forces product planning upfront while keeping design just-in-time."
tags: [adr, reeve, domain, hierarchy, vocabulary, keys, delivery]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "lorekeeper:skills/**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0004]
implements: []
rfc:
---

# ADR-0002: Work is initiative, epic and story under one key sequence, with a discovery mark above and five delivery states under two rules below

## Status
Proposed 2026-10-25.

## Context
The team plans in Jira Product Discovery ideas and delivers in a Scrum project with epics and stories across five columns. Developers plan implementation with Lorekeeper, where one brainstorming session yields one spec sized for one plan. Specs have grown fat because they carry product decisions that nothing above them captured. The team is small, several developers are juniors, and under-planning is the observed failure. Reeve's single column enum was honest only while the agent was the sole actor; an explicit edge list for delivery kept missing cases in review; Jira keys tell a reader the project but not the level; and keys outlive tools, appearing in branches, commits and records.

## Facts relied on
- F1 The brainstorming skill decomposes anything too large for one plan into sub-projects with one spec each — source: Lorekeeper `skills/brainstorming/SKILL.md`.
- F2 Linear, Jira and Plane each stop at three hierarchy levels plus sub-items — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §4.
- F3 The Jira delivery board on 2026-10-25 used To Do, In Progress, Ready for review, In testing, Done under epics such as "Selma v.2" — source: https://grantigo-team.atlassian.net/jira/software/projects/SCRUM/boards/1.
- F4 Mothra separated a refinement machine from a workflow machine and gated sprint entry on the former — source: research digest §3.

## Considered options
- **Four levels, initiative, epic, feature as spec, story as PR slice** — no re-cut of existing epics; four levels to teach and route.
- **Three levels, epic is the spec, stories are plan slices** — the crispest gate; demands full upfront design the team does not sustain.
- **Three levels, epic holds decisions and breakdown, story is the unit one person or one agent session finishes; discovery as a mark; delivery as free movement under two rules** — simplest rule set and no change of grain for Reeve or Lorekeeper.

## Decision
Daniel Thyselius models work as initiative, epic and story with kinds story, bug and chore, keyed from one sequence per instance, with a discovery-complete mark on initiatives and epics and five freely movable delivery states under two rules, because that forces product decisions to the epic while leaving implementation design just-in-time, and because every explicit edge list reviewed was found incomplete.

- **R1** A story MUST be sized so that one person or one agent session can finish it; it MAY link to any number of pull requests; the planner MUST NOT introduce a fourth hierarchy level.
- **R2** Stories MUST carry a kind of story, bug or chore; a story of kind story without an epic MUST be treated as untriaged.
- **R3** Discovery on initiatives and epics MUST be a mark holding timestamp and person; clearing it MUST be logged; no other refinement state MAY be stored.
- **R4** Delivery MUST be todo, in progress, in review, testing, done, freely movable, with exactly two rules: leaving todo for an active state MUST satisfy readiness or an interactive override with a written reason, and entering done MUST carry a resolution of completed or dropped; pull-request status MUST NOT move a story.
- **R5** Keys MUST be `<PREFIX>-<n>` from one monotonic sequence per instance, MUST NOT change when an item moves level or parent, and MUST be used in artifact paths and branch names; imported Jira keys MUST be kept as external references.
- **R6** The vocabulary initiative, epic, story and decision MUST be used across API, database, CLI and UI.

## Consequences
- Stories can be larger than Jira user stories were; a small story needs criteria but no spec.
- A story can be dragged backwards or forwards without ceremony; the two rules are the whole policy.
- A key alone does not reveal the level; the UI and CLI show the kind beside it.
- Existing fat specs are not migrated; their product content belongs to epics going forward.

## Assumptions and invalidation triggers
- *Assumes one agent session per story is the right execution grain.* Trigger: a sustained need to run several sessions on one story ⇒ supersede with an epic-level spec that generates stories.
- *Assumes five delivery states and two resolutions cover the team's flow.* Trigger: the team needs a sixth state or a third resolution for a quarter ⇒ supersede R4.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0003, kb/ADR-0004.
```

### kb/ADR-0003

**Title:** ADR-0003: Readiness is computed from facts, never stored, and gates every exit from todo except dropping, with an interactive-only override

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0003
title: "ADR-0003: Readiness is computed from facts, never stored, and gates every exit from todo except dropping, with an interactive-only override"
description: "A story's readiness is computed on read from its ancestors' discovery marks and required decisions, its own required decisions, its acceptance criteria and its unfinished blockers, never stored, and it gates leaving todo for any active state with a written override for interactive users only, because a stored flag rots and an unmeasured gate cannot be tightened honestly."
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
depends_on: [kb/ADR-0002, kb/ADR-0004]
related: []
implements: []
rfc:
---

# ADR-0003: Readiness is computed from facts, never stored, and gates every exit from todo except dropping, with an interactive-only override

## Status
Proposed 2026-10-25.

## Context
The team's initiative template has a section for decisions required before building, and it stays empty, because nothing checks it. Published practice computes readiness from facts: Beads' ready query is "open with no open blockers", Shape Up's "solved" and Kubernetes' "implementable" encode decided-ness as a state. Nobody found phrases the gate as "every required decision accepted", which is the gap this record fills. Definition-of-ready practices fail when they harden into a barrier the team routes around, so a gate that is bypassed silently teaches nothing. Automation acting under a person's token must not be able to bypass the gate, or the bypass count measures nothing. Review found that a gate on entering in progress alone could be sidestepped by moving to review or testing, and that named maturity levels (decided, refined) were vocabulary nobody needed.

## Facts relied on
- F1 Readiness in Beads is a query, not a field — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §4.
- F2 Reeve's `needs_heavy` flag became orphaned metadata because its enforcement point was never built — source: `docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md` §Future work.
- F3 Atlassian measured agent-authored-for tickets as agent-ready 83% of the time versus 6% for traditional ones — source: research digest §6.

## Considered options
- **Stored readiness label set by humans** — zero computation and familiar from Jira labels.
- **Computed readiness, strict gate: the whole epic refined before any story starts** — maximises upfront planning.
- **Computed readiness, loose gate on this story's facts and its ancestors, interactive override with a written reason, no automation or agent override** — enforces the gate where it is cheap and measures bypasses.

## Decision
Daniel Thyselius computes a story's readiness on every read as: every initiative and epic above it has its discovery mark and no open unsatisfied required decision, a story of kind story has an epic, every open required decision on the story is satisfied, it has at least one acceptance criterion, and no story that blocks it is unfinished; the computation returns reasons on failure, gates leaving todo for any active state, allows an override only from an interactive token with a written reason, and allows none for automation or agents, because a computed value cannot rot and a recorded override lets the gate be tightened on evidence.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons; no named maturity level other than ready MAY be exposed on stories.
- **R2** An interactive user MAY move a non-ready story out of todo into an active state only by supplying a written reason, which MUST be written to the activity log; automation tokens and agent principals MUST NOT override.
- **R3** A required decision MUST be satisfied only when its last check found the referenced record accepted on the repository's default branch; no record, proposed with PR open, not found, and any other status MUST each yield a distinct reason; a closed required decision MUST be ignored.
- **R4** A story of kind story without an epic MUST be not ready with reason untriaged; bugs and chores MAY be ready without an epic. Unfinished MUST mean delivery not done. Cycles in blocks edges MUST NOT be prevented; they MUST leave every story in the cycle blocked with a visible reason.

## Consequences
- Immediately after Jira import most of the backlog is blocked, which is true; the importer grandfathers in-flight work and extracts criteria where Jira had them.
- Readiness queries touch ancestors, required decisions, dependencies and criteria on each read; indexes on those links are mandatory.
- Tightening to the strict gate later is a rule change plus a policy decision backed by the override filter on the activity feed, which the lead owns.

## Assumptions and invalidation triggers
- *Assumes the record check runs often enough that results are current.* Trigger: staleness between acceptance and check is routinely noticed ⇒ shorten the schedule and surface check age.
- *Assumes the loose gate is enough to change behaviour.* Trigger: override rate stays above half of starts after one quarter ⇒ supersede with the strict gate.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0004.
```

### kb/ADR-0004

**Title:** ADR-0004: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks the record's status on the default branch

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0004
title: "ADR-0004: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks the record's status on the default branch"
description: "The planner never owns a decision: it tracks required decisions as its own objects with a decider, links each to a record by repository, path and id without writing anything into the record, and a scheduled check reads the file's front-matter status on the default branch, because the versioned file must stay the single truth and accepted records are locked."
tags: [adr, reeve, decisions, lorekeeper, rfc]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve/src/commands/decision*", "lorekeeper:skills/recording-decisions/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0005]
implements: []
rfc:
---

# ADR-0004: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks the record's status on the default branch

## Status
Proposed 2026-10-25.

## Context
The team keeps architecture decision records beside the code they govern and shared ones in the knowledge base, with front matter carrying status and relations. Lorekeeper locks an accepted record to five permitted edits, and a proposal's PR merges only once the record is accepted, so proposed records exist only on open branches. RFCs were written separately, often with security, UX or product people, and covered both broad product questions and non-architectural hard-to-reverse choices. Practitioners keep records in the repo and the tracker holds a tracking item that links out; Rust's RFC process is the canonical example. An earlier draft of this design mirrored every record, including proposals on branches, and review found that this produced status-mapping, branch-scanning and snapshot-ordering machinery out of proportion to the problem; a later draft cached record bodies and commit stamps, which review found unnecessary since the team reads records on GitHub.

## Facts relied on
- F1 Shared ADRs use front matter with id, status, supersedes and superseded_by — source: `/home/daniel/Source/grantigo/knowledge-base/knowledge/adrs/0022-portal-reads.md`.
- F2 Accepted records are locked to five edits and a proposal merges only once accepted — source: Lorekeeper `skills/recording-decisions/reference/lifecycle.md` on origin/main.
- F3 Non-coders on the team have read access to the knowledge-base repository — source: `/home/daniel/Source/grantigo/household.json` team access.
- F4 Rust's RFC merge procedure creates the tracking issue first and links it from the RFC — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §6.

## Considered options
- **Decisions authored and stored in the planner** — one place for non-technical readers; two sources of truth.
- **Mirror every record from every home, including proposals on branches** — a complete catalogue; heavy sync machinery and collisions between canonical and proposed versions.
- **Required decisions as planner objects, each linked to one record by path, checked for status on the default branch** — readiness from one lookup, the file untouched, nothing to scan.

## Decision
Daniel Thyselius keeps every decision record in its repository or knowledge-base home, tracks the decisions an item requires as planner-owned objects with a decider and an optional due date, links each to a record by repository, path and id written only on the planner side, and runs a check that reads the file's front-matter status on the default branch, because readiness needs one fact per required decision and the versioned file must remain the only truth and must not be edited to add links.

- **R1** The planner MUST NOT edit, accept, reject or cache a record; the only record data it stores is the check result and the reference.
- **R2** A required decision MUST be a planner-owned object with title and decider, an optional due date, attached to an initiative, epic or story, optionally referencing one record by repository, path and id and one PR url; closing it MUST be a logged action with a reason; a required decision MUST represent a consequential choice that blocks work, not a clarification.
- **R3** Setting or changing a reference MUST reset the check result and trigger an immediate check of that required decision; no item keys MAY be written into records.
- **R4** The check MUST read each referenced file on the repository's default branch and parse `status` and `superseded_by` from front matter, MUST report not found when the file cannot be read and "proposed, PR open" when a PR url is set, and MUST NOT walk ADR directories or depend on Lorekeeper.
- **R5** Broad product reasoning MUST live in the initiative brief; a hard-to-reverse non-architectural choice MUST be a decision record, and Lorekeeper's record triage MUST draft process, tooling and product records rather than decline them; no separate RFC artifact MAY be introduced.

## Consequences
- Readers follow the link to GitHub for a record's text; the planner shows only what is open, who owns it and whether it is accepted.
- A superseded or rejected record shows as a reason on the required decision, and a person re-points or closes it; nothing is unready forever by accident.
- The Lorekeeper changes are the record triage and one link step at draft time.

## Assumptions and invalidation triggers
- *Assumes a check every ten minutes is frequent enough.* Trigger: staleness between acceptance and check is routinely noticed ⇒ run the check from a merge webhook.
- *Assumes reading records on GitHub is acceptable to non-coders.* Trigger: a demonstrated need to read or author records outside git ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0003, kb/ADR-0005.
```

### kb/ADR-0005

**Title:** ADR-0005: Lorekeeper optionally drives Reeve through the reeve CLI when it is present; Reeve never depends on Lorekeeper

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0005
title: "ADR-0005: Lorekeeper optionally drives Reeve through the reeve CLI when it is present; Reeve never depends on Lorekeeper"
description: "Lorekeeper's planning skills are an optional part of Lorekeeper that activates when a reeve on the path reports itself as the planner and the user is logged in, implemented as new planning-initiative and refining-epic skills plus one step each in brainstorming and recording-decisions, and Reeve neither links nor requires Lorekeeper, because the planner is the product and the plugin is one client of it; this inverts the earlier rule that Lorekeeper must not know Reeve exists."
tags: [adr, reeve, lorekeeper, integration, cli]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["lorekeeper:skills/**", "reeve:crates/reeve/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0004, kb/ADR-0002]
implements: []
rfc:
---

# ADR-0005: Lorekeeper optionally drives Reeve through the reeve CLI when it is present; Reeve never depends on Lorekeeper

## Status
Proposed 2026-10-25.

## Context
Reeve's original design hard-required Lorekeeper and stated that Lorekeeper must not know Reeve exists, because Reeve was an execution loop meant to sit beside an existing planning tool. The successor is the planning tool, and using it should be an optional part of Lorekeeper that gets more value out of the knowledge base. Initiative-first work means Lorekeeper must create epics, stories and required decisions, link drafted records to them, and read context from the planner. Lorekeeper already drives external tools by shelling out to CLIs such as gh. The brainstorming skill has a hard gate and a checklist that ends in a spec and a hand-off to writing-plans, which planning at the initiative or epic level must not produce. The current Reeve installs a `reeve` binary with different commands. An earlier draft added a manifest section validated across three tools; review found an environment-free detection on the path and the user's login to be enough.

## Facts relied on
- F1 The 2026-04-29 design deferred Jira intake and stated Lorekeeper stays a pure plugin unaware of Reeve — source: `docs/superpowers/specs/2026-04-29-reeve-design.md` §Deferred.
- F2 The brainstorming skill's hard gate and checklist force a spec and a writing-plans hand-off — source: Lorekeeper `skills/brainstorming/SKILL.md` on origin/main.
- F3 The current Reeve workspace builds a `reeve` CLI binary — source: `crates/reeve/Cargo.toml`.

## Considered options
- **Lorekeeper unaware of Reeve; humans copy keys between tools** — no cross-tool coupling.
- **Reeve exposes an MCP server that Lorekeeper connects to** — richer tool surface for agents.
- **Reeve exposes a CLI with JSON output; Lorekeeper's planning skills use it when it is present** — one surface for humans, agents and skills, degrading gracefully when absent.

## Decision
Daniel Thyselius makes Lorekeeper's planning skills an optional capability that uses the `reeve` CLI when a `reeve` on the path reports itself as the planner and the user is logged in, implemented as new planning-initiative and refining-epic skills, one link step in recording-decisions and one context-reading step in brainstorming, and keeps Reeve free of any Lorekeeper dependency, because the planner is the product every client talks to and a CLI is the surface the plugin already knows how to drive.

- **R1** Skills MUST detect the planner by `reeve --version` reporting `reeve-planner` and by a successful login; absent either, every skill MUST behave as before; no manifest change MAY be required.
- **R2** The runner's CLI MUST be renamed so it never answers to `reeve`.
- **R3** Every `reeve` command used by a skill MUST support `--json` and MUST exit with a distinct status for success, refusal and failure; create commands used by skills MUST be idempotent on the key or title given.
- **R4** Initiative and epic planning MUST be separate skills that produce no spec; the brainstorming skill's hard gate and checklist MUST NOT be weakened; recording-decisions MUST link a drafted record to its required decision when the planner is present.
- **R5** Reeve MUST NOT require Lorekeeper for any planning function; an MCP server, if ever added, MUST wrap the same commands.

## Consequences
- Two new skills and two new steps are more test surface for Lorekeeper, and nothing changes in witan-household.
- Skills run under an automation token, so every write is attributed to a person via automation and cannot override the readiness gate.

## Assumptions and invalidation triggers
- *Assumes a shell CLI is an adequate agent surface.* Trigger: a skill needs streaming or long-lived state the CLI cannot provide ⇒ add MCP under R5.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0004, kb/ADR-0002.
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

# ADR-0001: The planner is a Rust server on axum and sqlx serving a statically built SvelteKit client from the same binary

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
related: [reeve/ADR-0001]
implements: []
rfc:
---

# ADR-0002: PostgreSQL is the planner's datastore, accessed through sqlx

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

**Title:** ADR-0003: Users sign in with Google, gated by a configured allowlist, with admins named in configuration, two token kinds and no password store

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0003
title: "ADR-0003: Users sign in with Google, gated by a configured allowlist, with admins named in configuration, two token kinds and no password store"
description: "The planner authenticates humans through Google OpenID Connect only, refuses emails outside configured domains and addresses, treats configured emails as admins and everyone else as members, issues interactive and automation tokens with different powers, and stores no passwords, because everyone on the team has a Google identity, an open sign-in would expose the roadmap, and automation must not pass for a person."
tags: [adr, reeve, identity, auth, oidc, tokens]
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

# ADR-0003: Users sign in with Google, gated by a configured allowlist, with admins named in configuration, two token kinds and no password store

## Status
Proposed 2026-10-25.

## Context
The team is about six people, including non-coders without GitHub accounts; everyone has a Google identity. Mothra built seat and role management and an Entra sign-in that was blocked on admin consent, and that work displaced the product's core. The CLI needs a credential that is not a browser session, Lorekeeper skills act for a person but must not be able to use the human-only readiness override, and a token that outlives its owner is a hole. Earlier drafts added a GitHub OAuth adapter, approval queues, a viewer role and team-derived roles; review found all of it to be account management for six people.

## Facts relied on
- F1 Not every member of the team is on GitHub; everyone has a Google identity — source: team lead's statement in the design session; `/home/daniel/Source/grantigo/household.json` core-non-coders team.
- F2 Mothra's deployment was blocked on Entra admin consent and its auth hook fell back to the first user — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.

## Considered options
- **GitHub OAuth, roles from GitHub teams** — free roles for developers; excludes members without GitHub and adds a sync pipeline.
- **Local accounts with passwords** — works for everyone; hashing, reset flows and invitations.
- **Google OpenID Connect only, allowlist and admins in configuration, members otherwise, two token kinds** — covers everyone with no account management beyond a config file.

## Decision
Daniel Thyselius authenticates humans through Google OpenID Connect only, refuses any verified email outside the configured domains and addresses, treats configured emails as admins and everyone else as members, issues interactive and automation tokens with different powers, and stores no passwords, because that covers every team member with no account management beyond configuration and no open door.

- **R1** The planner MUST NOT store password hashes; sign-in MUST go through Google OpenID Connect; a verified email outside the configured allowlist MUST be refused.
- **R2** Roles MUST be admin or member, determined by configuration; an admin MAY disable a user, which MUST revoke that user's tokens; every authenticated request MUST check the current role and disabled flag.
- **R3** Tokens MUST be interactive or automation; automation tokens MUST NOT override readiness or issue tokens, and their writes MUST be logged as the owner via automation; the CLI MUST take an automation token from an environment variable and an interactive one from login.

## Consequences
- Adding GitHub sign-in later is an adapter; nothing in the model assumes one provider.
- There is no viewer role; a member can do anything a member can, and the activity log shows who did it.
- Changing who is an admin is a configuration change and a restart, which is acceptable at this size.

## Assumptions and invalidation triggers
- *Assumes every team member has a Google identity.* Trigger: a member without one ⇒ add a provider, never a password.
- *Assumes configuration-managed roles stay cheap.* Trigger: the instance serves more people than one config file can keep current ⇒ supersede with an admin page.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0004.
```

### reeve/ADR-0004

**Title:** ADR-0004: The planner is self-hosted, one team per instance, with no organisation or household column

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0004
title: "ADR-0004: The planner is self-hosted, one team per instance, with no organisation or household column"
description: "Reeve is deployed as one self-hosted instance per team, with the key prefix, repositories and sign-in rules in configuration and no organisation or household column in the schema, because a hosted multi-organisation service and its tenant scoping are effort the product does not need and a recurring source of bugs."
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

# ADR-0004: The planner is self-hosted, one team per instance, with no organisation or household column

## Status
Proposed 2026-10-25.

## Context
The planner replaces Jira for one company and is open source as the successor to Reeve. Mothra modelled Spaces as a tenant boundary and enforced tenant scoping by convention in every query, which produced bugs and consumed effort while the roadmap never shipped. The team's services run as Docker containers on hosts in Stockholm so that data stays in Sweden. An earlier draft kept a household as a grouping inside an instance; review found that one team has one household and that the concept was a second tenant boundary in disguise.

## Facts relied on
- F1 Mothra's tenant scoping was by convention and noted as a bug source — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.
- F2 The team's infrastructure provisions Docker hosts in the se-sto1 zone so data stays in Sweden — source: `/home/daniel/Source/grantigo/infra/README.md`.

## Considered options
- **Hosted multi-organisation service** — one deployment serves everyone; an organisation boundary in every table and query.
- **Self-hosted, several households per instance** — one deployment for a person with several teams; a grouping to scope every view and key.
- **Self-hosted, one team per instance, configuration for prefix and repositories** — no scoping anywhere.

## Decision
Daniel Thyselius ships the planner only as a self-hosted instance serving one team, with the key prefix, repositories, horizon buckets, allowlist and admins in configuration and no organisation or household concept in the schema, because the product is a planning tool for a team that runs it and any grouping above the initiative would be cost without a customer.

- **R1** No table MAY carry an organisation, tenant or household column; an instance MUST be assumed to belong to one team.
- **R2** The key prefix, the repositories and their ADR directories, the horizon bucket names, the sign-in allowlist and the admin emails MUST be instance configuration.
- **R3** The reference deployment MUST be one server container and one PostgreSQL container behind the team's existing reverse proxy, provisioned by the existing host scripts.

## Consequences
- Anyone else wanting Reeve runs their own instance; a second team at the same company runs a second instance.
- Access control is per instance and role; there is nothing to scope.

## Assumptions and invalidation triggers
- *Assumes no hosted offering is wanted.* Trigger: a decision to offer Reeve as a service ⇒ supersede; the schema change is large and that is intended.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0002, reeve/ADR-0003.
```

### reeve/ADR-0005

**Title:** ADR-0005: The planner has three bounded contexts, planning, identity and activity, each its own crate, with consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0005
title: "ADR-0005: The planner has three bounded contexts, planning, identity and activity, each its own crate, with consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time"
description: "The planner carries Reeve's architecture rules forward as a recorded decision with exactly three bounded contexts, planning, identity and activity, one crate each: ports defined by the consuming crate, adapters private behind factories, a CI isolation lint, no foreign keys across contexts and the handler owning the clock, because these rules exist so that parallel agents can work the codebase without colliding and more contexts than that were found to be ceremony."
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

# ADR-0005: The planner has three bounded contexts, planning, identity and activity, each its own crate, with consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time

## Status
Proposed 2026-10-25.

## Context
Reeve's domain-driven refactor was motivated by a concrete failure: a single large handlers file meant two agents working unrelated contexts collided on the same file. The resulting rules are documented as standards prose in the knowledge base but never recorded as a decision, so a successor could drift from them without anyone noticing. An earlier draft of the planner proposed seven crates, with separate decision and discussion contexts; review found that required decisions and comments are planning objects and that separate contexts for them added boundaries to police without a second team or a second deployable to justify them. Mothra went the other way, with heavy event modules and projections for a small tool, and paid for it in drift bugs.

## Facts relied on
- F1 The refactor's stated motivation was enabling parallel agent work on unrelated contexts — source: `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md` §Context.
- F2 The five rules and their enforcement by a grep lint are documented — source: `lore/knowledge/general/architecture.md`, `scripts/check-context-isolation.sh`.
- F3 Mothra's thirty-three event modules had two consumers and thirteen bypassing call sites — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.

## Considered options
- **One crate, modules per context** — less ceremony; nothing stops a cross-context import.
- **Three contexts, one crate each, consumer-defined ports, private adapters, isolation lint, no cross-context foreign keys, handler-owned time** — the compiler and CI enforce the boundaries that matter and no others.
- **A crate per aggregate, with event sourcing and projections** — full audit history; demonstrated to over-reach at this scale.

## Decision
Daniel Thyselius carries Reeve's architecture rules into the planner with exactly three bounded contexts, planning, identity and activity, because the rules exist to let several agents work the codebase in parallel, and three contexts are the fewest that keep identity and the audit log out of the planning crate.

- **R1** The planning, identity and activity contexts MUST each be one crate; the server MUST be the only composition root; a new context MUST NOT be added without superseding this record.
- **R2** A cross-context port MUST be defined in the consuming crate and implemented in the supplying crate; adapters MUST be crate-private and reachable only through factories returning trait objects.
- **R3** Tables MUST be owned by one context; foreign keys MUST NOT cross contexts; references MUST be by id or key; initiatives, epics, stories, required decisions and comments MUST all live in the planning context, and the key sequence MUST be owned there.
- **R4** Handlers MUST own time by passing a single timestamp into repositories; repositories MUST NOT read the clock.
- **R5** CI MUST run the context-isolation lint; the activity log MUST be an append-only table written through a port, and the planner MUST NOT adopt event sourcing or asynchronous projections.

## Consequences
- The planning crate is the large one; modules inside it are organised by aggregate so that agents still work separate files.
- Rollups such as initiative progress are computed on read, which costs a query but cannot drift.
- Within the planning context, foreign keys between its tables are permitted; the rule concerns context boundaries.

## Assumptions and invalidation triggers
- *Assumes three contexts stay enough.* Trigger: a second deployable or a second team needs a boundary inside planning ⇒ supersede with the split named.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- reeve/ADR-0001, reeve/ADR-0002.
```

