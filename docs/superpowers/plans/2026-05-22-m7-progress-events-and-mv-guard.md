# M7 — Progress events + destructive-mv guard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `card_progress` RPC + stage-checklist drill-in loader so the user sees what stage `ensure_running` is in, and add a `ConfirmDestructiveMv` modal so accidental destructive mvs don't tear down containers silently.

**Architecture:** Daemon emits stage events through a new `ProgressReporter` port (lives in `reeve-workspace` alongside `ListenerOps`; impl in `reeved`). A `run_stage` helper guarantees every `?` propagation through `ensure_running` emits a matching `Failed` event. TUI polls `card_progress` while a mv is pending, persists data until the card has a session, and renders a `✓`/spinner/`✗` checklist instead of the M6 generic loader. Destructive-mv guard wraps `schedule_mv` with an `is_destructive` check that opens a confirmation modal; in-container `mv-self` is unaffected.

**Tech Stack:** Rust 1.95, tokio 1, anyhow, serde_json, ratatui 0.29, crossterm 0.28. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-22-m7-progress-events-and-mv-guard-design.md`

**Branch:** `feat/m7-progress-events` (already checked out; spec was committed in `29478a8`, revisions in `e42ebdb`).

---

## File structure

### New files

| Path | Responsibility |
|---|---|
| `crates/reeve-workspace/src/progress.rs` | `ProgressReporter` trait + `run_stage` helper. Re-exported from `lib.rs`. |
| `crates/reeved/src/app/progress.rs` | `InMemoryProgress` impl of `ProgressReporter`. Held on `DaemonState`. |

### Modified files

| Path | Change |
|---|---|
| `crates/reeve-protocol/src/lib.rs` | Add `CardProgressParams`, `CardProgressResult`, `ProgressStage`, `ProgressKind`, `ProgressStatus`. |
| `crates/reeve-workspace/src/lib.rs` | Re-export `ProgressReporter`, `ProgressHandle`, `run_stage`. |
| `crates/reeve-workspace/src/lifecycle.rs` | Add `progress: ProgressHandle` field to `LiveWorkspaceLifecycle`; thread through `ensure_running` via `run_stage`. Tmux failure now fatal. |
| `crates/reeve-workspace/src/clones.rs` | `clone_all_for_card` takes `&dyn ProgressReporter, card_id`; wrap each clone in `run_stage`; inline entries skipped (no stage). |
| `crates/reeved/src/app/state.rs` | `DaemonState` gains `progress: Arc<InMemoryProgress>`. |
| `crates/reeved/src/main.rs` | Construct `InMemoryProgress`, pass clone to `LiveWorkspaceLifecycle::new`, store on `DaemonState`. |
| `crates/reeved/src/dispatch.rs` | Add `"card_progress"` arm. |
| `crates/reeve/src/tui/client_ext.rs` | Add `rpc_card_progress`. |
| `crates/reeve/src/tui/app.rs` | Add `card_progress` + `card_progress_last_poll` fields. |
| `crates/reeve/src/tui/actions.rs` | Poll progress in `refresh`; on `MvErr` do one extra fetch; add `is_destructive` + reroute `schedule_mv`. |
| `crates/reeve/src/tui/modal.rs` | Add `ConfirmDestructiveMv` variant. |
| `crates/reeve/src/tui/render.rs` | Rewrite `render_provisioning_loader`; change loader-visibility gate; render the new modal. |
| `tests/e2e.rs` | New test for `card_progress` round-trip via `reeve::client::call`. |

---

## Task 1: Wire protocol types

**Files:**
- Modify: `crates/reeve-protocol/src/lib.rs`

- [ ] **Step 1: Add the types**

Find the location near other RPC type definitions (after `MvParams` / `MvResult` etc., before any test module). Add:

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct CardProgressParams {
    pub id: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct CardProgressResult {
    pub card_id: String,
    pub stages: Vec<ProgressStage>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ProgressStage {
    pub kind: ProgressKind,
    pub status: ProgressStatus,
    /// Unix-epoch seconds when the stage started. Set on the Active report
    /// and never mutated.
    pub started_at: i64,
    /// Unix-epoch seconds when the stage transitioned to Completed/Failed.
    /// `None` while Active. Duration = finished_at - started_at when set.
    pub finished_at: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgressKind {
    CloneMeta { name: String },
    CloneSibling { name: String },
    DevcontainerUp,
    StartingTmux,
    Done,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus {
    Active,
    Completed,
    Failed,
}
```

- [ ] **Step 2: Add a round-trip serde test**

In the `tests` module at the end of `crates/reeve-protocol/src/lib.rs`:

```rust
#[test]
fn progress_kind_serializes_with_tag_kind() {
    let k = ProgressKind::CloneSibling {
        name: "reeve".into(),
    };
    let v = serde_json::to_value(&k).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"kind": "clone_sibling", "name": "reeve"}),
        "snake_case + #[serde(tag = \"kind\")] discriminator",
    );
}

#[test]
fn progress_stage_round_trips() {
    let stage = ProgressStage {
        kind: ProgressKind::DevcontainerUp,
        status: ProgressStatus::Active,
        started_at: 1_700_000_000,
        finished_at: None,
    };
    let s = serde_json::to_string(&stage).unwrap();
    let back: ProgressStage = serde_json::from_str(&s).unwrap();
    assert_eq!(back.status, ProgressStatus::Active);
    assert_eq!(back.finished_at, None);
}

#[test]
fn card_progress_result_with_no_stages_round_trips() {
    let r = CardProgressResult {
        card_id: "abc".into(),
        stages: vec![],
    };
    let s = serde_json::to_string(&r).unwrap();
    let back: CardProgressResult = serde_json::from_str(&s).unwrap();
    assert_eq!(back.card_id, "abc");
    assert!(back.stages.is_empty());
}
```

- [ ] **Step 3: Run tests**

```
cargo test -p reeve-protocol
```

Expected: all pass.

- [ ] **Step 4: Commit**

```
git add crates/reeve-protocol/src/lib.rs
git commit -m "feat(m7): protocol types — card_progress RPC + ProgressStage"
```

---

## Task 2: ProgressReporter port + run_stage helper

**Files:**
- Create: `crates/reeve-workspace/src/progress.rs`
- Modify: `crates/reeve-workspace/src/lib.rs`

- [ ] **Step 1: Create the new module**

Write `crates/reeve-workspace/src/progress.rs`:

