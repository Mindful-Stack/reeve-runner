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
