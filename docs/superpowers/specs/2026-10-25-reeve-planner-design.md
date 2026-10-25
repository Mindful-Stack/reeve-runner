# Reeve planner design

**Status:** Design (2026-10-25, revised twice the same day after seven reviews)

## Summary

Reeve becomes an agent-first planning tool that replaces Jira and Jira Product Discovery for the Grantigo team, built as the open-source successor to the current Reeve in the Witan family. It models work as initiative → epic → story, makes decision-taking a first-class phase by tracking the decisions an item still requires and checking their records in git, computes readiness from facts instead of a hand-set label, and exposes a CLI that Lorekeeper's skills drive so that planning is work an agent does, not work an agent skips. Agent execution loops are a later milestone; the planner ships first, with the contract that loops will need pinned at the interface level only.

The planner lives in a fresh `reeve` repository. The existing repository is renamed `reeve-runner`, keeps running unchanged as the local execution tool, and becomes one pluggable executor later. The two are coupled only by the planner's HTTP API and CLI.

This document is the parent design. Each milestone gets its own spec and plan. The research behind it is in `docs/superpowers/research/2026-10-25-agent-first-planning-research.md`.

## Context

The team plans in Jira Product Discovery, whose roadmap view holds about thirty initiatives in Now, Next, Upcoming, Later and Won't do buckets, and delivers in a Scrum project with epics and stories across the five columns To Do, In Progress, Ready for review, In testing, Done. The initiative template already has a section for decisions required before building. It stays empty, because nothing checks it, and decisions end up being taken by whichever developer writes the Lorekeeper spec at implementation time. The team describes its disease as under-planning: discovery and refinement do not happen, developers work isolated areas toward individual sprint goals, and the bus factor is high.

The current Reeve is a single-user local daemon that orchestrates parallel Claude Code sessions, one card per devcontainer, with no planning hierarchy, no decisions, no users, and no scheduler. Its design deferred the Jira part explicitly. Mothra, an earlier goal-centric Jira replacement in Elixir, never shipped a roadmap or a prioritisation model while spending its effort on seats, roles and theming; its reusable lessons are separate refinement and delivery state, readiness enforced by rules, and blockers as records.

Research across agent-first trackers, orchestration loops, decision processes and open-source candidates found: the industry shape is a tracker as state machine plus a separate supervisor that claims work; readiness is computed, never set; records belong in the repo with the tracker pointing at them; no open-source project ships a three-level hierarchy, a roadmap and an agent API under a permissive licence.

## Goals

- Replace Jira and Jira Product Discovery for the team: strategic planning view, delivery board, hierarchy, comments, PR linking, and migration of existing data.
- Make decisions a first-class phase: every initiative and epic lists the decisions it requires, with a decider and a due date, and readiness follows from whether their records are accepted.
- Make planning something agents do: Lorekeeper skills create epics, stories and required decisions through the CLI, and read packaged context from it. The milestone-one acceptance test for this goal is that triage proposes placements for untriaged stories and the epic-refinement skill drafts stories with criteria.
- Compute readiness so that a ready story is pickable by anyone, human or agent, which is the structural fix for the bus factor.
- Design the planning model so that execution loops are additive later.

## Non-goals for this design

- Building the execution loop, the scheduler or the oversight views. Milestone two.
- Goals and metrics as entities. Later, if ever.
- A wiki, document store or catalogue of every decision record. Records and behaviour docs stay in git; broad product reasoning lives in the initiative brief; the planner shows the decisions items require and the bodies of accepted records.
- Notifications by email or chat. A later epic of its own.
- Sprints, a timeline view, dashboards and reports, saved filters, Jira Product Discovery scoring and voting, attachments beyond a link, a refinement view. All deliberately absent from milestone one and additive.
- A hosted multi-organisation service.

## Milestones

**Milestone one: migrate off Jira.** Planning server, SvelteKit client, CLI and TUI, Jira import, record check, comments on stories and required decisions, PR linking through a GitHub webhook, manual backlog rank, and the Lorekeeper integration with the initiative-planning and epic-refinement skills. Humans and automation acting for a human are the only actors; no agent principals yet. Build order within the milestone: planning server and API, strategic planning view and delivery board, import, required decisions and the decisions view, Lorekeeper skills, TUI last.

Today's Reeve keeps running on the maintainer's machine with a thin bridge: a card is created from a story key, and moving the card calls `reeve story mv` through the planner CLI with an automation token, failing visibly when the gate refuses rather than overriding.

**Milestone two: loops.** Agent principals, the executor contract, sessions and activities, oversight views, budgets, possibly as human-triggered delegation first and autonomous pickup second. The executor behind the contract is chosen then: the Reeve runner, Claude Code background agents, or a hosted sandbox.

This document fully specifies milestone one and specifies milestone two at the interface level only.

## Repositories, binaries and naming

- `reeve` (new): the planner. Server, web client, CLI and TUI, OpenAPI document. Seeds carried over from the current repo: the card crate as the start of the planning crate, the protocol crate's type discipline, the architecture rules, the lore knowledge base, and the CLI and TUI crate's event, action and render structure. Binaries: `reeve-server` and `reeve` (CLI and TUI).
- `reeve-runner` (the current repository, renamed): frozen in maintenance mode with the bridge above, gutted into a real executor at milestone two or retired. Its CLI binary is renamed from `reeve` to `reeve-runner` so the two tools can coexist on one machine; its daemon stays `reeved`.
- Lorekeeper detects the planner by running `reeve --version`, which prints `reeve-planner <semver>`; a binary that prints anything else is not the planner.
- Both repositories are members of one Witan household. The existing `reeve-household` directory becomes that household once its manifest is migrated to the current schema; its knowledge base holds the shared ADR home.
- The runner never links a planner crate. Coupling is the public HTTP API and the CLI only, and every CLI read has an HTTP endpoint.

## Domain model

Three bounded contexts, planning, identity and activity, each its own crate, plus the protocol crate, the server and the CLI. Within a context, foreign keys are fine; across contexts, references are by id or key only.

### Planning context

**Household.** Name, key prefix, key sequence, manifest reference (repos and their ADR directories, knowledge base). The grouping for everything below. The server does not clone repositories.