```rust
//! Progress reporting for long-running workspace lifecycle steps.
//!
//! `ProgressReporter` is the port; `run_stage` is a helper that guarantees
//! every fallible step inside `ensure_running` emits a matching `Failed`
//! event when an `Err` propagates. Any `?` inside the lifecycle that
//! isn't wrapped in `run_stage` is a spec violation — review for it.

use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{ProgressKind, ProgressStatus};
use std::sync::Arc;

#[async_trait]
pub trait ProgressReporter: Send + Sync {
    /// Active = stage just started; Completed/Failed = update the most-recent
    /// matching stage's status. See InMemoryProgress for the exact mutation
    /// rules.
    async fn report(&self, card_id: &str, kind: ProgressKind, status: ProgressStatus);

    /// Drop all progress entries for this card. Called on ensure_stopped.
    async fn clear(&self, card_id: &str);
}

pub type ProgressHandle = Arc<dyn ProgressReporter>;

/// Wrap a fallible operation in matching Active / Completed-or-Failed reports.
/// On `Err`, fires `Failed` BEFORE the error propagates via `?`.
pub async fn run_stage<T, F>(
    progress: &dyn ProgressReporter,
    card_id: &str,
    kind: ProgressKind,
    f: F,
) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    progress
        .report(card_id, kind.clone(), ProgressStatus::Active)
        .await;
    match f.await {
        Ok(v) => {
            progress
                .report(card_id, kind, ProgressStatus::Completed)
                .await;
            Ok(v)
        }
        Err(e) => {
            progress
                .report(card_id, kind, ProgressStatus::Failed)
                .await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingReporter {
        calls: Mutex<Vec<(String, ProgressKind, ProgressStatus)>>,
        cleared: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ProgressReporter for RecordingReporter {
        async fn report(&self, card_id: &str, kind: ProgressKind, status: ProgressStatus) {
            self.calls
                .lock()
                .unwrap()
                .push((card_id.to_string(), kind, status));
        }
        async fn clear(&self, card_id: &str) {
            self.cleared.lock().unwrap().push(card_id.to_string());
        }
    }

    #[tokio::test]
    async fn run_stage_emits_active_then_completed_on_ok() {
        let r = RecordingReporter::default();
        let out: i32 = run_stage(&r, "c1", ProgressKind::DevcontainerUp, async { Ok(42) })
            .await
            .unwrap();
        assert_eq!(out, 42);
        let calls = r.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![
                ("c1".into(), ProgressKind::DevcontainerUp, ProgressStatus::Active),
                ("c1".into(), ProgressKind::DevcontainerUp, ProgressStatus::Completed),
            ],
        );
    }

    #[tokio::test]
    async fn run_stage_emits_active_then_failed_on_err() {
        let r = RecordingReporter::default();
        let res: Result<i32> = run_stage(&r, "c1", ProgressKind::DevcontainerUp, async {
            anyhow::bail!("boom")
        })
        .await;
        assert!(res.is_err());
        let calls = r.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![
                ("c1".into(), ProgressKind::DevcontainerUp, ProgressStatus::Active),
                ("c1".into(), ProgressKind::DevcontainerUp, ProgressStatus::Failed),
            ],
        );
    }
}
```

- [ ] **Step 2: Wire the module into the crate**

Edit `crates/reeve-workspace/src/lib.rs`. Add the module + re-exports:

```rust
pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;
pub mod handler;
pub mod lifecycle;
pub mod progress;       // <-- add
pub mod repo;

pub use clones::clone_all_for_card;
pub use container::{ContainerOps, ContainerOpsHandle, make_container_ops};
pub use git::{GitOps, GitOpsHandle, make_git_ops};
pub use lifecycle::{ListenerHandle, ListenerOps, LiveWorkspaceLifecycle};
pub use progress::{ProgressHandle, ProgressReporter, run_stage};  // <-- add
pub use repo::{ContainerRow, WorkspaceRepo, make_workspace_repo};
pub use repo::init_schema;
```

(Exact existing pub uses may differ; preserve them and add the two new lines.)

- [ ] **Step 3: Run tests**

```
cargo test -p reeve-workspace --lib progress::
```

Expected: 2 passed.

- [ ] **Step 4: Commit**

```
git add crates/reeve-workspace/src/progress.rs crates/reeve-workspace/src/lib.rs
git commit -m "feat(m7): ProgressReporter port + run_stage helper"
```

---

## Task 3: InMemoryProgress storage

**Files:**
- Create: `crates/reeved/src/app/progress.rs`
- Modify: `crates/reeved/src/app/mod.rs`

- [ ] **Step 1: Confirm `app/mod.rs` exists and inspect it**

```
ls crates/reeved/src/app/
cat crates/reeved/src/app/mod.rs
```

If `mod.rs` doesn't already declare `state`, this task will need to add `pub mod state;` first. The plan assumes the existing module declarations stay; we only add `pub mod progress;`.

- [ ] **Step 2: Write `crates/reeved/src/app/progress.rs`**

```rust
//! In-memory `ProgressReporter` impl. Stores per-card stage Vecs behind
//! a Mutex. Daemon restart wipes this; it's intentionally transient
//! (progress is the journey, not the destination).

use async_trait::async_trait;
use reeve_protocol::{ProgressKind, ProgressStage, ProgressStatus};
use reeve_workspace::ProgressReporter;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

pub struct InMemoryProgress {
    state: Arc<Mutex<HashMap<String, Vec<ProgressStage>>>>,
}

impl InMemoryProgress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Snapshot the stages for a card. Empty vec for unknown ids; never errors.
    pub async fn get(&self, card_id: &str) -> Vec<ProgressStage> {
        self.state
            .lock()
            .await
            .get(card_id)
            .cloned()
            .unwrap_or_default()
    }
}

impl Default for InMemoryProgress {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn is_terminal(s: ProgressStatus) -> bool {
    matches!(s, ProgressStatus::Completed | ProgressStatus::Failed)
}

#[async_trait]
impl ProgressReporter for InMemoryProgress {
    async fn report(&self, card_id: &str, kind: ProgressKind, status: ProgressStatus) {
        let mut guard = self.state.lock().await;
        let entry = guard.entry(card_id.to_string()).or_default();

        match status {
            ProgressStatus::Active => {
                // If the most recent stage was terminal (Done/Completed or
                // Failed), this is a fresh ensure_running attempt — reset.
                let prior_terminal = entry
                    .last()
                    .map(|s| is_terminal(s.status))
                    .unwrap_or(false);
                if prior_terminal {
                    entry.clear();
                }
                entry.push(ProgressStage {
                    kind,
                    status: ProgressStatus::Active,
                    started_at: now_secs(),
                    finished_at: None,
                });
            }
            ProgressStatus::Completed | ProgressStatus::Failed => {
                // Mutate the most recent stage whose kind matches. If we
                // can't find one (shouldn't happen — run_stage always
                // emits Active first), log a warn and append a synthetic.
                if let Some(stage) = entry
                    .iter_mut()
                    .rev()
                    .find(|s| s.kind == kind && s.status == ProgressStatus::Active)
                {
                    stage.status = status;
                    stage.finished_at = Some(now_secs());
                } else {
                    tracing::warn!(
                        "progress.report: no Active stage matching {:?} for card {}; appending",
                        kind,
                        card_id,
                    );
                    let t = now_secs();
                    entry.push(ProgressStage {
                        kind,
                        status,
                        started_at: t,
                        finished_at: Some(t),
                    });
                }
            }
        }
    }

    async fn clear(&self, card_id: &str) {
        self.state.lock().await.remove(card_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_unknown_card_returns_empty() {
        let p = InMemoryProgress::default();
        assert!(p.get("nope").await.is_empty());
    }

    #[tokio::test]
    async fn report_active_appends_stage() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active).await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ProgressKind::DevcontainerUp);
        assert_eq!(v[0].status, ProgressStatus::Active);
        assert!(v[0].finished_at.is_none());
    }

    #[tokio::test]
    async fn report_completed_mutates_most_recent_active() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active).await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Completed).await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].status, ProgressStatus::Completed);
        assert!(v[0].finished_at.is_some());
    }

    #[tokio::test]
    async fn report_completed_matches_kind_not_just_latest() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::CloneSibling { name: "a".into() }, ProgressStatus::Active).await;
        p.report("c", ProgressKind::CloneSibling { name: "a".into() }, ProgressStatus::Completed).await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active).await;
        // Completing the 'a' sibling AFTER DevcontainerUp's Active should
        // never apply — but since 'a' is already Completed there's no
        // Active 'a' to mutate. We log and append a synthetic.
        p.report("c", ProgressKind::CloneSibling { name: "a".into() }, ProgressStatus::Completed).await;
        let v = p.get("c").await;
        // 'a' completed, devcontainer_up active, synthetic 'a' completed appended.
        assert_eq!(v.len(), 3);
        assert_eq!(v[1].status, ProgressStatus::Active); // devcontainer_up still active
    }

    #[tokio::test]
    async fn report_active_after_terminal_resets_vec() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active).await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Completed).await;
        p.report("c", ProgressKind::Done, ProgressStatus::Completed).await;
        // Fresh ensure_running:
        p.report("c", ProgressKind::CloneMeta { name: "x".into() }, ProgressStatus::Active).await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ProgressKind::CloneMeta { name: "x".into() });
    }

    #[tokio::test]
    async fn report_active_after_failed_resets_vec() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active).await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Failed).await;
        p.report("c", ProgressKind::CloneMeta { name: "x".into() }, ProgressStatus::Active).await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
    }

    #[tokio::test]
    async fn clear_removes_entry() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active).await;
        p.clear("c").await;
        assert!(p.get("c").await.is_empty());
    }
}
```

