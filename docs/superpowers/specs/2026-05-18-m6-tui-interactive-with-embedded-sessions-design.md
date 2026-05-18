# Reeve M6 — Interactive TUI with embedded session pane

**Status:** Design (2026-05-18)
**Predecessor:** M5 `reeve attach <card>` (`2026-05-17-m5-attach-to-card-agent-design.md`)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

> **Path-shape note:** A vertical-slice (one-crate-per-context) refactor of the workspace is planned to land before this plan is written. The spec describes modules and responsibilities in logical terms; exact file paths resolve at writing-plans time once the refactor is in.

## Context

After M1-M5, Reeve has a working daemon, per-card containers, tmux+claude inside each container, `reeve attach <card>` for CLI access, fetch worker, push. But the TUI from M1 is still read-only: it draws the kanban from `ls` polls, accepts `q` to quit and `r` to refresh, nothing else. The user can't manage cards from it.

The user wants the TUI to be the daily driver. Three concrete needs:

1. **Manage cards from the TUI** — inspect, edit, move, create, delete without dropping to the CLI.
2. **See which cards need attention** — when claude in card A asks a question and card B's tests are running and card C is idle, the user should be able to tell at a glance and switch focus instantly.
3. **Work IN cards without leaving the TUI** — type to claude, see its response, navigate to the next card, all in the same terminal.

This is the kanban orchestrator's actual UX. Without it, the tool is technically capable but ergonomically a CLI wrapper.

## Summary

M6 rewrites the TUI as an interactive ratatui application with two new capabilities beyond the M1 viewer:

**Mutations.** Vim-style + arrow-key navigation across kanban columns. Keybindings for mv (`H`/`L`/`Shift+←`/`Shift+→`), edit brief (`e`), new card (`n`), delete (`d`), move-with-picker (`m`), and attach (`a`). All actions also reachable via a context-aware popup (`space`) that lists available actions with their letter shortcuts; disabled actions show greyed with the reason inline.

**Embedded session pane.** When a card sits in Implementing, the TUI spawns a `docker exec ... tmux attach -t reeve` connected to a pseudo-terminal that *it* controls. PTY bytes feed a `vt100::Parser`; the resulting screen renders into a Ratatui widget via `tui-term`. Focus toggles between the kanban (left) and the embedded session pane (right) with `Tab`. When the session pane has focus, keystrokes forward into the PTY; claude inside the container receives them as if `reeve attach` had been run. The user moves between Implementing cards by changing selection on the kanban; the right pane swaps to that card's session — instant, because all sessions live in memory in the background.

An activity badge marks cards whose session received output since the user last viewed it. Provides the "needs attention" signal.

Two new RPCs (`edit`, `delete`) plus reuse of existing ones (`ls`, `new`, `mv`, `attach`).

The CLI surface (`reeve ls`, `reeve new`, `reeve mv`, `reeve attach`, etc.) is **unchanged**.

M6 deliberately does *not* yet:

- Implement Planning sessions (the parent design's host-side claude per Planning card). The Planning column stays empty of agent activity until M7.
- Implement Planning → Implementing handoff. M8.
- Add idle/blocked detection beyond the activity-badge signal (no "claude is waiting for input" heuristic).
- Add full event timeline / diff view / chat tab from the parent design's drill-in. The drill-in for M6 is a side panel with card details only.
- Add multi-line brief editing or rich-text inputs. Briefs are single-line.
- Add card filtering / sorting beyond default column-and-creation-order.
- Add scrollback richer than vt100's default screen + ~1000-line ring buffer per session.
- Add cross-card operations from the TUI (`diff <other-card>`, etc.) — would need parent-design's auth model.
- Add TUI keybinding customization.
- Add mouse support (kanban is keyboard-only).
- Address M3.5/M4/M5 known-issues. Separate stabilization.

## Architecture

### Module structure (logical)

The current single-file TUI grows into a small module tree:

- **`tui/` (root)** — public `run()` entry; thin wrapper that creates the runtime and hands off.
- **`tui/app.rs`** — `App` struct (state below) + event loop. Owns all state; mutations go through here.
- **`tui/layout.rs`** — `LayoutMode` enum (`KanbanFull`, `FocusedColumnPlusDetail`) + render dispatch. The "easily changeable layout" hook lives here; adding a new layout = new enum variant + a render function.
- **`tui/render.rs`** — pure render functions for each panel (kanban grid, focused-column + detail, modals, status bar, session-error overlay, daemon-down banner). No state mutation; all input via `&App`.
- **`tui/events.rs`** — `KeyEvent` → `Action` translation. Pure mapping; documents the keybindings as code. Different `Action` sets depending on whether a modal is open and whether the session pane has focus.
- **`tui/actions.rs`** — `Action` execution: RPC calls, navigation updates, modal opens/closes, attach handoff to the session manager.
- **`tui/sessions.rs`** — `SessionManager`: PTY pool keyed by card_id. Spawns `docker exec ... tmux attach` per Implementing card; owns the `vt100::Parser` per session; exposes `screen_for(card_id)` for rendering and `send_keys(card_id, bytes)` for input. Closes sessions when cards leave Implementing.
- **`tui/client_ext.rs`** — typed RPC wrappers (`attach`, `edit`, `delete`) on top of existing `client::call`. Returns the protocol types directly for easier consumption by `actions.rs`.

### New RPCs

In the protocol crate:

- **`EditParams { id: String, brief: String }`** → **`EditResult { card: Card }`**. Method name `"edit"`.
- **`DeleteParams { id: String }`** → empty result (`null` Value). Method name `"delete"`.

Handler implementations:

- **`handle_edit`**: validate the card exists, update its brief and `updated_at` in sqlite, return the updated row. `NOT_FOUND` if absent. Future extensions (edit household, needs_heavy) are out of M6 scope but the param shape allows additive growth.
- **`handle_delete`**: fetch the card; if not found → `NOT_FOUND`. If in `Implementing`, call `state.effects.ensure_stopped(&card.id)` (same teardown that `mv → Done` triggers — container goes, listener stops, tmux dies with the container). Delete the card row. The card's host workspace (`~/reeve/cards/<id>/`) and any `refs/reeve/<id>/*` host-side refs **persist** per existing milestone semantics; M6 doesn't add reaper logic.

### Dependencies added

The reeve crate gains:

- **`portable-pty`** — cross-platform PTY support; spawn a child process with the slave end as stdin/stdout/stderr.
- **`vt100`** — pure-Rust VT100/xterm/ANSI emulator. Maintains a screen grid from a byte stream.
- **`tui-term`** — Ratatui widget for rendering a `vt100::Parser` screen. Drop-in.

All three are stable, used by wezterm and other production TUIs.

### Keybindings

**Always-active in kanban focus:**

| Key | Action |
|---|---|
| `q` | quit (graceful — closes all PTY sessions) |
| `r` | manual refresh (force an `ls` poll) |
| `?` | help overlay (key cheatsheet) |
| `h` / `←` | focus previous column |
| `l` / `→` | focus next column |
| `j` / `↓` | select card below within focused column |
| `k` / `↑` | select card above |
| `g` | jump to top of column |
| `G` | jump to bottom |
| `Enter` | open drill-in (`FocusedColumnPlusDetail` layout) |
| `Esc` | exit drill-in or close modal |
| `Tab` | shift focus to session pane (if `FocusedColumnPlusDetail` and the selected card has a live session) |
| `space` | open context-aware action menu |
| `H` / `Shift+←` | mv selected card one column left |
| `L` / `Shift+→` | mv selected card one column right |
| `m` | mv with column picker |
| `e` | edit selected card's brief |
| `n` | new card (Backlog) |
| `d` | delete selected card (confirm modal) |
| `a` | shorthand: `Enter` + `Tab` — open drill-in and focus the session pane |

**Session-pane focus:**

| Key | Action |
|---|---|
| `Shift+Tab` | return focus to kanban (only special-cased binding) |
| `Ctrl+\` | emergency: force focus back to kanban (fallback if Shift+Tab is captured) |
| Everything else | forwarded to the PTY (claude receives it inside the container's tmux) |

Inside the session, tmux's own bindings apply: `Ctrl+B [` for scrollback, `Ctrl+B D` for "detach" — but note that detach from the inner tmux just disconnects our PTY attach; the session inside the container stays alive. The TUI re-attaches on next refresh.

**Help overlay (`?`)** shows the full table; auto-generated from `events.rs` so it never drifts from the actual bindings.

### Layout modes

```rust
enum LayoutMode {
    /// Default: all 6 columns visible across the full width. No detail pane.
    KanbanFull,
    /// Drill-in: only the focused column visible (~35% left), detail pane on right (~65%).
    /// The detail pane shows card details OR the embedded session (live) when the
    /// selected card is in Implementing.
    FocusedColumnPlusDetail,
}
```

Adding new layouts later = new variant + render function in `layout.rs`. The event-loop doesn't care which is active; it dispatches Actions.

### App state

```rust
struct App {
    cards: Vec<Card>,                          // last `ls` snapshot, sorted by column + created_at
    focus: Focus,
    layout: LayoutMode,
    sessions: SessionManager,                  // PTY pool, vt100 parsers, drainer tasks
    status: Option<StatusMessage>,             // bottom-row text + level + expiry
    pending_attention: HashSet<CardId>,        // cards with unread output since last viewed
    households: Vec<HouseholdSummary>,         // cached for the `new` modal
    daemon_unreachable: bool,                  // true → full-screen banner instead of kanban
}

enum Focus {
    Kanban { column: Column, card_index: usize },
    SessionPane,
    Modal(ModalKind),
}

enum ModalKind {
    ActionMenu,
    EditBrief { card_id: String, buffer: String, cursor: usize },
    NewCard { household: Option<String>, brief: String, cursor: usize },
    DeleteConfirm { card_id: String },
    MoveColumn { card_id: String, choice: Column },
    Help,
}
```

Selection persists by card-id across `ls` refreshes — if the selected card was deleted from another frontend, focus falls back to the nearest neighbor by previous-index.

### Refresh strategy

Three independent tokio tasks:

1. **Card poll loop** — calls `ls` every 500 ms, pushes diffs to the main task via a channel. The main task updates `App.cards`.
2. **Session output drainer (per session)** — reads bytes from the PTY master, feeds `vt100::Parser`, signals the main task when there's new output for a non-focused session (sets `pending_attention`).
3. **Render loop** — driven by a dirty flag set by either of the above OR by user input. Renders at most every 16 ms (60 FPS cap). Idle TUI uses near-zero CPU.

### Mutations: optimistic with poll-reconcile

When the user presses `e` and saves a brief edit:

1. Send `edit` RPC.
2. **Success** — update `App.cards` immediately (don't wait for next poll); status bar shows `"saved"`.
3. **Failure** — `App.cards` unchanged; status bar shows the error in red.
4. The next `ls` poll reconciles regardless. If a concurrent CLI user mutated the same card, the daemon state wins.

`mv`, `delete` follow the same optimistic pattern. `new` is non-optimistic — the card appears on the next poll (typically ~500ms delay; acceptable).

### Embedded session pane

**Lifecycle:**

- On TUI startup, after the first `ls`: for each card in Implementing, call the `attach` RPC to get the container name + session name. Spawn a `portable_pty::CommandBuilder::new("docker").args(["exec", "-it", &container, "tmux", "attach", "-t", &session])` onto a fresh PTY. Initial size: derived from the right-pane Rect width/height.
- On `ls` refresh, diff implementing cards vs running sessions: spawn for new, close for removed.
- On TUI quit: close all PTYs; SIGTERM the child docker-execs; await reap.
- Closing a PTY does NOT close the in-container tmux session (we were attaching, not owning). Re-attaching later picks up where we left off.

**Rendering:**

The right pane (in `FocusedColumnPlusDetail` mode) renders either:

1. The selected card's `vt100::Parser` screen via `tui_term::PseudoTerminalWidget`, IF the card is in Implementing AND its session is alive.
2. The card's detail block (id / brief / household / column / created/updated_at / referenced repos) otherwise.
3. The session error overlay (Section 6 below) if a session errored.

The session is *live* — output continues to update even when the user is interacting with the kanban. Switching selection to another Implementing card swaps the rendered pane to that session instantly.

**Resize:**

On terminal resize, the App captures the new dimensions and calls `set_size` on each PTY. The vt100 parser handles the resize internally; tmux inside the container gets a SIGWINCH naturally and reflows.

**Activity badge:**

Each card's drainer task tracks "last output bytes count." The main task: when output arrives AND that card isn't currently the focused session, set `pending_attention.insert(card_id)`. The kanban render shows a `•` (cyan) next to cards in `pending_attention`. When the user makes a card the focused session (`Tab` into it OR moves selection to it under `FocusedColumnPlusDetail`), `pending_attention.remove(&card_id)`.

Activity badge persists in memory only; not in sqlite. New TUI session starts with all cards un-flagged.

### Mutations UX details

**Action menu (`space`).** Centered modal listing actions in fixed order: `e edit brief`, `m move to column…`, `H move left`, `L move right`, `a attach (open session)`, `d delete`. Disabled rows render dim with a reason: `"a attach (disabled: card not in Implementing)"`, `"H move left (already at Backlog)"`. Arrow navigation + Enter triggers the selected row; letter keys also trigger directly. `Esc` closes.

**Edit modal.** Single-line text input, pre-filled with current brief, cursor at end. Supports left/right/home/end/backspace/delete/typed chars. Enter saves (sends `edit` RPC, closes modal on success or shows error inline on fail). `Esc` cancels.

**New card modal.** If `households.len() > 1`: first step prompts for household (vertical picker, arrows + Enter); second step prompts for brief. If `households.len() == 1`: skip household step. If `households.len() == 0`: modal short-circuits with "no households configured. run `reeve household add` or `reeve household new` first."

**Delete confirm modal.** Two-line message: `"delete card '<short-id> <truncated-brief>'?"` + `"y/n  (or Enter/Esc)"`. Only `y` confirms; anything else cancels.

**Move column picker.** Vertical list of the 6 columns with the current column highlighted; arrows + Enter. Sends `mv` RPC. `Esc` cancels.

### Error & status display

**Status bar (bottom row).** Single line, transient. Color-coded by level (info white, warn yellow, error red). Auto-clears after 3 s of inactivity or on next keypress. Manual dismiss with `Esc` when no modal is open. Used for successful mutation feedback and non-fatal RPC errors.

**Modal error banner.** When a modal's submit fails (e.g. `edit` returns `NOT_FOUND`), show the error in red at the top of the modal. Modal stays open for retry or `Esc` out. Doesn't write to status bar.

**Session error overlay.** When a PTY exits non-zero (container died, tmux session not found, docker not running), the right pane renders an overlay instead of the vt100 grid:

```
┌─ session ended ─────────────────────────┐
│  docker exec exited with status N       │
│                                         │
│  Last 5 lines of output:                │
│    > tmux attach -t reeve               │
│    no server running                    │
│                                         │
│  'r' retry · 'x' dismiss                │
└─────────────────────────────────────────┘
```

`r` re-attempts session spawn for that card. `x` dismisses; pane goes blank until selection moves to another working card.

**Daemon-down banner (full-screen).** When the `ls` poll fails to connect to the socket, the entire kanban is replaced by a centered message: `"could not connect to <socket>. run 'reeve daemon start' from another terminal, then 'r' to retry. 'q' to quit."` Sessions and cards in memory stay there; rendering resumes on reconnect.

### Out of scope (do not implement in M6)

- Planning session infrastructure (host-side claude + tmux) — M7
- Planning → Implementing handoff workflow — M8
- Full split-pane drill-in with timeline / diff / chat tabs (parent design vision)
- Multi-line brief editing
- Edit fields beyond brief (household, needs_heavy, parent_id)
- Card filtering / sorting / search
- Custom keybindings
- Mouse support
- "Idle/blocked" heuristic detection beyond the activity badge
- Cross-card operations (diff, attach to non-self cards from inside a card's session — requires parent-design's auth model)
- TUI live-attach to a CLI process (`a` from CLI launching the TUI)
- M3.5/M4/M5 known-issues sweep
- Heavy-slot scheduler / shared services / service categories

## Testing

### Pure unit tests (most coverage)

**State transitions** in `app.rs` — given an `App` snapshot and an `Action`, assert the resulting state. No render, no IO.

- Selection clamps at column top/bottom (j past bottom = no wrap)
- After `delete`, selection falls back to nearest neighbor by index
- mv-right on a card in Done is a no-op
- After `ls` refresh, selection-by-card-id is preserved across reorders
- `pending_attention.insert` happens when drainer reports new bytes for non-focused card
- `pending_attention.remove` happens when that card becomes focused session
- Modal state machines: edit-brief buffer mutates correctly on each keypress; cursor stays in bounds
- Context-aware action menu: given a card in Backlog, "mv left" disabled; given a card in Implementing, "attach" enabled

**Keybinding → Action mapping** in `events.rs` — feed a `KeyEvent`, assert the produced `Action`. Documents bindings as code.

**Multi-household new-card flow.** With 0 / 1 / >1 households, assert modal opens to the correct step (short-circuits / brief / household-picker).

### Render snapshot tests

Ratatui's `TestBackend` renders to an in-memory grid; insta-style assertions catch layout regressions.

- "Kanban with 1 card per column renders all 6 column titles + counts"
- "FocusedColumnPlusDetail hides the other columns and shows the detail pane"
- "Status bar shows red text when level is Error"
- "Session error overlay renders correctly given a configured PTY failure"
- "Daemon-down banner renders when daemon_unreachable is true"

5-8 snapshot tests max — for layout regressions, not visual minutiae.

### Session pane / PTY integration

**vt100 round-trip:** feed `"hello\r\nworld"` to a `vt100::Parser`, render via `tui_term` into `TestBackend`, assert specific cells. Proves the rendering pipeline; trivial test.

**PTY lifecycle (no docker):** spawn `portable-pty` with a child running `echo done && exit 0`, drain bytes, assert exit code reaches the session manager. Validates the spawn-and-reap path without docker.

No docker-integration test for the embedded path — too fragile (timing, container build) and duplicates M5's e2e. Manual smoke covers this.

### Handler unit tests

`edit` and `delete` get the standard pattern from M3.5 / M4 / M5: happy path + 2-3 error modes each. ~6-8 new handler tests total.

### E2E (REEVE_TESTS_STRICT-aware)

**`m6_tui_lists_cards_via_test_backend`** — spawn daemon, create cards via CLI, drive the TUI programmatically with synthetic `KeyEvent`s against a `TestBackend`, assert the rendered grid. Validates the kanban renders real daemon data; no PTY involved. The only new e2e for M6.

Mutation paths from the TUI are covered by their handler unit tests + state-transition tests; building TUI e2e for each mutation is low-value ceremony.

### Manual smoke (M6 verification, with user)

1. `reeve daemon start`; create a household + a card via CLI; `reeve` launches the TUI; card visible.
2. `n` → new card; appears immediately.
3. `e` → edit brief; persists across `r` refresh.
4. `H` / `L` → mv left/right; column changes.
5. `m` → column picker → multi-column jump.
6. `d` → delete; card disappears after confirm.
7. Move a card to Implementing; embedded session pane shows the claude prompt.
8. `Tab` → focus session pane; type a message; claude responds in-pane.
9. `Shift+Tab` → focus kanban; switch selection to another Implementing card; session pane swaps; `pending_attention` dot on the prior card clears once focused.
10. Resize terminal; kanban and session pane reflow without artifacts.
11. Second `reeve` in another terminal; both see the same cards; mutate from one, watch the other update on poll.
12. `q` → quit; verify all docker-exec children are reaped (no leaks).

## Verification

A merged M6 must:

1. Pass all M1-M5 tests untouched.
2. Pass new unit, snapshot, PTY-lifecycle, handler, and e2e tests.
3. `cargo test --workspace` clean (with `REEVE_TESTS_STRICT=1`); `cargo fmt --check` clean; `cargo clippy --workspace -- -D warnings` clean.
4. Manual smoke (steps 1-12 above) all pass.

If the manual smoke surfaces an issue (vt100 rendering quirk, tmux nesting edge case, PTY resize lag), file a follow-up rather than block — flag in the PR description.

## Open items deferred to writing-plans

- Exact split between `app.rs`, `actions.rs`, and `events.rs`. The plan can adjust if one of them grows too thin or too fat in practice.
- Whether `SessionManager` lives in `tui/sessions.rs` or gets its own crate. Lean: inline in the tui module for now; can extract if the parent design's "multiple frontends" vision needs it shared.
- Whether `vt100::Parser` should be wrapped in a thin `SessionScreen` newtype to insulate consumers from the vt100 API surface, or whether direct use is fine. Lean: direct use; vt100 is stable and well-documented.
- Exact help-overlay rendering style (full table vs grouped sections).
- Whether the "danger key" (`Ctrl+\`) for forcing focus back to kanban needs more thought — could be `Ctrl+G` or similar. Pick at implementation; document in help.
- Scrollback ergonomics inside the session pane: do we expose a TUI-side scrollback key (e.g. `PageUp`), or rely entirely on tmux's `Ctrl+B [`? Lean: rely on tmux for M6; TUI-side scroll is M7+ polish.
