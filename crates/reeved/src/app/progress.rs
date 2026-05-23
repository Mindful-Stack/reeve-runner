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

#[derive(Default)]
pub struct InMemoryProgress {
    state: Arc<Mutex<HashMap<String, Vec<ProgressStage>>>>,
}

impl InMemoryProgress {
    pub fn new() -> Self {
        Self::default()
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

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A stage is "terminal" in the sense of "the run is over" only when the
/// whole lifecycle failed (any kind + Failed) or fully succeeded (Done +
/// Completed). Individual step completions (e.g. CloneSibling + Completed)
/// are not terminal — the lifecycle may still be in progress.
fn is_run_terminal(stage: &ProgressStage) -> bool {
    match stage.status {
        ProgressStatus::Failed => true,
        ProgressStatus::Completed => matches!(stage.kind, ProgressKind::Done),
        ProgressStatus::Active => false,
    }
}

#[async_trait]
impl ProgressReporter for InMemoryProgress {
    async fn report(&self, card_id: &str, kind: ProgressKind, status: ProgressStatus) {
        let mut guard = self.state.lock().await;
        let entry = guard.entry(card_id.to_string()).or_default();

        match status {
            ProgressStatus::Active => {
                // If the most recent stage was terminal (Completed or
                // Failed), this is a fresh ensure_running attempt — reset.
                let prior_terminal = entry.last().map(is_run_terminal).unwrap_or(false);
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
                // Mutate the most recent stage whose kind matches and is
                // still Active. If we can't find one (shouldn't happen —
                // run_stage always emits Active first), log a warn and
                // append a synthetic terminal stage.
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
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ProgressKind::DevcontainerUp);
        assert_eq!(v[0].status, ProgressStatus::Active);
        assert!(v[0].finished_at.is_none());
    }

    #[tokio::test]
    async fn report_completed_mutates_most_recent_active() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Completed)
            .await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].status, ProgressStatus::Completed);
        assert!(v[0].finished_at.is_some());
    }

    #[tokio::test]
    async fn report_completed_matches_kind_not_just_latest() {
        let p = InMemoryProgress::default();
        p.report(
            "c",
            ProgressKind::CloneSibling { name: "a".into() },
            ProgressStatus::Active,
        )
        .await;
        p.report(
            "c",
            ProgressKind::CloneSibling { name: "a".into() },
            ProgressStatus::Completed,
        )
        .await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        // Completing 'a' AFTER DevcontainerUp's Active: there's no Active
        // 'a' to mutate. We log + append a synthetic.
        p.report(
            "c",
            ProgressKind::CloneSibling { name: "a".into() },
            ProgressStatus::Completed,
        )
        .await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 3);
        assert_eq!(v[1].status, ProgressStatus::Active); // devcontainer_up still active
    }

    #[tokio::test]
    async fn report_active_after_terminal_resets_vec() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Completed)
            .await;
        p.report("c", ProgressKind::Done, ProgressStatus::Completed)
            .await;
        // Fresh ensure_running:
        p.report(
            "c",
            ProgressKind::CloneMeta { name: "x".into() },
            ProgressStatus::Active,
        )
        .await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ProgressKind::CloneMeta { name: "x".into() });
    }

    #[tokio::test]
    async fn report_active_after_failed_resets_vec() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Failed)
            .await;
        p.report(
            "c",
            ProgressKind::CloneMeta { name: "x".into() },
            ProgressStatus::Active,
        )
        .await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1);
    }

    #[tokio::test]
    async fn clear_removes_entry() {
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        p.clear("c").await;
        assert!(p.get("c").await.is_empty());
    }

    #[tokio::test]
    async fn cards_are_independent() {
        let p = InMemoryProgress::default();
        p.report("a", ProgressKind::DevcontainerUp, ProgressStatus::Active)
            .await;
        p.report(
            "b",
            ProgressKind::CloneMeta { name: "x".into() },
            ProgressStatus::Active,
        )
        .await;
        p.clear("a").await;
        assert!(
            p.get("a").await.is_empty(),
            "a's clear didn't remove a's stages"
        );
        assert_eq!(p.get("b").await.len(), 1, "a's clear must not touch b");
    }

    #[tokio::test]
    async fn report_completed_with_no_active_match_appends_synthetic() {
        // Direct test of the no-match-append branch: a Completed report
        // with no preceding Active for that kind. The synthetic stage
        // should carry started_at == finished_at, matching kind, matching
        // status.
        let p = InMemoryProgress::default();
        p.report("c", ProgressKind::DevcontainerUp, ProgressStatus::Completed)
            .await;
        let v = p.get("c").await;
        assert_eq!(v.len(), 1, "synthetic stage appended");
        assert_eq!(v[0].kind, ProgressKind::DevcontainerUp);
        assert_eq!(v[0].status, ProgressStatus::Completed);
        assert!(
            v[0].finished_at.is_some(),
            "synthetic stage has finished_at"
        );
        assert_eq!(
            v[0].started_at,
            v[0].finished_at.unwrap(),
            "started_at == finished_at signals 'no active phase observed'"
        );
    }
}