- [ ] **Step 3: Add module declaration**

In `crates/reeved/src/app/mod.rs`, alongside existing `pub mod` lines, add:

```rust
pub mod progress;
```

- [ ] **Step 4: Run tests**

```
cargo test -p reeved --lib app::progress::
```

Expected: 6 passed.

- [ ] **Step 5: Commit**

```
git add crates/reeved/src/app/progress.rs crates/reeved/src/app/mod.rs
git commit -m "feat(m7): InMemoryProgress storage with append/mutate semantics"
```

---

## Task 4: Wire progress through LiveWorkspaceLifecycle (signature + DaemonState)

**Files:**
- Modify: `crates/reeve-workspace/src/lifecycle.rs`
- Modify: `crates/reeved/src/app/state.rs`
- Modify: `crates/reeved/src/main.rs`

Lifecycle gains a `progress` field but doesn't yet USE it; that's the next task. Splitting keeps the diff small and reviewable.

- [ ] **Step 1: Add field + constructor arg to `LiveWorkspaceLifecycle`**

Edit `crates/reeve-workspace/src/lifecycle.rs`. Update the struct + `new`:

```rust
pub struct LiveWorkspaceLifecycle {
    workspaces: Arc<dyn WorkspaceRepo>,
    households: Arc<dyn reeve_household::HouseholdRepo>,
    git: Arc<dyn GitOps>,
    containers: Arc<dyn ContainerOps>,
    listeners: ListenerHandle,
    progress: crate::ProgressHandle,
    claude_dir: PathBuf,
    api_key: Option<String>,
}

impl LiveWorkspaceLifecycle {
    pub fn new(
        workspaces: Arc<dyn WorkspaceRepo>,
        households: Arc<dyn reeve_household::HouseholdRepo>,
        git: Arc<dyn GitOps>,
        containers: Arc<dyn ContainerOps>,
        listeners: ListenerHandle,
        progress: crate::ProgressHandle,
        claude_dir: PathBuf,
        api_key: Option<String>,
    ) -> Self {
        Self {
            workspaces,
            households,
            git,
            containers,
            listeners,
            progress,
            claude_dir,
            api_key,
        }
    }
}
```

- [ ] **Step 2: Add `progress` to `DaemonState`**

Edit `crates/reeved/src/app/state.rs`:

```rust
#[derive(Clone)]
pub struct DaemonState {
    pub db: DbHandle,
    pub cards: Arc<dyn reeve_card::CardRepo>,
    pub workspaces: Arc<dyn reeve_workspace::WorkspaceRepo>,
    pub containers: ContainerOpsHandle,
    pub git: GitOpsHandle,
    pub listeners: ListenerRegistry,
    pub household_repo: Arc<dyn reeve_household::HouseholdRepo>,
    pub household_lookup: Arc<dyn reeve_card::HouseholdLookup>,
    pub workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle>,
    pub progress: Arc<crate::app::progress::InMemoryProgress>,   // <-- add
    pub card_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}
```

- [ ] **Step 3: Construct in `main.rs`**

Edit `crates/reeved/src/main.rs`. Find the block that constructs `LiveWorkspaceLifecycle` and `DaemonState`. Update:

```rust
let progress = app::progress::InMemoryProgress::new();
let progress_handle: reeve_workspace::ProgressHandle = progress.clone();

let workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle> =
    Arc::new(reeve_workspace::LiveWorkspaceLifecycle::new(
        workspaces.clone(),
        household_repo.clone(),
        git.clone(),
        containers.clone(),
        listeners_arc,
        progress_handle,           // <-- new arg
        claude_dir,
        api_key,
    ));

let state = app::state::DaemonState {
    db: db.clone(),
    cards,
    workspaces,
    containers,
    git,
    listeners: listeners.clone(),
    household_repo,
    household_lookup,
    workspace_lifecycle,
    progress,                       // <-- new field
    card_locks: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
};
```

- [ ] **Step 4: Build (compile-only check)**

```
cargo build -p reeved
```

Expected: clean build. `progress` field is unused (warn) but no errors. We'll use it in the next task.

- [ ] **Step 5: Run existing tests to verify no regression**

```
cargo test --workspace --lib
```

Expected: all 178 tests pass (no functional change yet, just signature plumbing).

- [ ] **Step 6: Commit**

```
git add crates/reeve-workspace/src/lifecycle.rs crates/reeved/src/app/state.rs crates/reeved/src/main.rs
git commit -m "feat(m7): plumb ProgressReporter through LiveWorkspaceLifecycle"
```

---

## Task 5: Use run_stage in ensure_running; tmux becomes fatal

**Files:**
- Modify: `crates/reeve-workspace/src/lifecycle.rs`
- Modify: `crates/reeve-workspace/src/clones.rs`

This is the producer-side core change. After this task the daemon emits real progress events.

- [ ] **Step 1: Refactor `clone_all_for_card` signature**

Edit `crates/reeve-workspace/src/clones.rs`. The function signature gains two parameters:

```rust
pub async fn clone_all_for_card(
    ops: &dyn GitOps,
    household: &Household,
    manifest: &Manifest,
    workspace: &Path,
    progress: &dyn crate::ProgressReporter,
    card_id: &str,
) -> Result<()> {
    let meta = &household.meta_repo_path;

    // Pre-flight check (unchanged) — no progress stage.
    for entry in manifest.siblings() {
        let dir = meta.join(&entry.name);
        if !dir.exists() {
            anyhow::bail!(
                "sibling '{}' declared in household.json but not cloned at {} — \
                 run your workspace's bootstrap (e.g. `make setup`) first",
                entry.name,
                dir.display(),
            );
        }
    }

    // Step 1: meta-repo → workspace. Wrapped in run_stage.
    crate::run_stage(
        progress,
        card_id,
        reeve_protocol::ProgressKind::CloneMeta {
            name: manifest.meta_repo.clone(),
        },
        ensure_cloned(ops, meta, workspace, "workspace meta-repo"),
    )
    .await?;

    // Step 2: each separate-sibling (has source/.git/) gets a stage.
    // Inline entries (no source/.git/) are NOT cloned and produce NO stage.
    for entry in manifest.siblings() {
        let source = meta.join(&entry.name);
        if !source.join(".git").exists() {
            continue; // inline; presence verified in the loop below
        }
        let dest = workspace.join(&entry.name);
        crate::run_stage(
            progress,
            card_id,
            reeve_protocol::ProgressKind::CloneSibling {
                name: entry.name.clone(),
            },
            ensure_cloned(ops, &source, &dest, &format!("sibling '{}'", entry.name)),
        )
        .await?;
    }

    // Step 3: verify inline entries actually arrived in step 1's clone.
    for entry in manifest.siblings() {
        let source = meta.join(&entry.name);
        if source.join(".git").exists() {
            continue; // separate sibling — handled above
        }
        let dest = workspace.join(&entry.name);
        if !dest.exists() {
            anyhow::bail!(
                "inline entry '{}' is missing from workspace clone — \
                 is it correctly tracked in the meta-repo?",
                entry.name,
            );
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Update all call sites of `clone_all_for_card`**

```
grep -rn "clone_all_for_card" crates/ tests/
```

Update each call to pass `progress` + `card_id`. In `lifecycle.rs::ensure_running` (Step 3 below) that's `&*self.progress, card_id`. In any test, pass a mock reporter.

- [ ] **Step 3: Refactor `ensure_running` to use run_stage**

Edit `crates/reeve-workspace/src/lifecycle.rs`. Replace the body of `impl WorkspaceLifecycle for LiveWorkspaceLifecycle::ensure_running` after the adopt block:

```rust
// (Adopt-existing-container block at lines ~89-100 unchanged.)

