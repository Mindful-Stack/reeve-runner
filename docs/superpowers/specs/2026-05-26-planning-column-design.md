# Planning column — agent-driven kanban with container-scoped sessions

**Status:** Design (2026-05-26)

## Context

The Planning column has been a parking spot since the initial Reeve design. The 2026-04-29 spec proposed running a host-side Claude Code session for Planning, on the rationale that "Planning produces markdown only" and could skip the container. That branch was never built; the current transition matrix treats `* → Planning` as a no-op (or a defensive `ensure_stopped` from active columns). Cards in Planning sit idle until manually moved to Implementing.

We now want Planning to be a working phase: the agent runs there, brainstorms the design, writes an implementation plan, and signals when the work is ready to advance. This spec defines how Planning works, how the agent transitions through phases, and how Reeve stays plugin-agnostic so that other households (e.g., Ramudden) can plug their own skill conventions in without forking Reeve.

## Summary

**Planning becomes an "active" column.** The card's dev container, agent process, and conversation persist across all transitions between Planning, Implementing, Testing, and Review. The column reflects where the agent is in its workflow — the agent drives transitions via `mv-self` as it progresses through brainstorming → planning → execution → testing → review; the user's `mv` is a corrective override. No process respawn, no `/clear`, no per-prompt context injection. Phase-specific guardrails come from the household's installed plugin (Lorekeeper for witan), not from Reeve.

Reeve seeds the agent once at SessionStart with a card-identity prompt that names the phases, target artifact paths, and `mv-self` invocations as natural phase boundaries. The agent flows through phases by invoking its plugin's skills; the kanban board is a projection of the agent's self-reported state.

## Architecture

### Active vs. inactive columns

The six columns partition into two sets:

| Set | Columns | Container state |
|---|---|---|
| Active | Planning, Implementing, Testing, Review | Container + agent + conversation exist |
| Inactive | Backlog, Done | No container; nothing to attach to |

The card workspace and dev container are provisioned the first time a card enters an active column and torn down when it leaves to an inactive column. All transitions *between* active columns preserve the container — the agent process keeps running, the conversation keeps its history, and the column is just metadata.

### Transition matrix

The matrix simplifies from five arms to four:

| Transition | Effect |
|---|---|
| Inactive → Active | `ensure_running` (existing, idempotent) |
| Active → Active | no-op (container + agent persist) |
| Active → Inactive | `ensure_stopped` (existing) |
| Inactive → Inactive | no-op |

Visible delta versus today: `* → Planning` is now an `ensure_running` arm rather than a no-op or defensive `ensure_stopped`. The `* → Testing` and `* → Review` arms become explicit `ensure_running` (idempotent, so the runtime effect when transitioning from an active column is unchanged).

### Agent-driven transitions

The agent calls `reeve mv-self <column>` at natural phase boundaries:

| Agent phase | `mv-self` target |
|---|---|
| Brainstorming + planning | `planning` |
| Executing the plan | `implementing` |
| Running integration tests | `testing` |
| Ready for human review | `review` |
| Fully complete | `done` (by user, typically) |

The existing `mv-self` RPC already supports any column target and goes through the same per-card lock as user `mv`. No new RPC needed for this.

The user's `mv` remains available and is non-destructive *for active↔active transitions* — only column metadata changes. Moves to Backlog or Done still tear down the container, but those moves are explicit "park" or "complete" actions where teardown is desired.

If the agent forgets to call `mv-self`, the kanban lags but the card still works. The user corrects with `reeve mv`.

## SessionStart hook

