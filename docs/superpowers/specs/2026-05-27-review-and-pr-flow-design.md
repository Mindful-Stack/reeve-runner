# Review and PR flow — human-gated columns + isolated PR collaboration

**Status:** Design (2026-05-27)

## Context

The Planning column spec ([2026-05-26](2026-05-26-planning-column-design.md))
generalised "Active" columns and made Planning agent-driven, but left the
back half of the lifecycle thinly designed: Testing and Review wording in the
SessionStart prompt was acknowledged as a placeholder, and the column order
(`Implementing → Testing → Review → Done`) reflected an earlier model where
Testing meant "the agent runs integration tests" and Review meant "ready for
human sign-off."

We now want the back half of the flow to model a *real-world* local-first
review: the human eyeballs the diff in their preferred git tool, runs the
feature locally, ships it as a PR, then collaborates with the agent on PR
feedback before merging. This spec defines the column reorder, the new PR
column, the host-mediated PR bridge that preserves devcontainer isolation,
and the v1 shape of the `reeve review` command.

## Summary

**Column reorder and new PR column:**

```
Backlog → Planning → Implementing → Review → Testing → PR → Done
            agent       agent        human    human    agent+human   terminal
```

- **Review** is local human code review — eyeballs on the diff in GitKraken,
  sublime-merge, or whatever the user prefers. The agent has called
  `mv-self review` to declare "I think I'm done; look at this."
- **Testing** is local human manual testing — the human runs the feature,
  pokes at it, decides whether it works.
- **PR** is active (container alive) and is where the agent and human
  collaborate on PR comments. Reeve opens the PR on entering this column;
  `gh` runs on the host, never inside the container.
- **Done** is terminal and inactive; the user marks it manually when the PR
  is merged externally.

**Isolation preserved.** The devcontainer never gets `gh` or GitHub
credentials. The daemon on the host owns the `gh` integration and exposes a
handful of new self-scoped RPCs (`pr-create-self`, `pr-status-self`,
`pr-comments-self`, `pr-reply-self`) that the agent invokes via the per-card
socket — the same trusted-boundary pattern used by `mv-self`,
`status-self`, and `session-init-self`. The agent can only act on its own
card's PR, scoped by `caller_context.card_id`.

**Review tooling stays bring-your-own.** v1's `reeve review <card-id>` just
prints what's on disk (per-repo refs, commit summaries vs. main) and lets
the user open it in whatever tool they already use. No bespoke ratatui diff
viewer, no per-tool launcher config, no shell-out gymnastics. We deliberately
defer a bespoke review UI to v2; the data we want to gather first is how
people actually use this flow.

## Architecture

### Column semantics

| Column | Active? | Driver | Meaning |
|---|---|---|---|
| Backlog | inactive | — | parked, not started |
| Planning | active | agent | brainstorming + writing spec/plan |
| Implementing | active | agent | executing the plan |
| **Review** | active | **human** | local code review of the diff |
| **Testing** | active | **human** | local manual testing of the feature |
| **PR** | active | agent + human | PR is open; comment loop in progress |
| Done | inactive | — | shipped (merged on GitHub) |

The agent calls `mv-self` for the agent-driven boundaries (planning →
implementing → review). The human calls `mv` for the human-gated
boundaries (review → testing on local-approve, testing → pr on
ship-it, pr → done on merge-confirmed). Either side can call `mv` to send
work *back*: e.g. the human moves a card from Review or Testing back to
Implementing if changes are needed; the container stays alive, the
conversation persists, the agent picks up where it left off.

### Transition matrix

Unchanged from the planning-column spec — still a 4-arm matrix over
`{Active, Inactive}`:

| Transition | Effect |
|---|---|
| Inactive → Active | `ensure_running(card)` |
| Active → Active | no-op (container + agent + conversation persist) |
| Active → Inactive | `ensure_stopped(card_id)` |
| Inactive → Inactive | no-op |

Active set grows from four to five: `{Planning, Implementing, Review,
Testing, PR}`. Inactive set is still `{Backlog, Done}`. The
`Testing → PR` transition has a new side effect (push + `gh pr create`)
that is *layered on top of* the active→active no-op, not part of the
matrix itself — see "PR column" below.