let manifest = reeve_household::load_household_manifest(&household)
    .with_context(|| format!("load manifest for '{}'", household.name))?;

if let Some(parent) = workspace_path.parent() {
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create cards root {}", parent.display()))?;
}

// Clones are progress-reported per-entry inside clone_all_for_card.
crate::clones::clone_all_for_card(
    &*self.git,
    &household,
    &manifest,
    &workspace_path,
    &*self.progress,
    card_id,
)
.await?;

let devcontainer_path = workspace_path.join(".devcontainer").join("devcontainer.json");
if !devcontainer_path.exists() {
    anyhow::bail!(
        "devcontainer.json missing at {} — every witan-household must ship one at the conventional location",
        devcontainer_path.display(),
    );
}

let argv = crate::container::devcontainer_up_command(
    card_id,
    &workspace_path,
    &devcontainer_path,
    &cards_socket_dir,
    &self.claude_dir,
    self.api_key.as_deref(),
);

// DevcontainerUp groups spawn + discover + upsert as one semantic stage.
// Borrow each captured ref so the closure stays non-Move.
let containers = &*self.containers;
let workspaces = &*self.workspaces;
let progress = &*self.progress;

let container_name = crate::run_stage(
    progress,
    card_id,
    reeve_protocol::ProgressKind::DevcontainerUp,
    async {
        containers.spawn(argv).await?;
        let name = discover_with_deadline(containers, card_id).await?;
        workspaces
            .upsert_container(
                card_id,
                &name,
                reeve_protocol::ContainerStatus::Running,
                now_secs(),
            )
            .await?;
        Ok::<_, anyhow::Error>(name)
    },
)
.await?;

self.listeners.spawn(card_id, socket).await;

// Tmux start is NOW FATAL. Old warn-not-bail path is removed; M6's session
// pane makes a card-without-tmux non-functional from the TUI's POV. The
// M3-followup-5 reorder ensures a failure here keeps the column unchanged.
let tmux_cmd = format!(
    "export PATH=\"$HOME/.local/bin:$PATH\" && cd /workspaces/{card_id} && (claude || bash)"
);
crate::run_stage(
    progress,
    card_id,
    reeve_protocol::ProgressKind::StartingTmux,
    containers.exec_in_tmux(&container_name, "reeve", &tmux_cmd),
)
.await?;

progress
    .report(
        card_id,
        reeve_protocol::ProgressKind::Done,
        reeve_protocol::ProgressStatus::Completed,
    )
    .await;
Ok(())
```

Also update `ensure_stopped` to call `progress.clear`:

```rust
async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
    self.listeners.stop(card_id).await?;

    let row = match self.workspaces.get_container(card_id).await? {
        Some(r) => r,
        None => {
            self.progress.clear(card_id).await; // even with no container, drop progress
            return Ok(());
        }
    };

    if self
        .containers
        .inspect_status(&row.container_name)
        .await?
        .is_none()
    {
        self.workspaces.remove_container(card_id).await?;
        self.progress.clear(card_id).await;
        return Ok(());
    }

    self.containers.teardown(&row.container_name).await?;
    self.workspaces.remove_container(card_id).await?;
    self.progress.clear(card_id).await;
    Ok(())
}
```

- [ ] **Step 4: Write a happy-path test**

This requires a fake `ProgressReporter`, `ContainerOps`, etc. The existing tests in `crates/reeve-workspace/src/clones.rs` already use mock GitOps. Extend with a `RecordingProgress`:

Add to `crates/reeve-workspace/src/clones.rs` test module (alongside `MockGitOps`):

```rust
#[derive(Default)]
struct RecordingProgress {
    calls: std::sync::Mutex<Vec<(String, reeve_protocol::ProgressKind, reeve_protocol::ProgressStatus)>>,
}

#[async_trait::async_trait]
impl crate::ProgressReporter for RecordingProgress {
    async fn report(
        &self,
        card_id: &str,
        kind: reeve_protocol::ProgressKind,
        status: reeve_protocol::ProgressStatus,
    ) {
        self.calls
            .lock()
            .unwrap()
            .push((card_id.to_string(), kind, status));
    }
    async fn clear(&self, _: &str) {}
}
```

Then a test (place after the existing `clone_all_for_card_*` tests):

```rust
#[tokio::test]
async fn clone_all_emits_stages_for_separate_siblings_skips_inline() {
    use reeve_protocol::{ProgressKind, ProgressStatus};

    let dir = tempdir().unwrap();
    let meta = dir.path().join("meta");
    std::fs::create_dir_all(meta.join(".git")).unwrap();
    // separate sibling has its own .git/
    std::fs::create_dir_all(meta.join("backend").join(".git")).unwrap();
    // inline entry is just a dir, no .git/
    std::fs::create_dir_all(meta.join("lore")).unwrap();

    let workspace = dir.path().join("workspace");
    // clone op needs to "create" the workspace at the meta path; mock just
    // creates the destination + .git/.
    let ops = MockGitOps::new_with_clone_side_effect(|_, dest| {
        std::fs::create_dir_all(dest.join(".git")).unwrap();
        // For the workspace clone, also create the inline 'lore' dir so
        // the post-clone presence check passes.
        if dest.file_name().and_then(|n| n.to_str()) == Some("workspace") {
            std::fs::create_dir_all(dest.join("lore")).unwrap();
        }
    });

    let household = Household {
        name: "h".into(),
        meta_repo_path: meta.clone(),
    };
    let manifest = Manifest {
        meta_repo: "meta".into(),
        knowledge_base: None,
        repos: vec![
            RepoEntry { name: "meta".into(), url: None, tags: vec![], description: None },
            RepoEntry { name: "backend".into(), url: None, tags: vec![], description: None },
            RepoEntry { name: "lore".into(), url: None, tags: vec![], description: None },
        ],
    };
    let progress = RecordingProgress::default();

    clone_all_for_card(&ops, &household, &manifest, &workspace, &progress, "card-1")
        .await
        .unwrap();

    let calls = progress.calls.lock().unwrap();
    let kinds: Vec<&ProgressKind> = calls.iter().map(|(_, k, _)| k).collect();
    // Expected: CloneMeta(Active+Completed), CloneSibling{backend}(Active+Completed).
    // NO entries for inline 'lore'.
    assert_eq!(calls.len(), 4, "two stages × 2 events: {calls:?}");
    assert!(matches!(kinds[0], ProgressKind::CloneMeta { .. }));
    assert!(matches!(kinds[1], ProgressKind::CloneMeta { .. }));
    assert_eq!(calls[0].2, ProgressStatus::Active);
    assert_eq!(calls[1].2, ProgressStatus::Completed);
    assert!(matches!(kinds[2], ProgressKind::CloneSibling { name } if name == "backend"));
    assert!(matches!(kinds[3], ProgressKind::CloneSibling { name } if name == "backend"));
    assert_eq!(calls[2].2, ProgressStatus::Active);
    assert_eq!(calls[3].2, ProgressStatus::Completed);
    // No CloneSibling{name:"lore"} ever:
    assert!(!kinds.iter().any(|k| matches!(k, ProgressKind::CloneSibling { name } if name == "lore")),
        "inline entries must not produce CloneSibling stages");
}
```

Note: `MockGitOps::new_with_clone_side_effect` — if the existing mock doesn't expose this, add it as a simple constructor that takes a closure stored alongside `calls`:

```rust
impl MockGitOps {
    fn new_with_clone_side_effect(
        side_effect: impl Fn(&Path, &Path) + Send + Sync + 'static,
    ) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            // ... existing fields default ...
            clone_side_effect: Some(Box::new(side_effect)),
        }
    }
}
```

And the existing `MockGitOps::clone` impl needs to call the side-effect (if any). If retrofitting that is annoying, alternatively bypass the mock entirely for THIS test and use a `RealGitOps`-equivalent that uses `Command::new("git")`. The simpler option: assemble the directory shape by hand BEFORE calling `clone_all_for_card` and tell the mock to no-op — but then the function's "create workspace via clone" assumption breaks. Implementer judgement call here; pick whichever is least intrusive given the existing test patterns. Document the choice in the test's comment.

- [ ] **Step 5: Write a failure-path test**

```rust
#[tokio::test]
async fn clone_all_emits_failed_when_sibling_clone_errors() {
    use reeve_protocol::{ProgressKind, ProgressStatus};

    // Set up so the meta clone succeeds but backend's git.clone returns Err.
    // (Implementer fills in based on the mock framework chosen above.)

    let calls = progress.calls.lock().unwrap();
    // Expected: CloneMeta(Active+Completed), CloneSibling{backend}(Active+Failed).
    // No Done — the error propagated.
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[3].2, ProgressStatus::Failed);
}
```

- [ ] **Step 6: Run tests**

```
cargo test -p reeve-workspace
```

Expected: all pass including the new clone-stage tests. If any pre-existing test broke because it called `clone_all_for_card` with the old signature, update it to pass a `RecordingProgress::default()` + a dummy `"test-card"` id.

- [ ] **Step 7: Update the existing M3 warn-not-bail comment**

The old comment in `lifecycle.rs` near the tmux call referenced M5 + warn-not-bail. Since it's now fatal, remove the stale comment and replace with:

```rust
// M7: tmux start is fatal — a card in Implementing without a tmux session
// is non-functional from the M6 session pane's POV. The M3-followup-5 mv
// handler reorder keeps the card's column in its old value if this errors.
```

- [ ] **Step 8: Commit**

```
git add crates/reeve-workspace/src/clones.rs crates/reeve-workspace/src/lifecycle.rs
git commit -m "feat(m7): emit progress events from ensure_running + clone_all_for_card