**Initiative.** Key, title, household, brief with sections vision, capabilities (must, nice, out of scope) and known knowns as Markdown, horizon bucket (Now, Next, Upcoming, Later, Won't do), target month, owner, discovery-complete mark (timestamp and person, or none), labels. Progress is computed on read: stories done with resolution completed over stories not resolved won't do or duplicate; no value when the denominator is zero.

**Epic.** Key, title, initiative (nullable for migration, flagged on the epic page and the decisions view), brief with goal, scope and out of scope, discovery-complete mark, labels.

**Story.** Key, title, kind (story, bug, chore), optional epic, description, acceptance criteria as an ordered list of text items each with a done flag, optional estimate, owner (a user), optional delegate (a principal: a user in milestone one, an agent principal from milestone two), labels, rank within todo, typed dependency edges (blocks, relates) to other stories in the same household, linked PRs (repository, number, url, status draft, open, merged or closed), delivery state, resolution when done (completed, won't do, duplicate), external references (for example the Jira key), row version. Readiness is never stored. A story of kind story with no epic is untriaged.

**Required decision.** Attached to an initiative, epic or story: title, decider (a user), due date, optional record reference (repository, path, record id), optional PR url for reading a proposal, last check result (accepted, not accepted with the record's status, not found, or unchecked) with the commit it was read at, closed-at and reason when a human closes it. Discovery creates required decisions; a junior creates one to ask for a decision.

**Comment.** On a story or a required decision: author (a principal), body as Markdown, timestamp, optional external source id for imported comments. Plain and unthreaded. Comments never change state.

**Readiness computation.** For a story: its epic is decided if it has one; a story of kind story has an epic; every open required decision attached to the story is satisfied; at least one acceptance criterion; no open blocking story, where open means delivery not done. A required decision is satisfied when its last check found the record accepted on the repository's default branch; otherwise it yields a reason: no record yet, record not found, or the record's status, with "superseded by X, re-point" when the status is superseded. A closed required decision is ignored. For an epic, decided means discovery complete, its initiative decided if it has one, and every open required decision satisfied; refined means decided plus at least one story and criteria on every story. For an initiative, decided means discovery complete and every open required decision satisfied. Attaching a new required decision to a decided item makes it undecided. The computation returns the reasons when it fails.

**Dependency rules.** Blocks edges are rejected at write time when they would form a cycle; the check runs in a transaction holding the household row. Edges may cross epics and initiatives within a household and may not cross households.

### Identity context

**Principal.** Id and kind (user or agent). **User.** Verified email as identity, display name, linked provider identities (GitHub, Google), role (none, admin, member, viewer), status (pending approval, active, disabled). **Token.** Owner principal, kind (interactive or automation), expiry, revoked-at. **Agent principal** (milestone two): name, owning executor, token. **Allowlist.** Email domains and explicit addresses permitted to sign in.

### Activity context

One append-only table: actor principal, via (interactive or automation), entity (type and key), action, payload, timestamp. Feeds the activity feed, the admin page's audit list, and the event stream. The planning and identity contexts write to it through a port; nothing reads another context's rows from it.

### Session (milestone two, planning context)

One per story and executor attempt, a sub-aggregate of the story: state (pending, active, awaiting input, complete, error, stale), executor, epoch (monotonic per story, issued at claim), lease expiry refreshed by heartbeat, stop-requested flag, started and last-activity timestamps, reported cost and duration, typed activities (thought, action, elicitation, response, error) with timestamps and text. A user answers an elicitation by posting a response activity.

## Lifecycles and gates

**Discovery** on initiatives and epics is a mark, not a machine: the owner declares discovery complete, which stores the timestamp and person; clearing the mark reopens discovery and is logged. Discovery is where humans and agents research and create required decisions, each with a decider and a due date; a record is linked later. Overdue open required decisions surface on the strategic planning view and the decisions view.

**Delivery** on stories is free movement between todo, in progress, in review, testing and done, with two rules. Leaving todo for in progress, in review or testing requires ready, or an interactive override with a reason chosen from a short pick-list plus free text, written to the activity log; automation tokens and agent principals cannot override, and an agent principal may not be delegated a story that is not ready. Entering done requires a resolution: completed is allowed only from testing, won't do and duplicate from any state including todo, and those two are the only ungated ways out of todo. The lead owns the weekly override count, which is a first-class query so the gate can be tightened on evidence.

**Session** (milestone two): pending, active, awaiting input, complete, error, stale. Stale means the lease expired without heartbeat; a stale session is not live and frees the claim. Complete atomically moves delivery to in review. A live session carries the current epoch; writes with an older epoch are refused. The stop flag belongs to the session: it is set when delivery leaves in progress or a user asks, and a new claim starts a new session with it clear. A user's "stop and undelegate" also clears the delegate so nothing reclaims automatically.

**Required decision lifecycle.** Open without a record, open with a record reference, satisfied once the check finds the record accepted, or closed by a human with a reason. The record's own lifecycle, proposed to accepted or rejected and later superseded, lives in the file and in Lorekeeper; the planner only reads it.

**Intake.** A story of kind story without an epic is untriaged and appears in the triage inbox. Triage applies the placement tree: broken behaviour becomes a bug under the owning epic or none; a product change finds the covering epic, else the covering initiative and a new epic, else a new initiative in Later; internal work becomes a chore. Triage is a sequence of idempotent CLI commands (create initiative, create epic, set the story's epic) so a human can confirm each. In milestone one the triage skill proposes and a human confirms.

**PR linking.** A GitHub webhook receives pull-request events; the branch name carries the story key (`<key>-<attempt>`, attempt 1 for humans). The PR is linked to the story with its status. PR status is information, not control: the only automatic move is that a PR opening on a story in progress moves it to in review. Linking never starts a story, and a PR on a todo story is linked and flagged on the drawer. Merging moves nothing, because the planner cannot know the complete set of PRs a story needs; a human, or in milestone two the executor's complete transition, moves the story on. A linked PR is never deleted.

## Topology

**Server.** One Rust binary on axum and sqlx. Crates: `reeve-protocol` (API types; the OpenAPI document is generated from them), `reeve-planning`, `reeve-identity`, `reeve-activity`, `reeve-server` (composition root, serves the API and the static client, receives the GitHub webhook), `reeve` (CLI and TUI). Live updates are one server-sent events stream of activity entries, filtered to the principal's scope, with resume by last event id; in milestone two a per-session poll endpoint is the authoritative stop channel for executors.

**Client.** SvelteKit built with the static adapter, served by the server binary. Types generated from the OpenAPI document; a drift fails the build on either side.

**Hosting.** One server container plus one PostgreSQL container on the existing UpCloud Docker-host pattern in Stockholm, behind the team's existing nginx reverse proxy with TLS, backups and deploys through the existing scripts and digest pipeline. A documented restore procedure, rehearsed once before cut-over, with a second named person able to run it.

**Tenancy.** Self-hosted, single-tenant. No organisation column. Household is the only grouping; one instance can hold several households.

**Identity.** Google through OpenID Connect and GitHub through an OAuth adapter that fetches verified emails from GitHub's email endpoint; both resolve to a user keyed by verified email. Sign-in is refused unless the email matches the allowlist. A new user has role none and status pending approval until an admin approves and assigns a role; roles are admin, member, viewer and are set only by admins. Every authenticated request checks the owner's current role and status, so a demotion or a disabling takes effect on the next request; disabling a user also revokes their tokens. Tokens are interactive or automation; automation tokens cannot override, administer or mint tokens, and their writes are logged as the owner via automation. The first admin is created at deployment with `reeve-server bootstrap-admin <email>`, which allowlists the email with role admin so the first sign-in is active. A viewer sees every household in the instance; per-item access control is out of scope.

**Keys.** One sequence per household with a short prefix, for example `GRA-412` for any level; the kind is shown beside the key. Imported Jira keys are external references that the API and CLI resolve. Artifact paths use the key; branch names use `<key>-<attempt>`.

**CLI.** `reeve` with subcommand groups `initiative`, `epic`, `story`, `decision`, `comment`, `household`, `auth`, `import`, and `tui`. Every command, including `story context`, supports `--json`. Mutations are idempotent. Exit codes map to the typed response codes. `reeve --version` prints `reeve-planner <semver>`.

## Lorekeeper integration and CLI contract

**Direction and detection.** Lorekeeper calls Reeve, never the reverse. The household manifest gains an optional `reeve` section with the instance URL, validated by Lorekeeper's doctor and migrate scripts and added to the witan-household template. Skills check for the section and for a `reeve` on the path whose `--version` reports the planner; absent either, they behave as today. Reeve needs nothing from Lorekeeper to run.

**Skills.** The brainstorming skill is not given modes; its hard gate and spec checklist stay as they are, and it gains one first step that reads `reeve story context` when the planner is present. Two new skills reuse its exploration and architect-dispatch steps:

- `planning-initiative`: `reeve initiative show <key> --json`, discovery with the architect agent's survey plus research, output is updated known knowns and open questions (`reeve initiative edit <key> --json`), required decisions (`reeve decision require --on <key> --title ... --decider <email> --due <date> --json`), proposed epics (`reeve epic create --initiative <key> --json`), and for each required decision a drafted record through recording-decisions. No spec.
- `refining-epic`: epic-level discovery the same way, then refinement: stories with acceptance criteria (`reeve story create --epic <key> --title ... --criterion "..." --criterion "..." --json`). No spec.

Lorekeeper's record triage is changed so that process, tooling and product decisions are drafted as records rather than declined as "no code behind it", with an optional `kind` front-matter field.

**Linking a record.** When recording-decisions drafts a record for a required decision, it runs `reeve decision link <id> --record <record-id> --repo <name> --path <path> --pr <url>` from the household checkout, where the path is known. The planner stores the reference, resets the check result to unchecked, and runs the check for that one required decision immediately; re-pointing an existing reference does the same, so a previous acceptance never carries over to a new record. No item keys are written into records, so accepted records stay locked.

**Record check.** `reeve decision check` is one scheduled job, also runnable by hand, that for every open required decision with a reference reads the file at its path on the repository's default branch, parses the front-matter status and `superseded_by`, tolerating legacy records without front matter by reading their status line, and stores the result with the commit. For accepted records it also caches the body for the decisions view. It never walks ADR homes and does not use Lorekeeper. A file that cannot be read yields not found.

**Context packaging.** `reeve story context <key>` emits one bundle, as Markdown or with `--json`: the initiative's vision, the epic brief, every required decision with its check result and path, the criteria, the dependencies, and the comments.

**Readiness as a query.** `reeve story ready <key> --json` returns the boolean and reasons. `reeve story mv <key> in-progress` applies the gate, prints reasons on refusal, and accepts `--override <code> --reason "<text>"` for interactive tokens only. `reeve story ls --ready --json` lists pickable work.

**Triage.** The triage skill runs the placement tree against `reeve story ls --untriaged --json` and proposes; a human confirms with `reeve story edit <key> --epic <key>` or in the web client.

**Ratification.** The planner never accepts, rejects or edits a record. Discussion on a required decision is its comment thread; a human hands a summary to an interactive Lorekeeper session, which drafts or ratifies through its own flow, and the next check picks up the result. There is no accept action in the planner.

**Contract rules.** `--json` on every command; idempotent mutations; typed exit codes; every CLI read has an HTTP endpoint; no capability in a future MCP that the CLI lacks.

## Surfaces

**Web.** In priority order: the strategic planning view (initiatives in five horizon buckets with target month, discovery badge, overdue required-decision count and computed progress; drag between buckets); the delivery board (stories by delivery state with filters for household, epic, owner, kind, labels, and a ready indicator; drag to rank within todo); the decisions view (every open required decision with decider, due date, the item that requires it and its check result, overdue first, with the accepted record's body rendered read-only beneath satisfied ones, and comments); initiative and epic pages (brief with section editing, required decisions, children, activity); the story drawer, addressable by URL from any view (fields, criteria checklist, required decisions, dependencies, PRs, comments, activity, override prompt); the triage inbox; the admin page (users, approvals and roles, allowlist, households, tokens, audit list).

**TUI.** A thin board client, built last in milestone one: delivery board, story drawer with readiness reasons, move, triage, live over the event stream. Embedded sessions return with the runner in milestone two.

## Jira migration

`reeve import jira --site <url> --token-from-env JIRA_TOKEN --household <name>` reads both projects through the Jira Cloud API. The importer only creates; it never updates an existing item. A rehearsal import runs on the real instance and is shown to the team; the database is reset before the cut-over import, which runs once.

- Product Discovery ideas → initiatives. Roadmap bucket → horizon; target date → target month; description split by its template headings into vision, capabilities, known knowns, open questions; "ADRs required" text kept inside open questions; delivery links → initiative-to-epic relation.
- Delivery epics → epics, parented from delivery links, orphaned and flagged otherwise.
- Stories, bugs, tasks → stories with kind story, bug, chore. Status mapped column for column; Jira resolutions Won't Do and Duplicate → done with that resolution. Assignee → owner by email. Labels preserved. Jira key → external reference. Rich text → Markdown. Comments → comments with original author and date, keyed by Jira comment id.
- Grandfathering: stories in progress, in review or testing at import, and every epic and initiative with such stories, are marked discovery complete by the importer with a legacy label and excluded from override counts.
- Cut-over is a day: Jira is set read-only, the import runs, and Jira stays readable for reference. Rollback within the first two weeks is re-enabling Jira writes and abandoning the instance.
- Not imported: Confluence, sprints, attachments beyond a link.

After import, initiatives and epics without in-flight work have no discovery mark and nothing is linked, so most of the backlog is not ready. That is the true baseline; the override with reason keeps work moving while discovery catches up.

## Loops interface (milestone two, interface level only)

- Agent principal: a principal of kind agent with a token owned by an executor; may be a story's delegate; the owner stays a user. A story is delegated to one agent principal or none.
- Claim: `POST /stories/{key}/claim` by an executor succeeds only if the story is ready, delegated to that agent, in todo or in progress with no live session, and atomically creates a pending session with a new epoch, a lease and a clear stop flag, and moves delivery to in progress under the row version.
- Session and activities: the executor heartbeats the lease and posts state transitions and typed activities carrying the epoch; writes with a superseded epoch are refused with conflict; complete atomically moves delivery to in review; a user may post a response activity to answer an elicitation.
- Stop: the planner sets the session's stop flag when delivery leaves in progress or a user asks; the executor reads it from the per-session poll endpoint, which is authoritative, and from the filtered event stream as an optimisation; "stop and undelegate" also clears the delegate.
- PRs: the executor registers PRs on the story; the GitHub webhook updates their status.
- Brakes in the planner, runtime-neutral: a per-principal cap on live sessions, the lease deadline, and token revocation. Fine-grained budgets and scheduling live in the executor, which reports cost and duration on the session.
- Milestone-one obligations that make this additive: principals with kinds, the delegate as a principal, the activity log, row versions, branch names with attempts, and no stored readiness.

## Error handling

- One response envelope with typed codes: ok, not found, not ready (a refusal carrying reasons), conflict (carrying the current version or epoch), forbidden, validation, unauthenticated, pending approval. The CLI maps codes to exit statuses; not ready is a refusal, not a failure, and exits with its own status.
- Writes carry a row version; a stale write returns conflict.
- Import and the record check collect per-item failures, finish what they can, and report at the end.
- The event stream resumes from the last event id on reconnect.

## Testing

1. Domain tests, pure and fast: readiness computation including every check result, progress with every resolution, cycle rejection, the delivery rules, placement rules, front-matter and legacy status parsing against copies of real records from the Grantigo knowledge base and the file-data-extraction-service.
2. Repository tests against a real PostgreSQL started per test run, one set per context.
3. API tests through the axum router with a test database, including the GitHub webhook, the todo-exit gate on every target state, per-request role checks and token-kind enforcement; an OpenAPI snapshot so a wire change is a deliberate diff; generated client types checked in CI.
4. CLI end-to-end against a running server with a recorded Jira export as the import fixture and a fixture household with ADR files as the check fixture.
5. Web: a Playwright smoke across the strategic planning view, the decisions view and the board.

## Inherited choices and their verdicts

| Existing Reeve choice | Verdict |
|---|---|
| Bounded context per crate, consumer-defined ports, private adapters, isolation lint | Keep, with three contexts (reeve/ADR-0005) |
| SQLite through rusqlite, async-over-sync | Drop; PostgreSQL through sqlx (reeve/ADR-0002) |
| No cross-context foreign keys, handler owns the clock | Keep (reeve/ADR-0005) |
| Unix-socket JSON lines protocol | Drop; HTTP API with generated OpenAPI (reeve/ADR-0001) |
| Per-card socket as agent identity | Keep in the runner |
| Column transition runs the workspace lifecycle synchronously | Change; server never provisions, executors report sessions (kb/ADR-0008) |
| Single column enum mirroring the agent's phase | Change; discovery mark, delivery state with two rules, session (kb/ADR-0004) |
| Per-card lock across side effects | Change; row version and session epoch |
| One story, one workspace, one devcontainer | Keep in the runner |
| Reference clones, mirrored refs, no credentials in the container, host-side GitHub bridge | Keep (kb/ADR-0008 R5) |
| Push and PR creation human-only | Change; executor-bridge-mediated under the planner's brakes |
| Household manifest as the project unit | Keep |
| Hard Lorekeeper dependency, Lorekeeper unaware of Reeve | Invert (kb/ADR-0006) |
| Plugin-agnostic session prompt; artifacts named by UUID | Keep the first; artifacts by key (kb/ADR-0007) |
| Brief of 1–200 characters as the whole description | Drop; brief remains the name of an initiative's or epic's document |
| `needs_heavy`, `parent_id` branch lineage | Drop |
| No event log, TUI polls | Change; activity log and server-sent events |
| Ratatui TUI with embedded sessions | Keep as an HTTP client |
| Vocabulary card, column | Change; initiative, epic, story, decision, state (kb/ADR-0002) |
| CLI binary named `reeve` | Moves to the planner; the runner's CLI becomes `reeve-runner` |

## Open questions deferred past this design

- Executor choice for milestone two: Reeve runner, Claude Code background agents, or a hosted sandbox.
- Whether the strict gate (whole epic refined before any story starts) replaces the loose one; decided on override counts after a quarter.
- Notifications as their own epic; whether a refinement view or sprints return; reports.
- Whether an MCP server wrapping the CLI is wanted.

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

# ADR-0002: Work is modelled as initiative, epic and story, where a story is one pull request and an epic holds decisions and the breakdown

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
Daniel Thyselius models work as initiative, epic and story, with a story being what ships as one pull request per affected repository and an epic owning the decisions and the story breakdown, because that forces product planning to the epic while leaving implementation design just-in-time, and it is the only option that changes neither Reeve's nor Lorekeeper's unit of work.

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

**Title:** ADR-0003: Readiness is computed from facts, never stored, and gates every exit from todo except cancellation, with an interactive-only override

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0003
title: "ADR-0003: Readiness is computed from facts, never stored, and gates every exit from todo except cancellation, with an interactive-only override"
description: "A story's readiness is computed on read from its epic's and initiative's required decisions, its own required decisions, its acceptance criteria and its open blockers, never stored, and it gates every exit from todo except cancellation with a reasoned override for interactive users only, because a stored flag rots and an unmeasured gate cannot be tightened honestly."
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

# ADR-0003: Readiness is computed from facts, never stored, and gates every exit from todo except cancellation, with an interactive-only override

## Status
Proposed 2026-10-25.

## Context
The team's initiative template has a section for decisions required before building, and it stays empty, because nothing checks it. Published practice computes readiness from facts: Beads' ready query is "open with no open blockers", Mothra's refinement state machine required subtasks and no blockers, Shape Up's "solved" and Kubernetes' "implementable" encode decided-ness as a state. Nobody found phrases the gate as "every required decision accepted", which is the gap this record fills. Definition-of-ready practices fail when they harden into a barrier the team routes around, and the team has stated that under-planning is its disease, so a gate that is bypassed silently teaches nothing. Automation acting under a person's token must not be able to bypass the gate, or the bypass count measures nothing. A decision whose record is superseded or rejected must not make work unready forever.

## Facts relied on
- F1 Readiness in Beads is a query, not a field — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §4.
- F2 Reeve's `needs_heavy` flag became orphaned metadata because its enforcement point was never built — source: `docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md` §Future work.
- F3 Atlassian measured agent-authored-for tickets as agent-ready 83% of the time versus 6% for traditional ones — source: research digest §6.
- F4 Accepted records can be superseded, and the successor is named in `superseded_by` — source: `/home/daniel/Source/grantigo/knowledge-base/knowledge/adrs/0022-portal-reads.md` front matter.

## Considered options
- **Stored readiness label set by humans** — zero computation and familiar from Jira labels.
- **Computed readiness, strict gate: the whole epic refined before any story starts** — maximises upfront planning.
- **Computed readiness, loose gate on this story's facts, interactive override with reason, no automation or agent override** — enforces the gate where it is cheap and measures bypasses.

## Decision
Daniel Thyselius computes a story's readiness on every read as: its epic is decided if it has one, a story of kind story has an epic, every open required decision attached to the story is satisfied, it has acceptance criteria, and no blocking story is open; the computation returns reasons on failure, gates leaving todo for any state other than a cancelled done, allows an override only from an interactive token with a recorded reason, and allows none for automation or agent principals, because a computed value cannot rot and a recorded override lets the gate be tightened on evidence.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons.
- **R2** An interactive user MAY move a non-ready story out of todo into in progress, in review or testing only by supplying a reason code and text, which MUST be written to the activity log; automation tokens and agent principals MUST NOT override.
- **R3** Epic decided MUST mean discovery marked complete, its initiative decided if it has one, and every open required decision satisfied; epic refined MUST additionally require at least one story and criteria on every story; initiative decided MUST mean discovery marked complete and every open required decision satisfied.
- **R4** A required decision MUST be satisfied only when its last check found the referenced record accepted on the repository's default branch; no record, record not found, and any other status MUST each yield a distinct reason; a closed required decision MUST be ignored.
- **R5** A story of kind story without an epic MUST be not ready with reason untriaged; bugs and chores MAY be ready without an epic. Open MUST mean delivery not done.

## Consequences
- Immediately after Jira import most of the backlog is not ready, which is true, and overrides will be frequent at first; the importer grandfathers in-flight work so day one is not a wall of overrides.
- Readiness queries touch required decisions, dependencies and criteria on each read; indexes on those links are mandatory.
- Tightening to the strict gate later is a rule change plus a policy decision backed by override counts, which the lead owns.

## Assumptions and invalidation triggers
- *Assumes the record check runs often enough that results are current.* Trigger: staleness between acceptance and check is routinely noticed ⇒ add the check age as a readiness reason and shorten the schedule.
- *Assumes the loose gate is enough to change behaviour.* Trigger: override rate stays above half of starts after one quarter ⇒ supersede with the strict gate.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0004, kb/ADR-0005.
```

### kb/ADR-0004

**Title:** ADR-0004: Discovery is a mark, delivery is free movement under two rules, sessions carry their own state and stop flag, and pull requests are linked records rather than a state

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0004
title: "ADR-0004: Discovery is a mark, delivery is free movement under two rules, sessions carry their own state and stop flag, and pull requests are linked records rather than a state"
description: "Reeve keeps discovery as a completion mark on initiatives and epics, delivery as free movement between five states with a readiness rule on leaving todo and a resolution rule on entering done, and sessions as a separate machine with their own stop flag, with PRs as linked records, because one column enum was only honest when the agent was the sole actor and an explicit edge list is a liability for a small team."
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

# ADR-0004: Discovery is a mark, delivery is free movement under two rules, sessions carry their own state and stop flag, and pull requests are linked records rather than a state

## Status
Proposed 2026-10-25.

## Context
Reeve's single column enum, shared by wire, database and CLI, was a deliberate simplification: the column was a projection of the one agent's self-reported phase, and a PR column made shipping an explicit act. The successor has humans, automation, agents and a computed readiness acting on the same story, plus a planning side with its own discovery. Mothra's lesson was that keeping refinement and delivery apart kept the board from lying. Linear's agent sessions carry their own states and typed activities independent of the issue's workflow state. Review of an earlier draft found that an explicit edge list for delivery kept missing cases (release, reopen, cancellation) and that a stop flag on the story rather than the session leaked a previous attempt's stop into the next claim. Jira's Won't Do and Duplicate resolutions must survive import without counting as completed.

## Facts relied on
- F1 Reeve's columns are "a projection of the agent's self-reported state" — source: `lore/knowledge/domain/card.md`.
- F2 Mothra separated a refinement machine from a workflow machine and gated sprint entry on the former — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.
- F3 Linear agent sessions use pending, active, awaitingInput, complete, error and stale — source: research digest §4.
- F4 The Jira delivery board uses To Do, In Progress, Ready for review, In testing, Done — source: https://grantigo-team.atlassian.net/jira/software/projects/SCRUM/boards/1 on 2026-10-25.

## Considered options
- **One enum covering planning, delivery and agent phase** — simplest wire type, as Reeve has today.
- **Explicit delivery edge list plus a PR column** — every transition named and reviewable.
- **Discovery mark, free delivery movement under two rules, separate session machine, PRs as linked records** — the fewest rules that still enforce the gate and the resolution.

## Decision
Daniel Thyselius models discovery as a completion mark, delivery as free movement between five states constrained by a readiness rule and a resolution rule, and sessions as a separate machine carrying their own stop flag, and represents pull requests as linked records on a story, because the gate and the resolution are the only transitions that need a rule and every other edge list was found incomplete.

- **R1** Discovery MUST be a mark holding timestamp and person; clearing it MUST be logged; decided and refined MUST be derived.
- **R2** Delivery MUST be todo, in progress, in review, testing, done, freely movable, with exactly two rules: leaving todo for in progress, in review or testing MUST satisfy readiness or an interactive override, and entering done MUST carry a resolution of completed, won't do or duplicate, where completed MUST only be entered from testing and won't do or duplicate are the only ungated exits from todo; no other delivery state or rule MAY be added without superseding this record.
- **R3** Session state and activities MUST be written only by executors through the executor contract, except a user's response activity answering an elicitation; the planning context MUST NOT derive delivery from session internals.
- **R4** The stop flag MUST belong to the session; delivery leaving in progress or a user request MUST set it; a new claim MUST start a session with it clear; a session's complete transition MUST atomically move delivery to in review.
- **R5** A PR MUST be a linked record with status draft, open, merged or closed; "shipped" MUST NOT be a delivery state; a linked PR MUST NOT be deleted; linking MUST NOT move a story out of todo; the only automatic delivery move from PR events MUST be in progress to in review on a PR opening, and merging MUST move nothing.

## Consequences
- A story can be dragged backwards or forwards without ceremony, as in Jira; the two rules are the whole policy and are easy to teach.
- The explicit "ship it" act moves from a board column into the executor contract and the GitHub webhook, where loops need it anyway.
- The TUI's existing column rendering is rewritten around delivery states only.
- Sessions live inside the planning context as a sub-aggregate of the story so that a claim is one transaction; R3 constrains who writes them, not where they are stored.

## Assumptions and invalidation triggers
- *Assumes five delivery states and three resolutions cover the team's flow.* Trigger: the team needs a sixth state or a fourth resolution for a quarter ⇒ supersede R2.
- *Assumes session semantics match Linear's for executors in general.* Trigger: an executor cannot map its lifecycle onto the six states ⇒ amend the session machine via kb/ADR-0008.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0003, kb/ADR-0008.
```

### kb/ADR-0005

**Title:** ADR-0005: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks acceptance on the default branch

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0005
title: "ADR-0005: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks acceptance on the default branch"
description: "The planner never owns a decision: it tracks required decisions as its own objects with decider and due date, links each to a record by repository, path and id without writing anything into the record, and a scheduled check reads the file on the default branch to learn whether it is accepted, because the versioned file must stay the single truth and accepted records are locked."
tags: [adr, reeve, decisions, lorekeeper, rfc]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve/src/commands/decision*", "lorekeeper:skills/recording-decisions/**", "lorekeeper:references/adr-template.md", "lorekeeper:scripts/adr-lint.js"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0006]
implements: []
rfc:
---

# ADR-0005: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks acceptance on the default branch

## Status
Proposed 2026-10-25.

## Context
The team keeps architecture decision records beside the code they govern and shared ones in the knowledge base, with front matter carrying id, status, decided_by, reversibility, scope and relations. Lorekeeper locks an accepted record to five permitted edits, and a proposal's PR merges only once the record is accepted, so proposed records exist only on open branches. Some older repositories hold legacy records without front matter. RFCs were written separately, often with security, UX or product people, and covered both broad product questions and non-architectural hard-to-reverse choices such as tooling. Practitioners keep records in the repo and pitches or decision logs in a wiki layer, with the tracker holding a tracking item that links out; Rust's RFC process is the canonical example. An earlier draft of this design mirrored every record, including proposals on branches, and review found that this produced status-mapping, branch-scanning and snapshot-ordering machinery out of proportion to the problem.

## Facts relied on
- F1 Shared ADRs use front matter with id, status, decided_by, reversibility, blast_radius, scope, supersedes and superseded_by — source: `/home/daniel/Source/grantigo/knowledge-base/knowledge/adrs/0022-portal-reads.md`.
- F2 Accepted records are locked to five edits and a proposal merges only once accepted — source: Lorekeeper `skills/recording-decisions/reference/lifecycle.md` on origin/main.
- F3 The file-data-extraction-service records have no front matter — source: `/home/daniel/Source/grantigo/file-data-extraction-service/docs/architectural-decision-records/`.
- F4 Rust's RFC merge procedure creates the tracking issue first and links it from the RFC — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §6.

## Considered options
- **Decisions authored and stored in the planner** — one place for non-technical readers and comments; two sources of truth.
- **Mirror every record from every home, including proposals on branches** — a complete catalogue in the planner; heavy sync machinery and collisions between canonical and proposed versions.
- **Required decisions as planner objects, each linked to one record by path, checked for acceptance on the default branch** — readiness computed from one lookup, the file untouched, nothing to scan.

## Decision
Daniel Thyselius keeps every decision record in its repository or knowledge-base home, tracks the decisions an item requires as planner-owned objects with a decider and a due date, links each to a record by repository, path and id written only on the planner side, and runs a scheduled check that reads the file on the default branch to learn whether it is accepted, because readiness needs one fact per required decision and the versioned file must remain the only truth and must not be edited to add links.

- **R1** The planner MUST NOT edit, accept or reject a record; the only record data it stores is the check result, the commit it was read at, and a cached body for accepted records, all written only by the check.
- **R2** A required decision MUST be a planner-owned object with title, decider and due date, attached to an initiative, epic or story, optionally referencing one record by repository, path and id and one PR url; closing it MUST be a logged human action with a reason.
- **R3** Setting or changing a reference MUST reset the check result to unchecked and trigger an immediate check of that required decision, so a previous acceptance never carries over; the link MUST be stored in the planner only; no item keys MAY be written into records; `rfc` MUST remain a tolerated deprecated key in Lorekeeper's template.
- **R4** The check MUST read each referenced file on the repository's default branch, MUST parse status and `superseded_by` from front matter and the status line of legacy records, MUST report not found when the file cannot be read, and MUST NOT walk ADR homes or depend on Lorekeeper.
- **R5** Broad product reasoning MUST live in the initiative brief; a hard-to-reverse non-architectural choice MUST be a decision record, and Lorekeeper's record triage MUST draft process, tooling and product records rather than decline them; no separate RFC artifact MAY be introduced.

## Consequences
- Non-technical readers read a proposal through its PR link and read accepted bodies in the planner; discussion happens as comments on the required decision.
- The planner has no catalogue of every record; Lorekeeper's index remains the catalogue.
- A superseded or rejected record shows as a reason on the required decision, and a human re-points or closes it; nothing is unready forever by accident.
- The Lorekeeper changes shrink to the record triage, an optional `kind` field, and one link command at draft time.

## Assumptions and invalidation triggers
- *Assumes a scheduled check is frequent enough.* Trigger: staleness between acceptance and check is routinely noticed ⇒ shorten the schedule and surface check age.
- *Assumes non-technical participation is satisfied by comments and PR links.* Trigger: a demonstrated need to author records outside git ⇒ supersede.

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
description: "Lorekeeper skills call the reeve CLI when the household manifest declares a Reeve instance and a reeve on the path reports itself as the planner, through new planning-initiative and refining-epic skills and one link step in recording-decisions rather than modes of brainstorming, and Reeve neither links nor requires Lorekeeper, because the planner is the product and the plugin is one client of it; this inverts the earlier rule that Lorekeeper must not know Reeve exists."
tags: [adr, reeve, lorekeeper, integration, cli, manifest]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["lorekeeper:skills/**", "lorekeeper:scripts/**", "lorekeeper:commands/doctor.md", "witan-household:**", "reeve:crates/reeve/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0005, kb/ADR-0002]
implements: []
rfc:
---

# ADR-0006: Lorekeeper drives Reeve through the reeve CLI when a household declares an instance; Reeve never depends on Lorekeeper

## Status
Proposed 2026-10-25.

## Context
Reeve's original design hard-required Lorekeeper and stated that Lorekeeper must not know Reeve exists, because Reeve was an execution loop meant to sit beside an existing planning tool. The successor is the planning tool. Initiative-first work means Lorekeeper must create epics, stories and required decisions, link drafted records to them, and read context from the planner. Lorekeeper already drives external tools by shelling out to CLIs such as gh. The brainstorming skill has a hard gate and a checklist that ends in a spec file and a hand-off to writing-plans, which planning at the initiative or epic level must not produce. The current Reeve installs a `reeve` binary with different commands, and it keeps running during milestone one. Nothing in Lorekeeper's code validates unknown manifest sections today.

## Facts relied on
- F1 The 2026-04-29 design deferred Jira intake and stated Lorekeeper stays a pure plugin unaware of Reeve — source: `docs/superpowers/specs/2026-04-29-reeve-design.md` §Deferred.
- F2 The brainstorming skill's hard gate and checklist force a spec and a writing-plans hand-off — source: Lorekeeper `skills/brainstorming/SKILL.md` on origin/main.
- F3 The manifest schema file holds only a current version and the migrate script only stamps it — source: Lorekeeper `scripts/manifest-schema.json`, `scripts/migrate-manifest.js` on origin/main.
- F4 The current Reeve workspace builds a `reeve` CLI binary — source: `crates/reeve/Cargo.toml`.

## Considered options
- **Lorekeeper unaware of Reeve; humans copy keys between tools** — no cross-tool coupling.
- **Reeve exposes an MCP server that Lorekeeper connects to** — richer tool surface for agents.
- **Reeve exposes a CLI with JSON output; Lorekeeper calls it from dedicated planning skills when the manifest declares an instance** — one surface for humans, agents and skills, degrading gracefully when absent.

## Decision
Daniel Thyselius makes Lorekeeper depend on Reeve through the `reeve` CLI, gated on a `reeve` section in the household manifest and a `reeve` on the path whose version output names the planner, implemented as new planning-initiative and refining-epic skills, one link step in recording-decisions and one context-reading step in brainstorming, and keeps Reeve free of any Lorekeeper dependency, because the planner is the product every client talks to and a CLI is the surface the plugin already knows how to drive.

- **R1** The household manifest schema MUST gain an optional `reeve` section with the instance URL, validated by Lorekeeper's doctor and migrate scripts and present in the witan-household template; its absence MUST leave every Lorekeeper skill behaving as before.
- **R2** Skills MUST detect the planner by `reeve --version` reporting `reeve-planner`; the runner's CLI MUST be renamed so it never answers to `reeve`.
- **R3** Every `reeve` command used by a skill MUST support `--json`, MUST be idempotent on repeat, and MUST exit non-zero with a typed code on refusal.
- **R4** Initiative and epic planning MUST be separate skills that produce no spec; the brainstorming skill's hard gate and checklist MUST NOT be weakened; recording-decisions MUST link a drafted record to its required decision through the CLI when the planner is present.
- **R5** Reeve MUST NOT require Lorekeeper to be installed to perform any planning function, including the record check; an MCP server, if ever added, MUST wrap the same commands and MUST NOT expose capabilities the CLI lacks.

## Consequences
- The manifest schema change ripples through witan-household's template and Lorekeeper's doctor and migrate scripts, a three-tool change.
- Two new skills and two new steps are more test surface for Lorekeeper.
- Skills run under an automation token in milestone one, so every write is attributed to a person via automation and cannot override the readiness gate.

## Assumptions and invalidation triggers
- *Assumes a shell CLI is an adequate agent surface.* Trigger: a skill needs streaming or long-lived state the CLI cannot provide ⇒ add MCP under R5.
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

# ADR-0007: Every item takes a key from one sequence per household with a short prefix, and imported Jira keys are kept as external references

## Status
Proposed 2026-10-25.

## Context
Keys are the one identifier that leaves the tool: they appear in branch names, commit messages, PR titles, required-decision stubs and conversations. Jira uses one sequence per project, so the team has PIB-10 for an idea and SCRUM-586 for a story, which tells a reader which project but not which level. Linear uses one sequence per team for every issue type. The household is the grouping unit in Reeve. Beads uses hash ids to avoid collisions under concurrent offline creation, which the planner does not need because it is a single server.

## Facts relied on
- F1 Jira Product Discovery and the delivery project use separate key spaces, PIB and SCRUM — source: https://grantigo-team.atlassian.net/jira/polaris/projects/PIB and the SCRUM board, observed 2026-10-25.
- F2 Required-decision stubs attach to items by key and bind to records by id — source: kb/ADR-0005 R4.
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
- **R4** Artifact paths MUST use the item key, and branch names MUST be `<key>-<attempt>`, never the internal id.

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

**Title:** ADR-0008: Executors are pluggable clients that claim ready stories under a lease and epoch and report sessions and activities; scheduling and budgets live in the executor behind three planner-side brakes

**Tier:** high

**Home:** shared home (household knowledge base `adrs/`)

```markdown
---
id: kb/ADR-0008
title: "ADR-0008: Executors are pluggable clients that claim ready stories under a lease and epoch and report sessions and activities; scheduling and budgets live in the executor behind three planner-side brakes"
description: "Any holder of an agent-principal token that implements claim, heartbeat, session state and typed activities over the HTTP API is an executor; a claim issues a lease and a fencing epoch and starts a session with a clear stop flag, stop is read from a per-session endpoint, and the planner enforces only a live-session cap, lease expiry and token revocation while scheduling and budgets stay in the executor, because the planner must not privilege one runtime yet must be able to stop any of them."
tags: [adr, reeve, executor, loops, protocol, agents]
status: proposed
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security]
scope: ["reeve:crates/reeve-protocol/**", "reeve:crates/reeve-identity/**", "reeve:crates/reeve-planning/**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0001, kb/ADR-0003, kb/ADR-0004]
related: []
implements: []
rfc:
---

# ADR-0008: Executors are pluggable clients that claim ready stories under a lease and epoch and report sessions and activities; scheduling and budgets live in the executor behind three planner-side brakes

## Status
Proposed 2026-10-25.

## Context
Autonomous loops are a later milestone, but the planning model shipped first must not block them. Published orchestrators share one shape: the tracker is the state machine, a supervisor polls, claims atomically, caps concurrency, times out stalls and backs off, and each task runs in an isolated workspace whose agent pushes a branch and never merges. Symphony keeps scheduling in the supervisor and stops a run when the ticket's state changes. Linear models the agent as a delegate with a typed session. Review of an earlier draft found that silence-based staleness without a lease lets a partitioned executor resume and push over a successor's branch, that a global event stream leaks other households' activity to a hosted executor and can be missed, that a story-level stop flag leaked into the next attempt, and that a planner with no brake at all cannot stop a third-party runtime. The executor behind Reeve could be the existing runner, Claude Code background agents, or a hosted sandbox service, and that choice is best made with usage evidence.

## Facts relied on
- F1 Symphony polls on an interval, claims by state and labels, caps concurrency, requeues on stall, and stops a run when the ticket state changes — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §5.
- F2 Reeve's sandbox puts no credentials in the container and mediates GitHub through the host — source: `docs/superpowers/specs/2026-04-29-reeve-design.md` §Sandbox properties.
- F3 Reported failures of unattended loops are runaway cost and stuck retries, mitigated by iteration caps, stall timeouts and budgets — source: research digest §5.

## Considered options
- **Scheduler inside the planner that pushes work to registered executors** — one place for caps and budgets.
- **Executors poll and claim under a lease and epoch; scheduling and budgets inside each executor; three brakes in the planner** — the planner stays a state machine any runtime can drive and can still stop any of them.
- **No contract; the Reeve runner reads the database directly** — least API work, but binds the planner to one executor.

## Decision
Daniel Thyselius defines an executor as any client holding an agent-principal token that implements claim, heartbeat, session state and typed activities over the public HTTP API, with a lease and a fencing epoch issued at claim, a per-session stop endpoint, and scheduling and fine-grained budgets inside the executor behind three planner-side brakes, because the planner must remain a state machine that any runtime can drive while retaining the means to stop one.

- **R1** A claim MUST succeed only if the story is ready, delegated to that agent principal, in todo or in progress without a live session, and MUST atomically create a pending session with a new epoch, a lease and a clear stop flag and move delivery to in progress under the row version.
- **R2** Every session, activity, PR and bridge call MUST carry the session epoch; the planner MUST refuse a superseded epoch with conflict; a session whose lease expires without heartbeat MUST become stale and MUST NOT count as live.
- **R3** The authoritative stop channel MUST be a per-session poll endpoint exposing the session's stop flag; the event stream MUST be filtered to the principal's scope and is an optimisation only; an executor MUST tear down when stop is set or its epoch is superseded; a user's stop-and-undelegate MUST also clear the story's delegate.
- **R4** An agent principal MUST be distinct from a user; a story's owner MUST remain a user; a story MUST be delegated to at most one agent principal; pools, if wanted, MUST be an executor-side concern.
- **R5** Executors MUST NOT hold repository or planner credentials inside the agent's sandbox; pushing, PR creation and planner calls MUST pass through an executor-owned bridge outside the sandbox.
- **R6** The planner MUST enforce a per-principal cap on live sessions, lease expiry and token revocation, and MUST NOT enforce fine-grained budgets or scheduling; the session MUST carry reported cost and duration.
- **R7** Branch names MUST be `<key>-<attempt>` so that successive sessions on one story never share a branch.

## Consequences
- Two executors can race on the same story and one loses the claim, which is intended.
- A partitioned executor that rejoins is refused on its stale epoch and cannot damage its successor's work.
- The planner's brakes are coarse; fine budgets remain the executor's job, and oversight shows reported cost.
- Milestone one ships principals with kinds, the delegate as a principal, the activity log, row versions and attempt-numbered branches so that this contract is additive later.

## Assumptions and invalidation triggers
- *Assumes executors are trusted deployments operated by the team or a vendor the team configured.* Trigger: untrusted or multi-tenant executors ⇒ supersede with finer server-side limits.
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
related: [reeve/ADR-0004]
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
- reeve/ADR-0001, reeve/ADR-0004, reeve/ADR-0005.
```

### reeve/ADR-0003

**Title:** ADR-0003: Users sign in through Google OpenID Connect or a GitHub OAuth adapter, identified by allowlisted verified email, with admin-assigned roles, typed tokens and no password store

**Tier:** high

**Home:** local home (`reeve/docs/adr/`)

```markdown
---
id: reeve/ADR-0003
title: "ADR-0003: Users sign in through Google OpenID Connect or a GitHub OAuth adapter, identified by allowlisted verified email, with admin-assigned roles, typed tokens and no password store"
description: "The planner authenticates humans through Google OpenID Connect and a GitHub OAuth adapter, treats an allowlisted verified email as the user identity, holds new users pending approval until an admin assigns a role, issues interactive and automation tokens with different powers, bootstraps the first admin at deployment, and stores no passwords, because not every team member has GitHub, an open sign-in would expose the roadmap, and automation must not pass for a person."
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

# ADR-0003: Users sign in through Google OpenID Connect or a GitHub OAuth adapter, identified by allowlisted verified email, with admin-assigned roles, typed tokens and no password store

## Status
Proposed 2026-10-25.

## Context
The team is about six people, including non-coders who have no GitHub account; everyone has a Google identity. GitHub's human sign-in is OAuth 2.0 followed by API calls, and verified emails come from a separate endpoint, so it cannot be configured as a plain OpenID Connect provider. Mothra built seat and role management and an Entra sign-in that was blocked on admin consent, and that work displaced the product's core. The CLI needs a credential that is not a browser session, Lorekeeper skills act for a person but must not be able to use the human-only readiness override, and a token that outlives its owner is a hole. Review of an earlier draft found that without an allowlist any verified Google account could sign in and read every roadmap and decision, that deriving roles from GitHub teams with daily refresh was a sync pipeline for six people, and that a fresh instance had nobody who could approve the first user.

## Facts relied on
- F1 The household manifest declares a core-non-coders GitHub team with read access, and not every member of the team is on GitHub — source: `/home/daniel/Source/grantigo/household.json`; team lead's statement in the design session.
- F2 GitHub authorizes OAuth apps and lists verified emails through its REST API — source: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps and https://docs.github.com/en/rest/users/emails.
- F3 Mothra's deployment was blocked on Entra admin consent and its auth hook fell back to the first user — source: `docs/superpowers/research/2026-10-25-agent-first-planning-research.md` §3.

## Considered options
- **GitHub OAuth only, roles from GitHub teams** — simplest for developers, and the roles come for free; excludes members without GitHub and adds a team-sync pipeline.
- **Local accounts with passwords and invitations** — works for everyone; requires hashing, reset flows and invitation management.
- **Google OpenID Connect plus a GitHub OAuth adapter, allowlisted verified email as identity, pending approval with admin-assigned roles, typed tokens, bootstrap admin** — covers everyone without a password store, a sync pipeline, or an open door.

## Decision
Daniel Thyselius authenticates humans through Google OpenID Connect and a GitHub OAuth adapter, identifies a user by verified email that matches an allowlist so that both providers resolve to one user, holds new users pending approval until an admin assigns a role, issues interactive and automation tokens with different powers, bootstraps the first admin at deployment, and stores no passwords, because that covers every team member with the least account management and no open door.

- **R1** The planner MUST NOT store password hashes or implement password reset; sign-in MUST go through Google OpenID Connect or the GitHub OAuth adapter, and the GitHub adapter MUST take verified emails from GitHub's email endpoint.
- **R2** A user MUST be keyed by verified email; sign-in MUST be refused unless the email matches an allowlisted domain or address; a new user MUST have role none and status pending approval until an admin approves; two providers presenting the same verified email MUST resolve to one user.
- **R3** Roles MUST be admin, member or viewer and MUST be set only by an admin; every authenticated request MUST check the owner's current role and status; disabling a user MUST revoke every token they own; the first admin MUST be created by `reeve-server bootstrap-admin <email>` at deployment, which allowlists the email with role admin.
- **R4** Tokens MUST be interactive or automation; the CLI MUST authenticate with a token issued from the web client; automation tokens MUST NOT override readiness, administer, or mint tokens, and their writes MUST be logged as the owner via automation.
- **R5** Planner tokens MUST NOT be placed inside an agent sandbox; a bridge outside the sandbox MUST hold them.

## Consequences
- Adding a provider is configuration plus a callback for OpenID Connect providers, or an adapter for OAuth-only ones; Microsoft can follow if needed.
- Email as identity trusts the provider's verification; an unverified email is rejected at sign-in.
- Milestone one has no per-item access control; an approved viewer sees every household in the instance.
- An admin approves each new user once and picks a role, which is the whole of account management; GitHub team membership is not consulted.

## Assumptions and invalidation triggers
- *Assumes every team member has a Google or GitHub identity.* Trigger: a member with neither ⇒ add a provider, never a password.
- *Assumes manual role assignment stays cheap.* Trigger: the instance serves more people than one admin can keep current ⇒ supersede with role derivation from a directory.

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

# ADR-0004: The planner is self-hosted and single-tenant, with the household as the only grouping

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
Reeve's domain-driven refactor was motivated by a concrete failure: a single large handlers file meant two agents working unrelated contexts collided on the same file. The resulting rules are documented as standards prose in the knowledge base but never recorded as a decision, so a successor could drift from them without anyone noticing. An earlier draft of the planner proposed seven crates, with separate decision, household and discussion contexts; review found that required decisions, households and comments are all planning objects and that separate contexts for them added boundaries to police without a second team or a second deployable to justify them. Mothra went the other way, with heavy event modules and projections for a small tool, and paid for it in drift bugs.

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
- **R3** Tables MUST be owned by one context; foreign keys MUST NOT cross contexts; references MUST be by id or key; households, initiatives, epics, stories, required decisions, comments and sessions MUST all live in the planning context, and the household key sequence MUST be owned there.
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

