# Reeve M7 — Progress events for ensure_running + destructive-mv guard

**Status:** Design (2026-05-22)
**Predecessor:** M6 Interactive TUI (`2026-05-18-m6-tui-interactive-with-embedded-sessions-design.md`)
**Parent design:** `docs/superpowers/specs/2026-04-29-reeve-design.md`

## Context

After M6 the TUI is a daily driver: kanban with mutations, embedded session pane, daemon-down banner. Two UX gaps surfaced during smoke-testing M6:

1. **`ensure_running` is opaque.** Moving a card to Implementing kicks off a 30–150-second pipeline (clone meta-repo, clone siblings, `devcontainer up`, postCreate apt install + claude install, `tmux new-session`). The drill-in shows a generic spinner + elapsed counter (added in M6 commit `ac6515a`). The user can't tell what stage the daemon is in or what's left to do. Workaround today is `tail -f /tmp/reeved.log`, which contradicts the "TUI as daily driver" goal.

2. **Destructive mvs are too easy.** Pressing `H` on an Implementing card with an active claude session triggers `ensure_stopped` — container teardown, session loss. A single accidental keystroke costs the user minutes of provisioning plus whatever ephemeral state was in the session. The current debounce + per-card lock from M3-followup-1 prevent races, not accidents.

Both gaps share a theme: the lifecycle pipeline is expensive (in time, in lost state), and the TUI under-communicates around it.

## Summary

M7 adds two complementary improvements:

**Progress events.** A new `card_progress` RPC returns the stage-by-stage state of an in-flight `ensure_running` call for a given card. The daemon emits Active/Completed/Failed events as it transitions through each stage; the TUI polls the RPC at ~400 ms while the drill-in loader is visible and renders a stage checklist (✓ for done, spinner for active, dim for pending) inside the same panel the M6 generic loader occupies.

**Destructive-mv confirmation.** A modal prompt before any mv that would trigger `ensure_stopped`. The exact trigger matrix matches `reeve_card::transitions::on_column_change` — i.e., `Implementing | Testing | Review → Backlog | Planning`, or any column → `Done`. `y`/`Enter` confirms, `n`/`Esc` cancels. Wraps the existing mv-mutation path (`schedule_mv`) without changing the daemon side.

Both ship together because they touch the same code paths and are conceptually paired: "tell the user what expensive operations are doing" + "make expensive operations harder to start by accident."

## Out of scope (deferred)