clone_all_for_card now takes &dyn ProgressReporter + card_id and wraps
each per-entry clone (meta + separate siblings only — inline entries
get no stage) in run_stage. ensure_running adds DevcontainerUp stage
covering spawn+discover+upsert, StartingTmux stage covering exec_in_tmux,
and final Done(Completed). Tmux failure is now fatal (was warn-not-bail
under M5; obsolete under M6's session pane). ensure_stopped now calls
progress.clear so card teardown wipes any stale progress data."
```

---

## Task 6: card_progress RPC dispatch

**Files:**
- Modify: `crates/reeved/src/dispatch.rs`

- [ ] **Step 1: Add the dispatch arm**

In `crates/reeved/src/dispatch.rs`, find the existing `"ls" =>` / `"new" =>` / `"mv" =>` arms. Insert:

```rust
"card_progress" => {
    let params = parse_params::<reeve_protocol::CardProgressParams>(req.params)?;
    let stages = state.progress.get(&params.id).await;
    serde_json::to_value(reeve_protocol::CardProgressResult {
        card_id: params.id,
        stages,
    })
    .map_err(internal)
}
```

No per-card lock (read-only). No household lookup.

- [ ] **Step 2: Test the dispatch arm**

Find the existing dispatch-arm tests in the same file (under `#[cfg(test)] mod tests`). Add:

```rust
#[tokio::test]
async fn card_progress_unknown_card_returns_empty_stages() {
    let state = fresh_state().await;
    let req = RpcRequest {
        id: 1,
        method: "card_progress".into(),
        params: serde_json::json!({"id": "no-such-card"}),
    };
    let v = dispatch(req, state, CallerContext::host()).await.unwrap();
    let result: reeve_protocol::CardProgressResult = serde_json::from_value(v).unwrap();
    assert_eq!(result.card_id, "no-such-card");
    assert!(result.stages.is_empty());
}

#[tokio::test]
async fn card_progress_returns_reported_stages() {
    use reeve_protocol::{ProgressKind, ProgressStatus};
    let state = fresh_state().await;
    state.progress.report("c1",
        ProgressKind::DevcontainerUp,
        ProgressStatus::Active,
    ).await;
    let req = RpcRequest {
        id: 1,
        method: "card_progress".into(),
        params: serde_json::json!({"id": "c1"}),
    };
    let v = dispatch(req, state, CallerContext::host()).await.unwrap();
    let result: reeve_protocol::CardProgressResult = serde_json::from_value(v).unwrap();
    assert_eq!(result.stages.len(), 1);
    assert_eq!(result.stages[0].status, ProgressStatus::Active);
}
```

These rely on `fresh_state` constructing a `DaemonState` with the new `progress` field. If `fresh_state` already exists, find it and add `progress: app::progress::InMemoryProgress::new()`. If multiple fresh-state builders exist (per the M3-followup-1 commit history), update all of them.

- [ ] **Step 3: Run tests**

```
cargo test -p reeved card_progress
```

Expected: 2 new tests pass.

- [ ] **Step 4: Commit**

```
git add crates/reeved/src/dispatch.rs
git commit -m "feat(m7): card_progress RPC dispatch arm"
```

---

## Task 7: TUI client wrapper + app state

**Files:**
- Modify: `crates/reeve/src/tui/client_ext.rs`
- Modify: `crates/reeve/src/tui/app.rs`

- [ ] **Step 1: Add typed RPC wrapper**

Edit `crates/reeve/src/tui/client_ext.rs`. Add at the bottom of the file:

```rust
pub async fn rpc_card_progress(
    params: reeve_protocol::CardProgressParams,
) -> Result<Result<reeve_protocol::CardProgressResult, RpcError>> {
    call("card_progress", params).await
}
```

(Or whatever the existing `call` helper is named — match the pattern of `rpc_ls`, `rpc_mv`, etc.)

- [ ] **Step 2: Add app state fields**

Edit `crates/reeve/src/tui/app.rs`. In the `App` struct, add after `pending_mv_started_at`:

```rust
/// card_id → most recent CardProgressResult.stages. Survives MvErr;
/// GC'd when a live session exists for the card.
pub card_progress: std::collections::HashMap<String, Vec<reeve_protocol::ProgressStage>>,
/// Last poll instant per card; used to rate-limit the card_progress
/// fetch to ~400ms while pending_mvs has the card.
pub card_progress_last_poll: std::collections::HashMap<String, std::time::Instant>,
```

In `App::new`, initialize both as empty HashMaps:

```rust
card_progress: std::collections::HashMap::new(),
card_progress_last_poll: std::collections::HashMap::new(),
```

- [ ] **Step 3: Build**

```
cargo build -p reeve
```

Expected: clean (the fields are unused — warn, no error).

- [ ] **Step 4: Commit**

```
git add crates/reeve/src/tui/client_ext.rs crates/reeve/src/tui/app.rs
git commit -m "feat(m7): TUI rpc_card_progress wrapper + app state fields"
```

---

## Task 8: Poll card_progress in the drive loop + MvErr extra fetch

**Files:**
- Modify: `crates/reeve/src/tui/actions.rs`
- Modify: `crates/reeve/src/tui/app.rs`

- [ ] **Step 1: Add a `poll_card_progress` helper in actions.rs**

Place near the existing `refresh` function in `crates/reeve/src/tui/actions.rs`:

```rust
/// Polls card_progress for every card in pending_mvs whose last fetch is
/// at least `poll_interval_ms` old. Updates app.card_progress in place.
pub async fn poll_card_progress(app: &mut App, poll_interval_ms: u64) {
    let now = std::time::Instant::now();
    let pending_ids: Vec<String> = app.pending_mvs.keys().cloned().collect();
    for card_id in pending_ids {
        let due = app
            .card_progress_last_poll
            .get(&card_id)
            .map(|t| now.duration_since(*t).as_millis() as u64 >= poll_interval_ms)
            .unwrap_or(true);
        if !due {
            continue;
        }
        if let Ok(Ok(r)) = crate::tui::client_ext::rpc_card_progress(
            reeve_protocol::CardProgressParams { id: card_id.clone() },
        )
        .await
        {
            app.card_progress.insert(card_id.clone(), r.stages);
            app.card_progress_last_poll.insert(card_id, now);
        }
    }

    // GC: drop progress data for cards that now have a live session — the
    // session pane has taken over the visible display. Failed cards (no
    // session) keep their data so the user can still see what failed.
    app.card_progress
        .retain(|id, _| app.sessions.get(id).is_none());
    app.card_progress_last_poll
        .retain(|id, _| app.sessions.get(id).is_none());
}
```

- [ ] **Step 2: Call the poll from the drive loop**

In `crates/reeve/src/tui/app.rs`, find the periodic poll block (currently calls `actions::refresh(&mut app).await` every 500ms). Add the progress poll right after the refresh:

```rust
if app.last_refresh.elapsed() > Duration::from_millis(500) {
    crate::tui::actions::refresh(&mut app).await;
}
// Card-progress poll runs every drive iteration (~50ms tick) but the
// helper rate-limits to 400ms per card. Cheap when no cards are pending.
crate::tui::actions::poll_card_progress(&mut app, 400).await;
```

- [ ] **Step 3: Add the MvErr extra fetch**

In `process_op_event` (actions.rs), find the `OpEvent::MvErr` arm. Currently it does `decrement_pending` + clears `pending_mv_started_at` + sets a status error. Add a synchronous one-shot progress fetch BEFORE the decrement, so the loader has the terminal `Failed` stage cached even if the regular poll missed it:

```rust
OpEvent::MvErr { card_id, msg } => {
    // One-shot fetch BEFORE decrementing pending_mvs, so the loader
    // visibility gate ("card_progress has data + no session") catches
    // the Failed state. The regular 400ms poll may have just sampled
    // before the failure landed.
    if let Ok(Ok(r)) = crate::tui::client_ext::rpc_card_progress(
        reeve_protocol::CardProgressParams { id: card_id.clone() },
    )
    .await
    {
        app.card_progress.insert(card_id.clone(), r.stages);
    }
    decrement_pending(&mut app.pending_mvs, &card_id);
    if !app.pending_mvs.contains_key(&card_id) {
        app.pending_mv_started_at.remove(&card_id);
    }
    set_status_error(app, format!("mv failed: {msg}"));
}
```

Note: `process_op_event` is currently a sync `fn`. Since the new fetch is async, change its signature to `async fn` AND update its single call site in app.rs (the `while let Ok(event) = app.op_rx.try_recv() {` block) to `.await`. Be careful — the loop iterates events; each `process_op_event(...).await` blocks the loop briefly. That's acceptable since each await is a sub-ms UDS round trip.

If the implementer prefers to keep `process_op_event` synchronous, alternative: stash the fetch as a deferred action (e.g., insert `card_id` into a new `app.fetch_progress_now: HashSet<String>` field, and `poll_card_progress` consumes it ignoring rate limit). That keeps the function sync but adds a field. Either approach works.

- [ ] **Step 4: Run tests**

```
cargo test -p reeve
```

Expected: TUI tests still pass. No new test for `poll_card_progress` directly — it'll be covered by smoke + e2e. If the implementer wants a unit test, mock `rpc_card_progress` via a feature flag or extract the rate-limiting logic into a pure function.

- [ ] **Step 5: Commit**

```
git add crates/reeve/src/tui/actions.rs crates/reeve/src/tui/app.rs
git commit -m "feat(m7): poll card_progress in drive loop + extra fetch on MvErr"
```

---

## Task 9: Render the stage checklist; update loader-visibility gate

**Files:**
- Modify: `crates/reeve/src/tui/render.rs`

- [ ] **Step 1: Rewrite `render_provisioning_loader`**

Find the existing function in `crates/reeve/src/tui/render.rs` (added in M6 commit `ac6515a`). Replace its body:

```rust
fn render_provisioning_loader(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    app: &crate::tui::app::App,
    card_id: &str,
) {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph};

    let stages: &[reeve_protocol::ProgressStage] = app
        .card_progress
        .get(card_id)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    let elapsed = elapsed_str_from(&app.pending_mv_started_at, card_id);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Provisioning workspace…",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for stage in stages {
        let label = format_stage(&stage.kind);
        let span = match stage.status {
            reeve_protocol::ProgressStatus::Completed => Span::styled(
                format!("  ✓ {label}"),
                Style::default().fg(Color::Green),
            ),
            reeve_protocol::ProgressStatus::Active => Span::styled(
                format!("  {} {label}", spinner_frame()),
                Style::default().fg(Color::Yellow),
            ),
            reeve_protocol::ProgressStatus::Failed => Span::styled(
                format!("  ✗ {label}"),
                Style::default().fg(Color::Red),
            ),
        };
        lines.push(Line::from(span));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {elapsed}"),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  First-time provisioning of a card typically takes 30–90 seconds.",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Subsequent reprovisions reuse the cached image.",
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" card {} — provisioning ", &card_id[..8.min(card_id.len())])),
    );
    f.render_widget(para, area);
}

fn format_stage(kind: &reeve_protocol::ProgressKind) -> String {
    match kind {
        reeve_protocol::ProgressKind::CloneMeta { name } => format!("Cloning meta-repo {name}"),
        reeve_protocol::ProgressKind::CloneSibling { name } => format!("Cloning sibling {name}"),
        reeve_protocol::ProgressKind::DevcontainerUp => "Building container".into(),
        reeve_protocol::ProgressKind::StartingTmux => "Starting session".into(),
        reeve_protocol::ProgressKind::Done => "Ready".into(),
    }
}

fn spinner_frame() -> &'static str {
    // Pick a frame from the wall clock so animation runs without per-frame state.
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    FRAMES[(ms / 80) as usize % FRAMES.len()]
}

fn elapsed_str_from(
    started: &std::collections::HashMap<String, std::time::Instant>,
    card_id: &str,
) -> String {
    match started.get(card_id) {
        Some(t) => {
            let s = t.elapsed().as_secs();
            if s < 60 {
                format!("{s}s elapsed")
            } else {
                format!("{}m {}s elapsed", s / 60, s % 60)
            }
        }
        None => "elapsed: —".into(),
    }
}
```

(If `spinner_frame` or `elapsed_str_from` already exist from the M6 loader, don't duplicate; reuse the existing one.)

- [ ] **Step 2: Update the loader-visibility gate in `render_detail_pane`**

Find `render_detail_pane` in the same file. The M6 version had nested conditions on `card.column == Implementing` AND `session` AND `pending_mvs`. Replace with:

```rust
fn render_detail_pane(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    app: &crate::tui::app::App,
) {
    let Some(selected_id) = app.selected_card_id.as_ref() else {
        return render_no_selection(f, area);
    };
    let Some(card) = app.cards.iter().find(|c| &c.id == selected_id) else {
        return render_no_selection(f, area);
    };

    // 1. Live session takes priority — column-gated (only show when card
    //    is in Implementing AND we have a session).
    if card.column == reeve_protocol::Column::Implementing {
        if let Some(session) = app.sessions.get(&card.id) {
            return render_session_pane(f, area, app, &card.id, session);
        }
    }

    // 2. Progress data available → loader. NOT gated on card.column.
    //    A failed spawn whose optimistic column has reverted still
    //    surfaces the Failed checklist while the user is on the card.
    if app.card_progress.contains_key(&card.id) {
        return render_provisioning_loader(f, area, app, &card.id);
    }

    // 3. Static card detail.
    render_card_detail(f, area, app, card);
}
```

`render_session_pane`, `render_no_selection`, `render_card_detail` are the existing helpers; preserve their signatures. If they're inlined inside `render_detail_pane` today, extract them. Implementer notes the existing structure during the change.

- [ ] **Step 3: Build + run tests**

```
cargo build -p reeve
cargo test -p reeve
```

Expected: clean build; existing TUI tests pass.

- [ ] **Step 4: Commit**

```
git add crates/reeve/src/tui/render.rs
git commit -m "feat(m7): stage-checklist loader + new visibility gate

Renders the Vec<ProgressStage> from app.card_progress as a per-stage
checklist with ✓ / animated spinner / ✗ marks. Falls back to the M6
generic spinner copy when the RPC hasn't returned yet. The detail-
pane visibility gate is no longer tied to card.column == Implementing
or pending_mvs.contains_key — it's gated on card_progress having
data + no session, so failed spawns whose optimistic column reverted
still show the Failed checklist."
```

---

## Task 10: Destructive-mv guard — modal variant + is_destructive

**Files:**
- Modify: `crates/reeve/src/tui/modal.rs`
- Modify: `crates/reeve/src/tui/actions.rs`

- [ ] **Step 1: Add the modal variant**

Edit `crates/reeve/src/tui/modal.rs`. In the existing `ModalKind` enum, add:

```rust
ConfirmDestructiveMv {
    card_id: String,
    from: reeve_protocol::Column,
    to: reeve_protocol::Column,
},
```

- [ ] **Step 2: Add `is_destructive` + reroute `schedule_mv`**

Edit `crates/reeve/src/tui/actions.rs`. Find the existing `schedule_mv` (or `mv_relative` if that's the entry point). Rename the current body to `schedule_mv_unchecked` and add a checked wrapper:

```rust
pub fn schedule_mv(app: &mut App, card_id: String, target: reeve_protocol::Column) {
    let from = app
        .cards
        .iter()
        .find(|c| c.id == card_id)
        .map(|c| c.column);
    if let Some(from) = from {
        if from != target && is_destructive(from, target) {
            app.modal = Some(crate::tui::modal::ModalKind::ConfirmDestructiveMv {
                card_id,
                from,
                to: target,
            });
            return;
        }
    }
    schedule_mv_unchecked(app, card_id, target);
}

/// Whether the (from, to) transition would invoke `ensure_stopped` on the
/// daemon side. MUST mirror `reeve_card::transitions::on_column_change`'s
/// ensure_stopped routing. Drift-guarded by
/// `is_destructive_matrix_matches_lifecycle_routing` test below.
fn is_destructive(from: reeve_protocol::Column, to: reeve_protocol::Column) -> bool {
    use reeve_protocol::Column::*;
    matches!(
        (from, to),
        (Implementing | Testing | Review, Backlog | Planning)
            | (Implementing | Testing | Review, Done)
            | (Backlog | Planning, Done)
    )
}

fn schedule_mv_unchecked(app: &mut App, card_id: String, target: reeve_protocol::Column) {
    // (existing body of the original schedule_mv: optimistic update,
    //  pending_mv_intents insert, etc.)
}
```

- [ ] **Step 3: Test `is_destructive` matrix**

In the actions.rs test module:

```rust
#[test]
fn is_destructive_implementing_to_backlog() {
    use reeve_protocol::Column::*;
    assert!(is_destructive(Implementing, Backlog));
    assert!(is_destructive(Implementing, Planning));
}

#[test]
fn is_destructive_any_to_done() {
    use reeve_protocol::Column::*;
    assert!(is_destructive(Backlog, Done));
    assert!(is_destructive(Planning, Done));
    assert!(is_destructive(Implementing, Done));
    assert!(is_destructive(Testing, Done));
    assert!(is_destructive(Review, Done));
}

#[test]
fn is_destructive_within_pre_implementing_is_false() {
    use reeve_protocol::Column::*;
    assert!(!is_destructive(Backlog, Planning));
    assert!(!is_destructive(Planning, Implementing));
    assert!(!is_destructive(Backlog, Implementing));
}

#[test]
fn is_destructive_back_to_implementing_is_false() {
    use reeve_protocol::Column::*;
    assert!(!is_destructive(Testing, Implementing));
    assert!(!is_destructive(Review, Implementing));
}

#[test]
fn is_destructive_matrix_matches_lifecycle_routing() {
    // Cross-reference with reeve_card::transitions::on_column_change.
    // For each (from, to) pair, is_destructive must return true iff the
    // transition matrix would route to ensure_stopped.
    use reeve_protocol::Column;
    use reeve_protocol::Column::*;
    let all = [Backlog, Planning, Implementing, Testing, Review, Done];
    for from in all {
        for to in all {
            if from == to {
                continue;
            }
            // Hand-coded mirror of on_column_change's ensure_stopped arms:
            let routes_to_ensure_stopped = matches!(
                (from, to),
                (_, Done) | (Implementing | Testing | Review, Backlog | Planning)
            );
            assert_eq!(
                is_destructive(from, to),
                routes_to_ensure_stopped,
                "drift: ({from:?} → {to:?}) is_destructive={} but routes_to_ensure_stopped={}",
                is_destructive(from, to),
                routes_to_ensure_stopped,
            );
        }
    }
}
```

- [ ] **Step 4: Run tests**

```
cargo test -p reeve is_destructive
```

Expected: 5 passed.

- [ ] **Step 5: Commit**

```
git add crates/reeve/src/tui/modal.rs crates/reeve/src/tui/actions.rs
git commit -m "feat(m7): is_destructive matrix + ConfirmDestructiveMv modal variant

schedule_mv now checks is_destructive(from, to) before firing the
RPC; destructive transitions open a ConfirmDestructiveMv modal
instead of proceeding. The actual modal render + key handling come
in the next task. The is_destructive matrix mirrors
reeve_card::transitions::on_column_change; drift is guarded by a
test that hand-evaluates the same matrix and asserts agreement."
```

---

## Task 11: Confirmation modal render + key handling

**Files:**
- Modify: `crates/reeve/src/tui/render.rs`
- Modify: `crates/reeve/src/tui/actions.rs`

- [ ] **Step 1: Render the modal**

In `crates/reeve/src/tui/render.rs`, find `render_modal` (or wherever existing modal variants are matched). Add an arm for `ConfirmDestructiveMv`:

```rust
crate::tui::modal::ModalKind::ConfirmDestructiveMv { card_id: _, from, to } => {
    let area = centered_rect(60, 25, area);
    let from_label = from.label();
    let to_label = to.label();
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Move from {from_label} to {to_label}?"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  This will tear down the container and end the",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  claude session for this card.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Continue?     [y]es     [n]o",
            Style::default().fg(Color::Yellow),
        )),
    ];
    let popup = Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" Tear down workspace? "),
        );
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(popup, area);
}
```

`centered_rect`, `from.label()`, `to.label()` should already exist (used by other modals).

- [ ] **Step 2: Handle keys**

In `crates/reeve/src/tui/actions.rs`, find `handle_modal_key` (the function that matches on `ModalKind`). Add:

```rust
crate::tui::modal::ModalKind::ConfirmDestructiveMv { card_id, from: _, to } => {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            // User confirmed — close modal and proceed with the deferred mv.
            let id = card_id.clone();
            let target = *to;
            app.modal = None;
            schedule_mv_unchecked(app, id, target);
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.modal = None;
        }
        _ => {}
    }
}
```

- [ ] **Step 3: Test the y/n paths**

Add to actions.rs test module:

```rust
#[tokio::test]
async fn confirm_destructive_y_calls_unchecked_mv() {
    let mut app = App::new_for_test();
    let card_id = "abc".to_string();
    app.cards.push(test_card(&card_id, Column::Implementing));
    app.modal = Some(ModalKind::ConfirmDestructiveMv {
        card_id: card_id.clone(),
        from: Column::Implementing,
        to: Column::Backlog,
    });

    let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
    handle_modal_key(&mut app, key).await;

    assert!(app.modal.is_none(), "modal should be dismissed");
    assert!(
        app.pending_mv_intents.contains_key(&card_id),
        "schedule_mv_unchecked should have queued the mv",
    );
}

#[tokio::test]
async fn confirm_destructive_n_cancels() {
    let mut app = App::new_for_test();
    let card_id = "abc".to_string();
    app.cards.push(test_card(&card_id, Column::Implementing));
    app.modal = Some(ModalKind::ConfirmDestructiveMv {
        card_id: card_id.clone(),
        from: Column::Implementing,
        to: Column::Backlog,
    });

    let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
    handle_modal_key(&mut app, key).await;

    assert!(app.modal.is_none(), "modal should be dismissed");
    assert!(
        !app.pending_mv_intents.contains_key(&card_id),
        "no mv should have been queued",
    );
}
```

If `App::new_for_test` and `test_card` don't already exist, look for any existing app-setup helper in actions.rs tests and reuse — many modal tests likely have one.

- [ ] **Step 4: Run tests**

```
cargo test -p reeve confirm_destructive
```

Expected: 2 passed.

- [ ] **Step 5: Commit**

```
git add crates/reeve/src/tui/render.rs crates/reeve/src/tui/actions.rs
git commit -m "feat(m7): render + handle ConfirmDestructiveMv modal

y/Enter proceeds with schedule_mv_unchecked; n/Esc cancels."
```

---

## Task 12: E2E test + workspace verification + manual smoke

**Files:**
- Modify: `tests/e2e.rs`

- [ ] **Step 1: Add the e2e test**

Append to `tests/e2e.rs` after `m6_daemon_round_trip_for_tui_smoke`:

```rust
#[tokio::test]
async fn m7_card_progress_returns_empty_for_unknown_card() {
    build_workspace();
    let temp = tempfile::tempdir().unwrap();
    let env = isolated_env_with_config(temp.path());

    let _daemon = DaemonGuard(
        cmd("reeved", &env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reeved"),
    );

    let socket = temp.path().join("reeved.sock");
    let mut waited = Duration::ZERO;
    while !socket.exists() && waited < Duration::from_secs(2) {
        sleep(Duration::from_millis(50));
        waited += Duration::from_millis(50);
    }
    assert!(socket.exists(), "daemon socket not created within 2s");

    // No CLI raw-RPC subcommand exists; talk to the daemon directly
    // via the reeve client crate.
    let result = reeve::client::call(
        &socket,
        "card_progress",
        serde_json::json!({"id": "no-such-card"}),
    )
    .await
    .expect("transport ok")
    .expect("dispatch ok");

    let parsed: reeve_protocol::CardProgressResult =
        serde_json::from_value(result).expect("deserialize");
    assert_eq!(parsed.card_id, "no-such-card");
    assert!(parsed.stages.is_empty(), "no stages for unknown id");
}
```

The test depends on `reeve::client::call` being public from the `reeve` crate. Inspect `crates/reeve/src/client.rs` (or wherever `call` lives) and verify `pub use client::call;` in `crates/reeve/src/lib.rs`. If not exported, add a `pub mod client;` and `pub use client::call;` so the e2e test can reach it.

Note: this test is `#[tokio::test]` not `#[test]` because `client::call` is async. Check whether other tests in `tests/e2e.rs` are already tokio-tests (m1 / m6 may not be) and add the `#[tokio::main]` flavor or sync-wrapper as needed.

- [ ] **Step 2: Run the new test**

```
cargo test --test e2e m7_card_progress_returns_empty
```

Expected: PASS.

- [ ] **Step 3: Workspace-wide verification**

```
cargo test --workspace
cargo fmt --check
cargo clippy --workspace
```

`cargo test --workspace` must be clean. `cargo fmt --check` must produce no diff. `cargo clippy --workspace` may have pre-existing warnings (the 18 nits flagged in M6 review) but MUST NOT introduce new errors. Treat new warnings as a flag for review.

If `cargo fmt --check` shows drift from any of the M7 tasks, commit it:

```
cargo fmt
git add -A
git commit -m "chore(m7): cargo fmt residue"
```

- [ ] **Step 4: Commit any deferred work**

```
git add tests/e2e.rs
git commit -m "test(m7): e2e — card_progress dispatch round-trip via client::call"
```

- [ ] **Step 5: Manual smoke (paste this checklist into PR description)**

After the daemon is rebuilt + restarted:

1. Create a fresh card in a household whose meta-repo is workable. mv to Implementing. Drill in (`Enter`).
2. **Within ~5s**: loader shows `✓ Cloning meta-repo <name>` then `⠋ Cloning sibling <name>` (or `⠋ Building container` if no separate sibling).
3. **Within ~30-90s**: each clone stage flips to ✓; `⠋ Building container` runs; then `⠋ Starting session`; then loader collapses to the live session pane.
4. **Failure test**: mv a card whose household's meta-repo has been moved/deleted. Loader shows `✗ Cloning meta-repo` red, status bar carries error, card column reverts to its prior value, loader stays visible.
5. **Press `H` on the Implementing card** (containing an active session). Modal opens: "Move from Implementing to Backlog? This will tear down...". Press `n`. Modal closes, card stays in Implementing, session intact.
6. Repeat (5) with `y`. Card moves to Backlog, container tears down, progress data cleared.
7. mv a Backlog card to Planning with `L`. **No modal**. Card moves immediately (no destructive transition).
8. mv a Backlog card to Done via `m` picker (selects Done). Modal opens (Done is destructive from any source).

Document any deviation in the PR description.

---

## Self-review

**Spec coverage check (against `docs/superpowers/specs/2026-05-22-m7-progress-events-and-mv-guard-design.md`):**

- Wire protocol (CardProgress*, ProgressStage, ProgressKind, ProgressStatus) → Task 1 ✓
- ProgressReporter port → Task 2 ✓
- run_stage helper → Task 2 ✓
- InMemoryProgress storage → Task 3 ✓
- Plumbing through LiveWorkspaceLifecycle + DaemonState + main.rs → Task 4 ✓
- Producer integration (ensure_running uses run_stage; clone_all_for_card per-entry; inline entries skipped; DevcontainerUp groups spawn+discover+upsert; tmux fatal) → Task 5 ✓
- card_progress dispatch arm → Task 6 ✓
- rpc_card_progress client wrapper + app state → Task 7 ✓
- Drive-loop poll + GC (gated on sessions, not pending_mvs) + MvErr extra fetch → Task 8 ✓
- Stage-checklist render + new loader-visibility gate → Task 9 ✓
- is_destructive matrix + schedule_mv reroute + drift test → Task 10 ✓
- ConfirmDestructiveMv modal render + key handling + y/n tests → Task 11 ✓
- E2E test using client::call directly + workspace verification + manual smoke → Task 12 ✓

**Placeholders:** none. Every step has full code or a clear test-and-implement loop.

**Type consistency:**
- `ProgressKind`, `ProgressStatus`, `ProgressStage` defined in Task 1 and used identically in Tasks 2/3/5/9.
- `ProgressReporter` trait signature in Task 2 matches consumers in Tasks 3/5.
- `card_progress` field on App (Task 7) used by poll (Task 8) + render (Task 9).
- `is_destructive` signature matches the schedule_mv reroute (Task 10) and the modal handler (Task 11).
- `ConfirmDestructiveMv { card_id, from, to }` variant fields match across modal.rs (Task 10), render.rs (Task 11), and the handle_modal_key (Task 11).

**Known judgement calls flagged for implementer:**
- Task 5 step 4: how to extend the existing MockGitOps to support a clone side-effect — pick the least intrusive option.
- Task 8 step 3: process_op_event made async vs deferred-fetch field — both work, implementer's choice.
- Task 9 step 2: render_session_pane / render_no_selection / render_card_detail extraction depends on the existing structure; preserve it.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-22-m7-progress-events-and-mv-guard.md`. Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, two-stage review (spec + code quality) after each. Same workflow as M6.

**2. Inline Execution** — execute tasks in this session via executing-plans.

Which approach?