At provision time, Reeve writes `<workspace>/.claude/settings.local.json` with a `SessionStart` hook stanza:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "*",
        "hooks": [
          {"type": "command", "command": "reeve session-init-self"}
        ]
      }
    ]
  }
}
```

`.claude/settings.local.json` is gitignored by Claude Code convention, so it doesn't ride the card branch.

**Idempotency and existing files.** The write is unconditional — provision overwrites any existing `.claude/settings.local.json` in the workspace clone. In practice this file is gitignored and won't exist in a fresh clone; if a household ships one in its meta-repo against convention, Reeve replaces it for the card's clone (the host meta-repo is untouched). On a re-provision of an already-running card, the write is idempotent — the same contents are produced from the same template, so the file ends up byte-identical.

The hook fires once when claude boots inside the container. Its stdout is injected as additional context at the start of the session. The agent never sees the hook fire again unless the session is restarted (which doesn't happen — active↔active transitions don't respawn the process).

## New RPC: `session-init-self`

Self-scoped, read-only. Lives in `reeve-card/src/handler.rs`. The handler:

1. Reads the calling card via the caller-context card id.
2. Looks up the household via `HouseholdLookup` to get the household manifest.
3. Reads the manifest's `spec_path` and `plan_path` (with defaults if unset — see below).
4. Renders the prompt template from `id`, `brief`, `household.name`, `spec_path`, `plan_path`.
5. Returns the rendered text.

The dispatch arm in `crates/reeved/src/dispatch.rs` routes `session-init-self` to the handler. Pure function; the caller-context provides identity, so no params are needed.

## Prompt template

The rendered prompt:

```
You are reeve card <id> in the <household> household.
Brief: <brief>

Take this card through these phases:
  1. Brainstorm the design  →  <spec_path>/<id>-design.md
  2. Write the implementation plan  →  <plan_path>/<id>.md
  3. Execute the plan
  4. Run integration tests
  5. Submit for human review

Your household's installed plugin provides the skills for each
phase — invoke them as they apply.

Signal phase changes to the kanban board as you progress:
  reeve mv-self planning      (during brainstorming + planning)
  reeve mv-self implementing  (when executing the plan)
  reeve mv-self testing       (when running integration tests)
  reeve mv-self review        (when ready for human review)