### Review column

The fetch worker already mirrors every card branch into the host's
meta-repo and into every sibling that has its own `.git/`, at
`refs/reeve/<card-id>/<branch>`. By the time a card enters Review, the
work is already on disk in the user's host repos. The Review column is
just a kanban label saying "now's the time to look."

**v1 `reeve review <card-id>` command.** Print-only. For each repo in
the household that has a card branch with commits vs. main:

```
$ reeve review a3f2b1c5-...

Card a3f2b1c5  brief="refactor the auth middleware"  household=witan
Status: in Review since 2026-05-27 14:33

Affected repos:
  meta-repo (reeve)
    ref:    refs/reeve/a3f2b1c5/refactor-auth
    head:   d4e5f6a  refactor(auth): consolidate session lookup
    commits ahead of main: 7

  sibling: lorekeeper
    ref:    refs/reeve/a3f2b1c5/refactor-auth
    head:   1a2b3c4  refactor(auth): adopt new session port
    commits ahead of main: 2

Open in your tool of choice. To proceed:
  reeve mv a3f2b1c5 testing       # approve, move to local testing
  reeve mv a3f2b1c5 implementing  # send back for changes
```

No tool launching, no local branch materialisation, no worktree creation.
GitKraken and sublime-merge can both browse arbitrary refs in the
`refs/reeve/*` namespace; users navigate there themselves. If we learn
later that this friction is real, the host-side launcher hook is an
additive change.

