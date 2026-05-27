---
title: Card — kanban metadata + column-transition policy
description: The Card bounded context. Owns the kanban entity that represents one unit of work, its column lifecycle, and the cross-aggregate policy that drives Workspace state when a column changes.
tags: [domain, ddd, core, card, kanban]
---

# Card

## Purpose

Card is the central aggregate of Reeve: one feature's worth of work, modelled as a kanban item that flows through six columns. The Card context owns:

- The Card entity (id, household reference, brief, column, timestamps, `needs_heavy` flag, optional `parent_id`).
- The Column lifecycle (Backlog → Planning → Implementing → Testing → Review → Done) and the policy that decides what *other* aggregates must do when the column changes.
- The two cross-aggregate *ports* Card consumes from neighbouring contexts (`WorkspaceLifecycle`, `HouseholdLookup`). They live with Card because the *consumer* defines what it needs; the suppliers implement them.

A Card has no behaviour of its own beyond moving through columns — provisioning, container management, fetch, and push all belong to the [[domain/workspace]] aggregate, which Card invokes via `WorkspaceLifecycle`.

## Key entities

- **Card** — root aggregate. Identity is a UUID generated at creation. Holds `household` (a name reference to [[domain/household]]), `brief` (1–200 chars, the human-readable summary), `column`, `needs_heavy` (whether the card requires the heavy lease in Testing), `parent_id` (set when this card is a branch of another), `created_at`, `updated_at`.
- **Column** — pure value enum: `Backlog`, `Planning`, `Implementing`, `Testing`, `Review`, `Done`. Wire-form via `as_wire_str()`/`from_wire_str()`; the same lowercase form is the SQLite `column_` field, the CLI argument parser's vocabulary, and the JSON envelope. Single source of truth.
- **`WorkspaceLifecycle`** (port, defined here) — what Card needs from Workspace: `ensure_running(&Card)` and `ensure_stopped(&str)`. Implemented in [[domain/workspace]].
- **`HouseholdLookup`** (port, defined here) — what Card needs from Household: `exists(&str) -> bool`. Implemented in [[domain/household]].

## Ubiquitous language