The user may move your card to a different column to redirect you;
treat that as a phase-change instruction from them.
```

The template is plugin-agnostic. It names workflow verbs (brainstorm, plan, execute, test, review), not plugin-specific skill IDs. The agent's installed plugin (Lorekeeper for witan; the equivalent for other households) provides the actual skill invocations for each verb.

The Planning end of this template is the focus of v1; Testing and Review wording stays light here and gets fleshed out in a follow-up brainstorming session.

## Manifest changes

The `Household` manifest gains two optional fields:

```json
{
  "name": "witan",
  "meta_repo": "...",
  "spec_path": "docs/agent/specs",
  "plan_path": "docs/agent/plans"
}
```

Defaults when absent:
- `spec_path`: `docs/agent/specs`
- `plan_path`: `docs/agent/plans`

These are plugin-neutral defaults — households following superpowers conventions can override to `docs/superpowers/specs` and `docs/superpowers/plans`; Ramudden or other plugins can override to whatever their convention is.

Implemented as `Option<String>` on the Rust manifest struct with serde defaults; the accessor returns the configured value or the default. No breaking change to existing manifests.

## Artifact paths

Per the manifest defaults above:

- Spec: `<workspace>/<spec_path>/<card-id>-design.md` (default: `<workspace>/docs/agent/specs/<card-id>-design.md`)
- Plan: `<workspace>/<plan_path>/<card-id>.md` (default: `<workspace>/docs/agent/plans/<card-id>.md`)

Card-id (UUID) is the filename anchor, not a date+slug. Reeve has unambiguous card identity, so the host-workflow convention of `<date>-<slug>` (which exists to disambiguate when there's no card id) is unnecessary. Trade-off accepted: filenames are uglier, but `ls <spec_path>/` maps files to cards 1:1, which is more useful for tooling.

Both files are written by the agent into the meta-repo clone inside the workspace. They ride the card branch via the existing fetch worker and surface in the host meta-repo as `refs/reeve/<card-id>/<branch>`. The user reviews them as commits, merges via `reeve push <card-id>`.

## Error handling

| Failure mode | Behavior |
|---|---|
| Hook command fails (daemon unreachable, RPC error) | Claude logs the failure; session boots without injected context. User attaches and re-seeds manually if needed. Annoying, recoverable. |
| Agent forgets to call `mv-self` at a phase boundary | Kanban lags reality. User corrects with `reeve mv`. Non-destructive. |
| Daemon restart while card is active | Agent conversation persists in the container. Adopt-existing-running-container path (existing) reattaches. Next user prompt continues as normal. |
| User moves Active → Active | Column metadata updates; container and agent persist. Agent picks up the new column on next prompt via attach (it can call `reeve status-self` if confused). |
| User moves Active → Inactive (→ Backlog or → Done) | Container torn down. Conversation lost. This is intentional — parking or completing a card is supposed to release resources. |
| Manifest missing `spec_path` / `plan_path` | Defaults to `docs/agent/specs` / `docs/agent/plans`. |
| `session-init-self` called for a non-existent card | Returns `INVALID_ARGUMENT` (matches existing self-scoped RPC convention). Should not happen in practice — the per-card socket identity guarantees the card exists. |

## Testing

Coverage per `lore/knowledge/general/testing.md` (unit, integration, e2e layers):

1. **Unit — `reeve-card/src/transitions.rs`**: pin `Planning` as an active column; pin `Active → Active` as no-op; pin that `* → Planning` from inactive runs `ensure_running`.
2. **Unit — `reeve-card/src/handler.rs`**: `handle_session_init_self` renders the template correctly given a card + mock household. Cover the default-path case (manifest fields absent) and the configured-path case.
3. **Unit — `reeve-household/src/manifest.rs`**: `spec_path` and `plan_path` parse correctly when present, default correctly when absent, are backwards-compatible with existing manifests.
4. **Unit — `reeve-workspace/src/lifecycle.rs`**: the settings.local.json write step produces a parseable JSON file with the expected hook stanza, and is idempotent (re-running provision does not corrupt an existing file).
5. **Integration — `reeved/src/dispatch.rs`**: dispatch arm for `session-init-self` routes correctly and authenticates against the per-card socket identity.
6. **E2E** — full provision lifecycle: card created in Backlog → `mv Planning` → container provisioned → `<workspace>/.claude/settings.local.json` present and parseable → `reeve session-init-self` reachable from inside the container → returns the rendered prompt. Then `mv-self Implementing` from inside the container → column metadata updates, container persists, no respawn.

## Future work (explicitly out of scope for this design)

The following items came up during brainstorming and are deferred to subsequent sessions:

- **`reeve discuss <card-id>`** — host-side tmux Claude session scoped to the household's meta-repo for pre-planning fuzzy chat (no container, no card transition). Serves the "is this even worth doing?" use case before commiting to a card workflow.
- **`card.new-self` RPC** — agent inside a container creates branched sub-cards via the per-card socket; auto-sets `parent_id` to the calling card. Lets the agent defer subtasks naturally as it discovers them mid-implementation.
- **Container pre-warm during Planning** — kick off `devcontainer up` in the background while the user is still chatting, to parallelize the Implementing-transition cost.
- **`/compact`-on-transition** — opt-in setting to compact the conversation when the agent calls `mv-self`, for long-running cards where context fragmentation across phases is undesirable.
- **Fleshed-out Testing / Review / Done phase prompts** — the v1 template names these phases but is light on guidance. Separate brainstorming sessions will design the Testing and Review flows (column auto-advance criteria, lease semantics, review checkpoints, merge-to-main flow).
- **Configurable plugin hint** — a manifest field naming the household's plugin (e.g., `"plugin_hint": "Lorekeeper"`) so the prompt can reference it by name. v2 sugar; v1 leaves the prompt generic.

## Open questions

None at design time. All clarifying decisions were made during brainstorming (see conversation history); the design is internally consistent.

## See also

- [[domain/card]] — Column transition matrix and policy (this design updates §"Column-transition policy")
- [[domain/workspace]] — `LiveWorkspaceLifecycle::ensure_running` (unchanged; called from a new arm)
- [[domain/household]] — Manifest definition (this design adds two optional fields)
- [[general/architecture]] — The four-file pattern this design respects
- `docs/superpowers/specs/2026-04-29-reeve-design.md` — Original Planning trust model (superseded by this design)