The command is read-only — it does not mutate card state. It is purely
informational. Implementation lives in the CLI crate (`crates/reeve/`)
and talks to the daemon through a new read-only `review-info` RPC that
returns per-repo ref + commit-ahead counts. The handler lives in
**`reeve-workspace`** (like `attach` and `push`, it is primarily a
git-state read over the household's repos); it consumes
`CardRepo::get` + `HouseholdLookup` + `GitOps` to assemble its
response.

**Iteration loop.** If the human spots issues during Review, they:

1. `reeve attach <card-id>` to enter the live agent session
2. Tell the agent what to fix (free-form chat)
3. Optionally `reeve mv <card-id> implementing` to make the kanban
   reflect what's happening

The agent then fixes things, eventually calls `mv-self review` again,
loop continues. No new machinery needed — `attach` + `mv` already exist.

### Testing column

Local manual testing by the human. The container is alive (the dev
container is the agent's home, not the test environment — the human is
testing the feature outside the container, against whatever target
environment the household runs in).

The agent is *idle* during Testing. It does not run anything autonomously.
If the human wants to ask the agent a question or hand it a small fix
while testing, they `reeve attach`; otherwise the agent just sits there
with its conversation history intact.

The `needs_heavy` flag on the Card aggregate is unchanged by this spec.
It was designed to gate exclusive access to a household-scoped heavy
resource during automated testing; that design is orphaned in the
human-Testing world and should be reconsidered in a follow-up. For v1
it remains a no-op piece of metadata.

### PR column

Entering PR is the user's explicit "ship it" decision: `reeve mv <id> pr`.
The transition has a side effect:

1. For each repo with a card branch ahead of main:
   1. Push the branch to `origin` (reuse the `reeve push` internals).
   2. Run `gh pr create --base main --head <branch> --title <title>
      --body <body>` (host-side, in the host's clone of the repo).
   3. Record the resulting PR number against the card.
2. Commit the column change.

If any step fails, the card stays in Testing (per the
lifecycle-runs-before-column-commit invariant). Partial-success across
multiple repos is logged but does not roll back already-opened PRs;
re-running `mv pr` is safe because we look up existing PRs by branch
before trying to create.

**Title and body.** v1 defaults:
- Title: the card brief (truncated to 72 chars if needed).
- Body: a short Reeve-generated stub plus the commit list:

```
Card a3f2b1c5 from the witan household.

Brief: refactor the auth middleware

7 commits — generated by Reeve.
```

These defaults are *intentionally minimal*. The bespoke "fill out the PR
body" workflow is the agent's job in the PR column (it can call
`pr-edit-self` to rewrite the body once it sees the diff). We don't try
to be clever about title/body at creation time.

**Multi-repo PRs.** A card can touch the meta-repo and any number of
siblings. Each affected repo gets its own PR; they are tracked per-repo.
There is no cross-repo coordination layer (e.g., we don't try to gate
merges or stage commits across repos). The user opens them in whatever
order GitHub presents.

### gh-on-host bridge

The daemon owns `gh`. The container has no gh binary, no
`~/.config/gh`, no `GITHUB_TOKEN`. The container's only path to GitHub
is through the daemon, via new self-scoped RPCs:

| RPC | Daemon action | Returns |
|---|---|---|
| `pr-create-self` | already done at `mv pr`; this RPC just looks up the recorded PR(s) | `Vec<{repo, pr_number, url}>` |
| `pr-status-self` | `gh pr view <num> --json state,mergeable,statusCheckRollup` per recorded PR | per-PR state + CI status |
| `pr-comments-self` | `gh api repos/<owner>/<repo>/pulls/<num>/comments` + `gh pr view --json comments` | list of `{id, author, path, line, body, in_reply_to}` |
| `pr-reply-self` | `gh api ... -X POST` to post a review comment reply | the posted comment's id |
| `pr-edit-self` | `gh pr edit <num> --title ... --body ...` | confirmation |

Each handler:
1. Reads the calling card via `caller_context.card_id`.
2. Looks up the card's PR records (new `card_prs` table — see Schema).
3. If the requested operation references a PR, verifies it belongs to
   *this* card. Reject otherwise.
4. Shells out to `gh` via `tokio::process::Command`, run from the host's
   clone of the relevant repo (so `gh` resolves the right
   owner/repo automatically).
5. Parses JSON output and returns wire DTOs.

The agent inside the container experiences this as "I have a set of PR
verbs available." It cannot run arbitrary `gh` commands. The daemon
enforces card-scoping. Auto-mode (`--dangerously-skip-permissions`)
stays safe because the worst the agent can do via these verbs is post
comments on its own PR — which is a controlled action we've authorised
by exposing the verb.

The host's `gh` must be authenticated before Reeve will be useful here;
documented as a prereq. If `gh auth status` fails, `pr-*-self` calls
return a clear error and the agent falls back to telling the user "gh
isn't logged in on the host, please run `gh auth login`."

### PR → Done

Manual. The user runs `reeve mv <id> done` when they see the PR is
merged on GitHub. No polling, no webhooks, no GitHub Actions
integration. Reeve tracks *the user's understanding* of card state, not
the literal GitHub state.

`mv done` is `Active → Inactive` and runs `ensure_stopped`: container
torn down, conversation lost, listener stopped. The PR records on
`card_prs` are kept for audit; nothing references them after Done.

### Iteration back to Implementing

From any active column, `reeve mv <id> implementing` is valid and
preserves the container. This is the "send it back" lever. The agent
sees the column change on its next `status-self` and resumes work.

From PR specifically, this is the common case: PR feedback says "this
needs a code change," user moves the card back to Implementing, agent
addresses the comment via the normal code-write loop, then
`mv-self review` re-enters the loop. The PR stays open the whole time;
the new commits push themselves into the existing PR via the standard
push step (when re-entering PR).

Edge case: if the user reverts to Implementing from PR, the open PRs
stay open on GitHub — we don't close them. Re-entering PR re-pushes
(no-op if already up-to-date) and re-records (idempotent: same PR
numbers, no duplicate `card_prs` rows).

## SessionStart prompt update

The Planning-column spec defined a SessionStart prompt template that
named the kanban phases. With the column reorder and the new PR column,
the relevant section becomes:

```
Take this card through these phases:
  1. Brainstorm the design  →  <spec_path>/<id>-design.md
  2. Write the implementation plan  →  <plan_path>/<id>.md
  3. Execute the plan
  4. Submit for human review

Your household's installed plugin provides the skills for each
phase — invoke them as they apply.

Signal phase changes to the kanban board as you progress:
  reeve mv-self planning      (during brainstorming + planning)
  reeve mv-self implementing  (when executing the plan)
  reeve mv-self review        (when you believe the implementation is done)

The human reviews your code and tests it locally; if they ship it,
they'll open a PR and move you to the PR column. There you can:
  - reeve pr-status-self    (check PR state + CI)
  - reeve pr-comments-self  (read review comments)
  - reeve pr-reply-self     (post a reply)
  - reeve pr-edit-self      (update PR title/body)

If the human moves you back to Implementing, address their feedback
and call mv-self review again when ready.
```

The "running integration tests" line from the prior template is
removed. Testing-as-a-human-phase doesn't have an agent-side action.

## Schema changes

### `cards` table

Unchanged. The `pr_number` data lives in a separate table — there can
be more than one PR per card (multi-repo).

### New `card_prs` table

```sql
CREATE TABLE IF NOT EXISTS card_prs (
    card_id    TEXT NOT NULL,
    repo_name  TEXT NOT NULL,
    pr_number  INTEGER NOT NULL,
    pr_url     TEXT NOT NULL,
    opened_at  INTEGER NOT NULL,
    PRIMARY KEY (card_id, repo_name)
);
```

- No FK to `cards` (schema independence per
  `lore/knowledge/general/architecture.md`).
- `repo_name` matches the manifest entry name (the meta-repo's
  `household.name` or a sibling's `repos[i].name`).
- Upsert semantics on `mv pr` re-runs: same `(card_id, repo_name)`
  updates the row rather than inserting a duplicate.

Owned by **`reeve-card`** (PR records are *about* a card; the
`card_prs` table is a satellite of the card aggregate). The trait,
adapter, factory all live in `reeve-card/src/repo.rs`.

### Column enum

Add `Pr` variant. Wire form `pr`. Display form `PR`. CLI argument
parser accepts `pr` (case-insensitive). Existing pattern, single
source of truth in `reeve-card/src/domain.rs::Column`.

### `card_prs` migration ordering

New table; CREATE TABLE IF NOT EXISTS in `reeve_card::init_schema`. No
data migration needed — existing rows in the cards table get
`column_=...` unchanged; no card has PRs yet.

## RPC surface additions

In `reeve-workspace`:

| Method | Handler | Notes |
|---|---|---|
| `review-info` | `handle_review_info` | Read-only. Returns `Vec<RepoReviewInfo { repo_name, ref_path, head_sha, head_subject, commits_ahead }>` for a card. Used by the CLI's `reeve review`. Consumes `CardRepo::get` + `HouseholdLookup` + `GitOps`. |

In `reeve-card` (self-scoped, called from inside the container):

| Method | Handler | Notes |
|---|---|---|
| `pr-create-self` | `handle_pr_create_self` | Looks up the card's `card_prs` rows and returns them. Idempotent. The actual `gh pr create` runs on the host as a side effect of `mv pr`, not on this RPC. |
| `pr-status-self` | `handle_pr_status_self` | For each PR row, shells out to `gh pr view <num> --json ...`. Returns `Vec<PrStatus>`. |
| `pr-comments-self` | `handle_pr_comments_self` | For each PR row, fetches review + issue comments via `gh`. Returns `Vec<PrComment>`. |
| `pr-reply-self` | `handle_pr_reply_self` | Posts a reply to a specific comment id. Validates that the parent comment belongs to one of *this* card's PRs. |
| `pr-edit-self` | `handle_pr_edit_self` | Updates title and/or body of a specific PR (must belong to this card). |

All PR RPCs depend on a new `GhOps` port (defined in `reeve-card`,
since Card is the consumer), implemented in **`reeve-workspace`**
(where the existing `GitOps` port lives — `gh` is git-adjacent and
shells out the same way). The port surface is small:

```rust
#[async_trait]
pub trait GhOps: Send + Sync {
    async fn create_pr(&self, repo_path: &Path, branch: &str,
                       base: &str, title: &str, body: &str) -> Result<PrCreated>;
    async fn view_pr(&self, repo_path: &Path, pr_number: u64) -> Result<PrView>;
    async fn list_comments(&self, repo_path: &Path, pr_number: u64) -> Result<Vec<PrComment>>;
    async fn post_reply(&self, repo_path: &Path, pr_number: u64,
                        parent_comment_id: u64, body: &str) -> Result<u64>;
    async fn edit_pr(&self, repo_path: &Path, pr_number: u64,
                     title: Option<&str>, body: Option<&str>) -> Result<()>;
    async fn list_prs_for_branch(&self, repo_path: &Path, branch: &str) -> Result<Vec<u64>>;
}
```

Adapter `RealGhOps` shells out via `tokio::process::Command`; pure
factory `make_gh_ops`. Same pattern as `GitOps`.

The `Testing → PR` side effect lives in
`reeve-card/src/transitions.rs` and calls into the existing
`WorkspaceLifecycle::push_for_pr` (new method) which:

1. Iterates `iter_card_repos`.
2. For each repo with a card branch: fast-forward + push + (if PR
   doesn't exist) create PR.
3. Records `card_prs` rows.

Returns `Vec<PrCreated>`. The `transitions.rs` arm for
`Testing → PR` calls this *before* committing the column.

## CLI additions

| Command | Routes to | Description |
|---|---|---|
| `reeve review <card-id>` | `review-info` RPC | Print the per-repo refs + commit summaries. Read-only. |
| `reeve mv <id> pr` | `mv` RPC (existing) | Triggers the push + `gh pr create` side effect. |
| `reeve pr-status <id>` | `pr-status-self` (proxied) | Same as the self-scoped RPC, but host-callable. Convenience for the user to peek at PR state without attaching. |

The `pr-comments`, `pr-reply`, `pr-edit` variants are container-only in
v1 (called from inside the agent's tmux). The host user has `gh`
directly; we don't need to mirror those into the host CLI.

## Container image

The container needs `reeve` available so it can call the new
self-scoped RPCs. Already true — the dev container template provisions
the `reeve` binary into the workspace from M5 onward.

The container does **NOT** need `gh`. Explicitly: the dev container
template should *not* install `gh`. If a household's
`devcontainer.json` happens to install `gh` anyway, that's the
household's choice — but Reeve's RPC bridge is the *intended* path and
the only one Reeve knows about.

## Error handling

| Failure mode | Behavior |
|---|---|
| User moves Testing → PR; `gh` not installed on host | Push happens; `gh pr create` fails with a clear error message; card stays in Testing. User installs `gh` and retries. |
| User moves Testing → PR; `gh` not authenticated | Push happens; `gh pr create` fails; card stays in Testing. User runs `gh auth login` and retries. |
| User moves Testing → PR; push fails (non-FF, network) | Card stays in Testing. The standard `reeve push` error reporting applies. |
| User moves Testing → PR; one repo's PR creation fails midway | PRs created before the failure are recorded; failed repo's PR is not. Card stays in Testing (the column-commit step never ran). Re-running `mv pr` is idempotent: `list_prs_for_branch` finds the existing PRs, only the failed repo gets a new attempt. |
| Agent calls `pr-comments-self` before `mv pr` has run | RPC returns empty list. The card has no `card_prs` rows yet. |
| Agent calls `pr-reply-self` with a comment id from a different card's PR | RPC returns `INVALID_ARGUMENT`. Authorisation is by card_id. |
| User moves PR → Implementing | Container persists; PR rows persist; PR stays open on GitHub. The agent sees the new column on next `status-self`. |
| User moves PR → Done | Standard `Active → Inactive` teardown. PRs on GitHub are not touched. `card_prs` rows kept. |
| Daemon restart while card is in PR | Reconcile-on-startup adopts the running container; the agent's PR state (held in GitHub) is unaffected; `card_prs` rows survive in SQLite. |

## Testing

Per `lore/knowledge/general/testing.md`:

1. **Unit — `reeve-card/src/transitions.rs`**: pin the new `Pr` arm.
   `Testing → PR` calls `push_for_pr` then commits column. Failure of
   `push_for_pr` leaves the card in Testing.
2. **Unit — `reeve-card/src/handler.rs`**: each of `review-info`,
   `pr-create-self`, `pr-status-self`, `pr-comments-self`,
   `pr-reply-self`, `pr-edit-self` with mock `GhOps`. Cover the
   card-scoping enforcement explicitly: a reply-self attempt against a
   comment id that belongs to a different card returns
   `INVALID_ARGUMENT`.
3. **Unit — `reeve-card/src/repo.rs`**: `CardPrRepo` upsert semantics,
   list-by-card, schema migration on a fresh DB.
4. **Unit — `reeve-workspace/src/gh.rs`** (new module): pure argv
   builders for `gh pr create`, `gh pr view`, etc. No
   `tokio::process` in the unit tests; the adapter is exercised by
   integration only.
5. **Integration — `reeved/src/dispatch.rs`**: dispatch arms for the
   six new RPCs route correctly.
6. **E2E**: full flow with a real `gh`-authenticated host (gated by
   env var so CI without GitHub creds skips it): create card →
   `mv-self planning/implementing/review` → user `mv testing` →
   `mv pr` → assert `card_prs` row exists + GH PR exists →
   `pr-status-self` returns open → manual cleanup closes the PR →
   `mv done`.

## Future work (explicitly out of scope)

- **Bespoke ratatui review UI.** Decide later, after watching v1 usage.
  If most reviews happen on the host in external tools, skip it; if the
  pain shows up at a specific step, build a focused layer for that step.
- **PR-comments-as-files mirror.** A future option: the daemon could
  mirror PR comments into a file the agent can `Read` (e.g.
  `<workspace>/.reeve/pr-comments.json` updated by a poller). Cleaner
  than RPC for read-heavy access; v1's RPC bridge is sufficient.
- **PR auto-close on `mv done`.** Currently we leave the PR open on
  `mv done` because the user already merged it externally. If we ever
  flip to "Reeve does the merge for you," `mv done` would call
  `gh pr merge` instead of being a no-op on the GitHub side.
- **Cross-repo merge coordination.** When a single card opens PRs in
  multiple repos, the user merges them independently. A future spec
  might add `reeve merge <card-id>` that merges all of them in
  dependency order. Not needed for the witan household today.
- **`needs_heavy` semantics revisited.** The flag was designed for
  exclusive heavy-lease access during agent-driven Testing. Now that
  Testing is human, the flag has no enforcement point. A follow-up
  spec should either repurpose it (e.g. heavy lease during a future
  "CI" column) or remove it.
- **Review-time "request changes" with structured feedback.** v1
  expects the user to `reeve attach` and chat. A v2 could add
  `reeve mv <id> implementing --message "..."` that pipes structured
  feedback into the agent's session via a new
  `read-pending-message-self` RPC.
- **Host-side `reeve discuss <card-id>`.** Already named as future
  work in the planning-column spec; nothing in this spec changes that.

## Open questions

None at design time. Decisions taken during brainstorming:

- v1 review surface is print-only; tool integration deferred.
- gh on host; daemon mediates via self-scoped RPCs.
- PR is a separate active column (rather than a no-column side effect
  of `mv done`).
- PR → Done is manual; no GitHub polling.
- Multi-repo cards open one PR per affected repo; tracked in a
  separate `card_prs` table.

## See also

- [[domain/card]] — the aggregate this spec grows
- [[domain/workspace]] — host of `GitOps`, soon to host `GhOps`
- [[domain/household]] — the source of `meta_repo_path` for `gh` cwd resolution
- [[general/architecture]] — schema independence, port-adapter pattern, factory functions
- [[frameworks/rust/inverted-dependency-ports]] — why `GhOps` lives in `reeve-card`
- `docs/superpowers/specs/2026-05-26-planning-column-design.md` — the prior spec that established the active/inactive split and the `mv-self` model
- `docs/superpowers/specs/2026-04-29-reeve-design.md` — the original Reeve design (superseded for the back half of the lifecycle by this spec)