- **Streaming postCreate output.** Surfacing the apt-install / claude-install stdout into the loader would be the next level of progress fidelity. Requires either a stream-subscribe RPC or accepting a stream of incremental updates. Defer until the coarse stage checklist proves insufficient.
- **Per-byte / percent progress.** No realistic source of accurate completion percentages (devcontainer CLI doesn't expose them). The stage checklist conveys "what's done / what's running" without a fake percent.
- **Progress UI in the kanban itself.** The existing yellow `⟳` badge on Implementing-bound cards (M6 commit `1b86284`) stays as-is. Per-stage detail is only shown in the drill-in.
- **Configurable confirmation behavior.** No `--yes-i-am-sure` flag or settings toggle. If users want to disable the confirmation later we'll revisit; for now it's always-on for destructive mvs.

---

## Architecture

Touched crates: `reeve-protocol`, `reeve-workspace`, `reeved`, `reeve`. No new crate. No new ports across crate boundaries beyond the one described below.

### Wire protocol (in `reeve-protocol`)

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CardProgressParams {
    pub id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CardProgressResult {
    pub card_id: String,
    pub stages: Vec<ProgressStage>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProgressStage {
    pub kind: ProgressKind,
    pub status: ProgressStatus,
    /// Unix-epoch seconds when the stage started (`Active` report fired).
    /// Never mutated after creation.
    pub started_at: i64,
    /// Unix-epoch seconds when the stage transitioned to Completed/Failed.
    /// `None` while Active. Duration = finished_at - started_at when set.
    pub finished_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgressKind {
    CloneMeta { name: String },
    CloneSibling { name: String },
    DevcontainerUp,
    StartingTmux,
    Done,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus { Active, Completed, Failed }
```

A `Vec<ProgressStage>` (not a single current stage) so the TUI can render the full timeline — completed checks above the active spinner. Order is append-order; UI displays top-to-bottom.

`Done` is a sentinel terminal stage with status `Completed`; never `Active`. Allows the TUI to draw "✓ Ready" as the final entry.

### Producer (in `reeve-workspace`)

A new port `ProgressReporter` defined alongside `ListenerOps` in `lifecycle.rs`:

```rust
#[async_trait]
pub trait ProgressReporter: Send + Sync {
    /// Active means "stage just started." Completed/Failed mutates the most
    /// recent stage whose kind matches.
    async fn report(
        &self,
        card_id: &str,
        kind: ProgressKind,
        status: ProgressStatus,
    );

    /// Drop all progress entries for this card. Called on ensure_stopped.
    async fn clear(&self, card_id: &str);
}
pub type ProgressHandle = Arc<dyn ProgressReporter>;
```

Wired through `LiveWorkspaceLifecycle::new(..., progress: ProgressHandle)` and stored as a field. Passed down to `clones::clone_all_for_card` as `&dyn ProgressReporter` along with `card_id`.

#### `run_stage` helper

Every fallible step in `ensure_running` (clone, devcontainer up, discover, upsert, tmux start) must be wrapped so an `Err` propagating via `?` ALWAYS emits a matching `Failed` event. Introduce a single helper that enforces this:

```rust
// reeve-workspace/src/lifecycle.rs (or a sibling progress.rs module)
async fn run_stage<T, F>(
    progress: &dyn ProgressReporter,
    card_id: &str,
    kind: ProgressKind,
    f: F,
) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    progress.report(card_id, kind.clone(), ProgressStatus::Active).await;
    match f.await {
        Ok(v) => {
            progress.report(card_id, kind, ProgressStatus::Completed).await;
            Ok(v)
        }
        Err(e) => {
            progress.report(card_id, kind, ProgressStatus::Failed).await;
            Err(e)
        }
    }
}
```

Every fallible step in the lifecycle MUST go through `run_stage`. Reviewers should flag any new `?` inside `ensure_running` that isn't inside a `run_stage` wrapper.

#### Stage grouping

A stage covers a semantic unit, not a single function call. `DevcontainerUp` groups three operations (spawn + discover + upsert) because they're a single conceptual step from the user's POV ("container is building"). Decomposing further would surface internal daemon bookkeeping (`upsert_container` is just a DB write) that the user doesn't care about.

#### Where reports fire

In `LiveWorkspaceLifecycle::ensure_running`:

```rust
clones::clone_all_for_card(
    &*self.git, &household, &manifest, &workspace_path,
    &*self.progress, card_id,
).await?;

let container_name = run_stage(
    &*self.progress, card_id, ProgressKind::DevcontainerUp,
    async {
        self.containers.spawn(argv).await?;
        let name = discover_with_deadline(&*self.containers, card_id).await?;
        self.workspaces
            .upsert_container(card_id, &name, ContainerStatus::Running, now_secs())
            .await?;
        Ok(name)
    },
).await?;

self.listeners.spawn(card_id, socket).await; // infallible; no stage

// tmux start is now FATAL on failure (see "Tmux failure semantics" below).
let tmux_cmd = format!(
    "export PATH=\"$HOME/.local/bin:$PATH\" && cd /workspaces/{card_id} && (claude || bash)"
);
run_stage(
    &*self.progress, card_id, ProgressKind::StartingTmux,
    self.containers.exec_in_tmux(&container_name, "reeve", &tmux_cmd),
).await?;

self.progress
    .report(card_id, ProgressKind::Done, ProgressStatus::Completed)
    .await;
Ok(())
```

In `clones::clone_all_for_card`, each per-entry clone gets its own stage:

```rust
// Step 1 — meta-repo. Inline entries don't apply here.
run_stage(progress, card_id,
    ProgressKind::CloneMeta { name: manifest.meta_repo.clone() },
    ensure_cloned(ops, meta, workspace, "workspace meta-repo"),
).await?;

// Step 2 — each sibling. ONLY entries with `source/.git/` get a stage;
// inline entries (no .git/ on the source side) don't produce progress
// events because there's no clone to track.
for entry in manifest.siblings() {
    let source = meta.join(&entry.name);
    if !source.join(".git").exists() {
        continue; // inline; verified-present check still happens later
    }
    let dest = workspace.join(&entry.name);
    run_stage(progress, card_id,
        ProgressKind::CloneSibling { name: entry.name.clone() },
        ensure_cloned(ops, &source, &dest, &format!("sibling '{}'", entry.name)),
    ).await?;
}

// Inline-entry presence check (existing behaviour, unchanged).
for entry in manifest.siblings() {
    let source = meta.join(&entry.name);
    if source.join(".git").exists() { continue; }
    let dest = workspace.join(&entry.name);
    if !dest.exists() {
        anyhow::bail!("inline entry '{}' missing from workspace clone", entry.name);
    }
}
```

#### Tmux failure semantics

M5/M6 lifecycle was warn-not-bail on `exec_in_tmux` failure: the column transition still succeeded; the user discovered the issue when they ran `reeve attach`. With M6's embedded session pane integral to the UX, that rationale is obsolete — a card in Implementing without a tmux session is broken from the TUI's POV. **M7 makes tmux-start failure fatal**: `run_stage` propagates the error via `?`, `handle_mv` returns Err, and (per M3-followup-5's reorder) the card stays in its previous column.

Consequence: the stage Vec ending in `StartingTmux(Failed)` is the terminal state for a tmux-start failure. No `Done` event fires. The TUI renders the Failed checklist; the user retries with `r` or by mv'ing in again.

#### Failure path (cleaned up)

Every `?` inside `ensure_running` flows through a `run_stage` call, which means an in-progress stage's `Failed` event is guaranteed to fire before the error propagates out. The caller (`handle_mv`) propagates the error; with the M3-followup-5 reorder the column stays in its old value. The `Failed` entry sits on the Vec until cleared by `ensure_stopped` or by a fresh `ensure_running` start (which resets the Vec on its first `Active` report).

### Storage (in `reeved`)

```rust
// reeved/src/app/state.rs (new struct or extension to DaemonState)
pub struct InMemoryProgress {
    state: Arc<Mutex<HashMap<String, Vec<ProgressStage>>>>,
}

#[async_trait]
impl ProgressReporter for InMemoryProgress { ... }

impl InMemoryProgress {
    pub async fn get(&self, card_id: &str) -> Vec<ProgressStage> {
        self.state.lock().await.get(card_id).cloned().unwrap_or_default()
    }
}
```

Held on `DaemonState` as `pub progress: Arc<InMemoryProgress>`. Construction wires it in `main.rs` and passes a clone to `LiveWorkspaceLifecycle::new` as the `ProgressHandle`.

Semantics:
- `report(card, kind, Active)` with the current state of `card_id` ending in `Done(Completed)` or `Failed`: reset the Vec to `[ProgressStage { kind, Active, now }]`.
- `report(card, kind, Active)` with no entry: insert `[ProgressStage { kind, Active, now }]`.
- `report(card, kind, Active)` with a non-terminal Vec: append `ProgressStage { kind, Active, now }`.
- `report(card, kind, Completed | Failed)`: find the most recent stage whose `kind == kind`, update its `status` and `at`. If no match (shouldn't happen), log a warn and append.
- `clear(card)`: remove the entry.

Why in-memory only: progress is intrinsically transient — a Vec keyed by the journey, not the destination. The destination (container row, card column) is persisted. Daemon restart loses in-flight progress, which is acceptable: any spawn the previous daemon process was running is dead with it; the TUI's `r` triggers retry, which fires fresh `Active` reports.

### Dispatch (in `reeved`)

```rust
// crates/reeved/src/dispatch.rs
"card_progress" => {
    let params = parse_params::<CardProgressParams>(req.params)?;
    let stages = state.progress.get(&params.id).await;
    serde_json::to_value(CardProgressResult { card_id: params.id, stages })
        .map_err(internal)
}
```

No per-card lock (read-only). No household lookup. Unknown card returns `stages: vec![]`, never errors — TUI relies on empty-vec semantics.

### Consumer (in `reeve`)

TUI client wrapper:

```rust
// reeve/src/tui/client_ext.rs
pub async fn rpc_card_progress(params: CardProgressParams)
    -> Result<Result<CardProgressResult, RpcError>>;
```

App state additions:

```rust
// reeve/src/tui/app.rs
pub struct App {
    // ... existing ...
    pub card_progress: HashMap<String, Vec<ProgressStage>>,
    pub card_progress_last_poll: HashMap<String, Instant>,
}
```

Drive-loop poll (after the existing 500ms `ls` refresh):

```rust
let now = Instant::now();
let pending_ids: Vec<String> = app.pending_mvs.keys().cloned().collect();
for card_id in pending_ids {
    let due = app.card_progress_last_poll.get(&card_id)
        .map(|t| now.duration_since(*t) >= Duration::from_millis(400))
        .unwrap_or(true);
    if due {
        if let Ok(Ok(r)) = client_ext::rpc_card_progress(
            CardProgressParams { id: card_id.clone() }
        ).await {
            app.card_progress.insert(card_id.clone(), r.stages);
            app.card_progress_last_poll.insert(card_id, now);
        }
    }
}
// On MvOk: once the session pane appears, the progress data isn't useful
// anymore. GC card_progress for cards that now have a live session.
app.card_progress.retain(|id, _| app.sessions.get(id).is_none());
app.card_progress_last_poll.retain(|id, _| app.sessions.get(id).is_none());
```

Polling is gated on `pending_mvs` (active spawn → fetch progress). GC is gated on `sessions` (live session → progress data is stale). The gap between the two is intentional: a **failed** spawn decrements `pending_mvs` (polling stops) but doesn't create a session — so its last known progress Vec stays in `app.card_progress` and remains renderable until either the user re-attempts the mv (which fresh-polls and overwrites) or the card transitions to a live session.

For an extra robustness step, on `OpEvent::MvErr` fire one final `rpc_card_progress` fetch synchronously before the handler returns, so the post-failure render shows the terminal `Failed` stage even if the last polling tick happened just before the failure landed.

Renderer replaces the body of `render_provisioning_loader`:

```rust
fn render_provisioning_loader(f, area, app, card_id) {
    let stages = app.card_progress.get(card_id).map(|v| v.as_slice()).unwrap_or(&[]);
    let elapsed = elapsed_str_from(&app.pending_mv_started_at, card_id);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Provisioning workspace…",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];
    for stage in stages {
        let label = format_stage(&stage.kind);
        let line = match stage.status {
            ProgressStatus::Completed => Span::styled(format!("  ✓ {label}"),
                Style::default().fg(Color::Green)),
            ProgressStatus::Active    => Span::styled(format!("  {} {label}", spinner_frame()),
                Style::default().fg(Color::Yellow)),
            ProgressStatus::Failed    => Span::styled(format!("  ✗ {label}"),
                Style::default().fg(Color::Red)),
        };
        lines.push(Line::from(line));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("  {elapsed}"),
        Style::default().fg(Color::DarkGray))));
    // Existing hint copy follows…
}

fn format_stage(kind: &ProgressKind) -> String {
    match kind {
        ProgressKind::CloneMeta { name }     => format!("Cloning meta-repo {name}"),
        ProgressKind::CloneSibling { name }  => format!("Cloning sibling {name}"),
        ProgressKind::DevcontainerUp         => "Building container".into(),
        ProgressKind::StartingTmux           => "Starting session".into(),
        ProgressKind::Done                   => "Ready".into(),
    }
}
```

When `stages` is empty (RPC hasn't returned yet — first 400 ms), the loader falls back to the M6 "Provisioning workspace…" + elapsed counter alone. Same animation framing as today.

#### Loader-visibility gate (changed from M6)

The drill-in's detail pane chooses between three render paths. M6 gated the loader on `card.column == Implementing && sessions.get(card_id).is_none() && pending_mvs.contains_key(card_id)`. M7 changes the third condition: gate on `card_progress.contains_key(card_id)` (i.e., we have progress data to show), and drop the column check entirely:

```rust
fn render_detail_pane(f, area, app) {
    let Some(card) = selected_card(app) else { return render_no_selection(f, area); };

    // 1. Live session takes priority — column-gated as today.
    if card.column == Column::Implementing {
        if let Some(session) = app.sessions.get(&card.id) {
            return render_session_pane(f, area, app, session);
        }
    }

    // 2. Progress data available → loader. NOT gated on card.column, so a
    //    failed spawn whose optimistic column has reverted to Backlog still
    //    shows the Failed checklist while the user is on the card.
    if app.card_progress.contains_key(&card.id) {
        return render_provisioning_loader(f, area, app, &card.id);
    }

    // 3. Static card detail.
    render_card_detail(f, area, app, card);
}
```

User dismisses a lingering Failed checklist by either:
- Closing the drill-in (`Esc`) — the loader simply isn't rendered.
- Re-attempting the mv — `OpEvent::MvOk` cleans up `card_progress` via the session-presence GC; `MvErr` overwrites with fresh stages.
- Moving the card out of Implementing → daemon's `ensure_stopped` calls `clear`, next TUI poll (next mv) would see empty stages.

No explicit "clear progress" keybinding for now — the auto-cleanup paths cover the common cases.

Sketch mid-spawn:

```
  Provisioning workspace…

  ✓ Cloning meta-repo reeve-self
  ✓ Cloning sibling reeve
  ⠋ Building container

  42s elapsed

  First-time provisioning of a card typically takes…
```

### Destructive-mv guard (in `reeve` TUI only)

New `ModalKind` variant:

```rust
// reeve/src/tui/modal.rs
ConfirmDestructiveMv {
    card_id: String,
    from: Column,
    to: Column,
},
```

#### Trigger

In `actions::schedule_mv` (the entry point for `H` / `L` / `Shift+arrow` / column-picker `Enter`), after looking up the source column:

```rust
fn schedule_mv(app: &mut App, card_id: String, target: Column) {
    let from = app.cards.iter()
        .find(|c| c.id == card_id)
        .map(|c| c.column);
    if let Some(from) = from {
        if is_destructive(from, target) {
            app.modal = Some(ModalKind::ConfirmDestructiveMv {
                card_id,
                from,
                to: target,
            });
            return;
        }
    }
    schedule_mv_unchecked(app, card_id, target);
}

fn is_destructive(from: Column, to: Column) -> bool {
    use Column::*;
    matches!((from, to),
        (Implementing | Testing | Review, Backlog | Planning) |
        (Implementing | Testing | Review, Done) |
        (Backlog | Planning, Done)
    )
}
```

`schedule_mv_unchecked` becomes the existing body of `schedule_mv` (optimistic update + debounced RPC).

The trigger matrix matches the daemon's `on_column_change` arms that route to `ensure_stopped`. False positive is impossible; false negative only if a future column gains lifecycle side-effects without updating `is_destructive` — flag in a comment to keep the two in sync.

#### Modal flow

```rust
// in handle_modal_key for ConfirmDestructiveMv
KeyCode::Char('y') | KeyCode::Enter => {
    app.modal = None;
    schedule_mv_unchecked(app, card_id, to);
}
KeyCode::Char('n') | KeyCode::Esc => {
    app.modal = None;
}
```

#### Render

A small centered popup (≈40% wide × 25% tall) titled "Tear down workspace?" with body text that names `from` → `to` explicitly. Borders red to match the destructive-action visual language (matches the existing `render_delete_modal` red border). Yes/no hint on the bottom line.

Example body:

```
  Move from Implementing to Backlog?

  This will tear down the container and end the
  claude session for this card.

  Continue?     [y]es     [n]o
```

#### Edge cases

- **mv from non-destructive source** (e.g., Backlog → Planning): `is_destructive` returns false, no modal, immediate proceed. Identical to today.
- **mv-self from inside container**: in-container claude calls `mv-self` via the per-card socket. That path is in `reeve-card::handle_mv_self`, not in the TUI. **No confirmation** — the in-container agent can self-promote without prompting the host user. This is intentional: the agent should be able to push itself to Testing without UI confirmation. If the user wants to override this, they can mv back from the host TUI.
- **Modal already open** when destructive mv is triggered: shouldn't happen (modal handling and mv triggers are in different code paths, and `handle_modal_key` consumes input first). But defensively, opening a new modal replaces any existing one — same semantics as the M6 modal stack.

---

## Tests

### Unit

| Crate | Test | Asserts |
|---|---|---|
| reeved | `InMemoryProgress::report_active_appends_stage` | new entry; status=Active; at=now |
| reeved | `InMemoryProgress::report_completed_mutates_last_matching` | most recent CloneSibling{x}, not earlier kinds |
| reeved | `InMemoryProgress::report_active_after_terminal_resets` | Vec ending in Done(Completed) or X(Failed) is replaced |
| reeved | `InMemoryProgress::clear_removes_entry` | get returns vec![] after clear |
| reeved | `InMemoryProgress::get_unknown_returns_empty` | no panic, returns vec![] |
| reeve-workspace | `clones::clone_all_for_card_emits_active_completed_per_repo` | mock reporter records Active+Completed for meta + each **separate-sibling** (source/.git/ exists) in declaration order |
| reeve-workspace | `clones::clone_all_for_card_skips_inline_entries_in_progress` | manifest with one inline (no source/.git/) and one separate sibling: reporter sees CloneMeta + CloneSibling for the separate one, **no** stage for the inline |
| reeve-workspace | `clones::clone_all_for_card_emits_failed_on_clone_error` | sibling clone fails → Active then Failed for that sibling, no further reports |
| reeve-workspace | `lifecycle::ensure_running_happy_path_emits_full_stage_sequence` | full Vec: clones, DevcontainerUp(Active+Completed), StartingTmux(Active+Completed), Done(Completed) |
| reeve-workspace | `lifecycle::ensure_running_devcontainer_up_failure_reports_failed` | DevcontainerUp(Failed), no later stages, returns Err |
| reeve | `actions::schedule_mv_destructive_opens_confirm_modal` | Implementing → Backlog opens ConfirmDestructiveMv with correct from/to |
| reeve | `actions::schedule_mv_non_destructive_proceeds` | Backlog → Planning fires the RPC, no modal |
| reeve | `actions::is_destructive_matrix_matches_lifecycle_routing` | matches the source-of-truth table in the file's comment |
| reeve | `modal::confirm_destructive_y_calls_unchecked_mv` | y/Enter closes modal + fires schedule_mv_unchecked |
| reeve | `modal::confirm_destructive_n_cancels` | n/Esc closes modal + no RPC fired |

### E2E

Extend `m6_daemon_round_trip_for_tui_smoke` (or new sibling): the reeve CLI doesn't expose a raw-RPC subcommand, so the test invokes the daemon's `card_progress` directly via `reeve::client::call` against the same UDS the daemon binds. Assert the response shape — for a card whose mv hasn't started, stages is `vec![]`; for an unknown id, also `vec![]` (no NOT_FOUND). Without docker we can't exercise the lifecycle producing stages, so this verifies routing + empty-vec semantics only.

```rust
// Sketch:
let socket = temp.path().join("reeved.sock");
let value = reeve::client::call(&socket, "card_progress",
    serde_json::json!({"id": card_id})).await.unwrap().unwrap();
let result: CardProgressResult = serde_json::from_value(value).unwrap();
assert!(result.stages.is_empty());
```

Render-snapshot tests for the loader and modal are deferred to a follow-up render-test infrastructure task (no existing pattern in the repo to extend).

### Manual smoke

After implementation, smoke list:

1. mv a fresh card to Implementing. Drill in (`Enter`). Loader appears within 500 ms; first "✓ Cloning meta-repo …" appears within ~5 s; "Building container" within ~10 s; "Starting session" within ~90 s; loader collapses to session pane within ~95 s total.
2. Use a card whose meta-repo is missing (intentionally bad household path). Confirm `Failed` is rendered red against the failing stage, loader does not collapse, status bar carries the error.
3. mv a card in Implementing to Backlog with `H`. Confirm modal opens; press `n`. Card stays in Implementing, session intact.
4. Repeat (3) with `y`. Card moves to Backlog, container tears down.
5. mv a card in Backlog to Planning with `L`. **No modal**. Card moves immediately.
6. mv a card in Backlog to Done via `m` picker. Confirm modal opens (Done is destructive from any source).

---

## Risks / open notes

1. **Lock contention on `InMemoryProgress`.** Single global `Mutex<HashMap>`. Fine for ≤ 10 concurrent provisions. Switch to `DashMap` if parallelism scales.
2. **`Done` lingers indefinitely in the daemon's HashMap** unless cleared. Acceptable: the daemon-side entry is reset by the next `ensure_running` for the same card or wiped on `ensure_stopped`. TUI-side GC is now gated on `sessions.get(id).is_some()` (i.e., the spawn succeeded and the user is in the session pane) so the data hangs around for failure-visibility but disappears once the card transitions to its live session. Memory cost on the daemon: tens of bytes per Implementing card ever provisioned in this daemon's lifetime. If a user runs the daemon for weeks across thousands of provisions, prune by removing entries whose only stage is `Done(Completed)` older than 1 hour.
3. **In-flight progress lost on daemon restart.** A spawn the previous daemon was running is dead with it (the subprocess dies). The user retries via `r`; new daemon starts fresh. Surfaced as a slight UI flicker — loader briefly empty, then re-fills as the new spawn reports.
4. **Reporter `report` holds the inner `Mutex` briefly inside an async fn.** No await held across the lock — `lock → mutate → drop` is synchronous within the fn body, then the fn awaits nothing further. Safe.
5. **`is_destructive` matrix can drift from `on_column_change` matrix.** They're in different crates (`reeve` vs `reeve-card`). Two-line comment in `actions.rs` next to `is_destructive` cross-referencing the source-of-truth, and a unit test (`is_destructive_matrix_matches_lifecycle_routing`) that asserts they agree. Refactoring `on_column_change` to expose an `is_side_effecting()` query would eliminate the duplication; deferred — the matrix is stable.
6. **The 400 ms progress poll is in addition to the 500 ms ls poll.** Two RPCs per pending card during the spawn window. Both are sub-ms over UDS, both are short responses. No observable impact on daemon load. If we later scale to dozens of concurrent provisions, batch via a `card_progress_many` variant.
7. **Tmux-fatal behavior change.** M5/M6 was warn-not-bail; M7 makes it fatal. Side-effect: any household template that ships without tmux installed in the container now FAILS the mv to Implementing rather than silently producing a card with no session. Mitigation: the `Failed` stage in the loader makes the diagnosis immediate ("✗ Starting session"). Affected templates: the witan-household template was updated in `7eff816` (during M6 smoke) to install tmux, so the bundled template is fine. Third-party templates without tmux will trip.
8. **Failure visibility persists beyond pending_mvs.** With the visibility gate changed to "card_progress has data + no session", a Failed checklist stays visible in the drill-in until the user re-attempts the mv or the card transitions to a session. This is intentional. If a user wants to forget a failure exists, they can close the drill-in (the checklist is only rendered in the drill-in's detail pane, never in the kanban-full layout).

---

## Self-review

- **Spec coverage of stated goals:**
  - "Tell user what stage daemon is in" → `card_progress` RPC + stage checklist render ✓
  - "Don't lose work to accidental destructive mvs" → `ConfirmDestructiveMv` modal ✓
- **Placeholders:** none. Every interface is named, every flow traced.
- **Internal consistency:** `is_destructive` matrix matches `on_column_change` for "Done from anywhere" + "(Implementing|Testing|Review) → (Backlog|Planning)" — yes. The matrix appears in two places (spec + planned source) but the spec self-flags this as a known duplication with a test guarding drift. Summary text and detail sections agree on the matrix.
- **Scope creep check:** kept tight to the two needs. Stream output, render snapshots, configurable confirmation all explicitly deferred.
- **Architecture fit:** `ProgressReporter` follows the existing `ListenerOps` pattern (port in workspace, impl in reeved, wired in main.rs). Storage stays in `reeved`'s memory — no DB schema change, no cross-context FKs. Card aggregate untouched.

### Changes from the first draft (pre-user-review)

- **`ProgressStage` timestamps**: dropped overwrite-on-completion semantics; now `started_at: i64` + `finished_at: Option<i64>` so consumers can compute per-stage duration without losing start time.
- **Failure reporting**: introduced a single `run_stage` helper that wraps EVERY fallible step (clones, devcontainer up, discover, upsert, tmux start). Reviewer flagged that the old draft only wrapped clones + spawn + tmux, leaving discover/upsert failures without a `Failed` event.
- **Tmux failure semantics**: now fatal (`?`-propagated). The old draft emitted `StartingTmux(Failed)` followed by `Done(Completed)`, which was a contradictory signal. The M5 warn-not-bail rationale ("user discovers via reeve attach") is obsolete under M6's session pane.
- **UI failure visibility**: the loader-render gate no longer requires `card.column == Implementing` or `pending_mvs`. Now: "card_progress has entries + no session." A failed spawn whose optimistic column reverts to Backlog still shows the Failed checklist in the drill-in until the user re-attempts or moves on. TUI-side GC moved from `pending_mvs.contains_key` to `sessions.get(id).is_some()`.
- **Inline entries**: explicitly call out that they don't produce progress stages (the clone step doesn't apply); only separate-sibling repos with `source/.git/` do.
- **E2E test**: uses `reeve::client::call` directly (no raw-RPC CLI exists), with a sketch.