- *Card* — the kanban item. Not *task* (that name is reserved for a future sub-card concept) and not *issue* (Jira-flavoured; we don't have it).
- *Column* — the kanban state. Not *status* (that means container status). Not *stage* (overloaded with the milestone-stage concept in our internal refactor history).
- *Backlog/Planning/Implementing/Testing/Review/Done* — the six column names in canonical kanban order. Title-case in display, lowercase on the wire.
- *Brief* — the human-written summary of what the card is for. Always non-empty, max 200 chars (the constructor enforces it).
- *Branch* (as a verb on a Card) — to spawn a sibling card seeded with the parent's history. `parent_id` is set; the child is otherwise independent.
- *Needs heavy* — flag set at creation (or during Planning) declaring the card requires the household's heavy lease during Testing. Default false.

## Invariants

These are enforced in the domain layer (`reeve-card/src/domain.rs::Card`) and pinned by tests. They are part of the Card aggregate's contract:

- **Card invariants** — id is non-empty, brief is non-empty and ≤ 200 chars. `Card::new` and `Card::from_persisted` are the only constructors; they fail with `CardError` on violation. Direct struct construction is impossible (private fields).
- **Lifecycle runs before column commit.** When a card moves to a new column, the cross-aggregate side effects (`WorkspaceLifecycle::ensure_running` / `ensure_stopped`) run *first*. If they succeed, the column is committed. If they fail, the card stays in its current column with the error propagated to the caller. This means: no half-state where Card is "Implementing" but the workspace failed to provision. Fixed in `fe52675` after observing the inverse cause subtle bugs.
- **Per-card serialization of mv/delete.** All RPCs that mutate column state (mv, mv-self, delete) acquire `state.lock_card(&card_id).await` *before* reading the card and *hold it until* both DB write and lifecycle side effects complete. Concurrent mv calls on the same card are thus serialised; cross-card concurrency is unconstrained. Introduced in `53dcb5d`.
- **Handler owns the clock.** Repository methods that update timestamps take `now: i64` as an argument; they do *not* call `SystemTime::now()` themselves. The handler computes `now_secs()` once at the start of the operation and passes it through. Makes tests deterministic and surfaces the "what time is it" decision at the boundary. Introduced via `update_brief`/`update_column` in `2a4169d`.
- **Household existence checked at `mv → Implementing`** (and at `mv-self → Implementing`). The handler calls `HouseholdLookup::exists(&household)` *before* mutating; missing household returns `INVALID_ARGUMENT` and the card stays put. This is the only pre-mutation precondition; other transitions (e.g. → Done) need no household lookup.

## Column-transition policy

The transition matrix is the only place that knows how a column change maps to Workspace actions. Pure function in `reeve-card/src/transitions.rs`:

```rust
pub async fn on_column_change(
    old: Column, new: Column,
    card: &Card,
    workspace: &dyn WorkspaceLifecycle,
) -> Result<()>;
```

| Transition | Effect |
|---|---|
| same column | no-op |
| Inactive → Active | `ensure_running(card)` |
| Active → Active | no-op (container + agent persist; conversation continues) |
| Active → Inactive | `ensure_stopped(card_id)` |
| Inactive → Inactive | no-op |

Where Active = {Planning, Implementing, Testing, Review} and Inactive = {Backlog, Done}. The container exists for any Active column; transitioning between active columns is purely a metadata update.

**Agent-driven transitions.** The agent inside the card's container calls `reeve mv-self <column>` at natural phase boundaries (brainstorming + planning → `planning`; executing → `implementing`; tests → `testing`; ready for review → `review`). User-issued `mv` remains available as a corrective override. Both go through the same per-card lock and the same transition matrix; the only difference is the caller-context origin.

Tests in `reeve-card/src/transitions.rs::tests` pin each arm with a `MockFx` impl of `WorkspaceLifecycle`.

## RPC surface (8 methods)

| Method | Handler | Description |
|---|---|---|
| `ls` | `handle_ls` | List all cards, ordered by `created_at`. |
| `new` | `handle_new` | Create a card in Backlog. Constructor enforces invariants. Returns the created `Card`. |
| `mv` | `handle_mv` | Move a card to a new column (host-socket call). Acquires per-card lock; runs lifecycle; commits column. |
| `mv-self` | `handle_mv_self` | Same as `mv` but from the per-card socket — the agent moves *itself*. Caller context supplies the card id; the agent cannot move a different card. |
| `status-self` | `handle_status_self` | From the per-card socket: return the calling card + its container status. Read-only. |
| `session-init-self` | `handle_session_init_self` | From the per-card socket: render the SessionStart prompt for the calling card (card identity + brief + household's spec/plan paths). Read-only; consumed by claude's SessionStart hook to inject planning instructions. |
| `edit` | `handle_edit` | Rename the card's brief. Invariants re-checked. |
| `delete` | `handle_delete` | Delete a card. If currently Implementing, tears down the workspace first via `WorkspaceLifecycle::ensure_stopped`. Other columns delete directly — for cards in Testing/Review, the user is responsible for `mv → Done` first if they want a guaranteed teardown (the transition matrix's `* → Done` ensures stopped). |

The dispatch table at `crates/reeved/src/dispatch.rs` routes each method through the appropriate `reeve_card::handle_*` function. Handlers receive port references (`&dyn CardRepo`, `&dyn WorkspaceLifecycle`, `&dyn HouseholdLookup`) — no `DaemonState` kitchen sink.

## Integration points

- **Inbound:** RPC dispatch via Unix socket (host socket for the user; per-card sockets for the agent).
- **Outbound to [[domain/workspace]]:** the `WorkspaceLifecycle` port. Card never holds a `WorkspaceRepo` or any workspace concrete; it knows only the trait.
- **Outbound to [[domain/household]]:** the `HouseholdLookup` port (existence check). Card knows nothing about manifest structure or repo declarations.
- **Persistence:** `CardRepo` trait + `SqliteCardRepo` adapter in `reeve-card/src/repo.rs`. The cards table owns its schema; no FKs to other contexts.
- **Wire boundary:** `From<Card>` and `TryFrom<protocol::Card>` bridge `reeve_card::Card` (domain entity with private fields) and `reeve_protocol::Card` (wire DTO). See [[frameworks/rust/from-tryfrom-boundary-conversion]].

## Key workflows

1. **Creating a card.** Host calls `new` → handler calls `Card::new(household, brief, needs_heavy)` → invariants checked → `CardRepo::insert` writes the row → wire DTO returned. Pure write, no side effects on other aggregates.
2. **Moving to Implementing.** Host calls `mv id=X column=Implementing` → handler acquires `state.lock_card(X)` → reads existing card → `HouseholdLookup::exists(card.household)?` → if false, return `INVALID_ARGUMENT` → otherwise call `on_column_change(old=current, new=Implementing, card, workspace_lifecycle)` → policy calls `workspace.ensure_running(card)` → on success, `CardRepo::update_column` commits the column → wire DTO returned. On any failure, card stays in old column.
3. **Self-move from inside a container.** Per-card socket → caller_context.card_id = X → handler calls `mv_self(X, new_column)` → same as `mv` except the card_id comes from the socket identity, not from params. The agent cannot move another card.
4. **Renaming a card.** `edit id=X brief="new text"` → handler validates new brief → `CardRepo::update_brief(id, brief, now)` → wire DTO returned. No cross-aggregate effects.
5. **Deleting a card.** `delete id=X` → handler reads card → if column == Implementing: `workspace_lifecycle.ensure_stopped(X)` first → then `CardRepo::delete(X)` → empty result. Other columns skip the teardown step.

## See also

- [[general/architecture]] for the broader rules this context follows (vertical-slice crate, inverted-dep ports, `pub(crate)` adapters).
- [[frameworks/rust/inverted-dependency-ports]] for *why* the two ports live in `reeve-card` (the consumer crate).
- [[frameworks/rust/from-tryfrom-boundary-conversion]] for the γ pattern that bridges `reeve_card::Card` and `reeve_protocol::Card`.
- [[domain/workspace]] for what `WorkspaceLifecycle::ensure_running` actually does.
- [[domain/household]] for what `HouseholdLookup::exists` checks.
