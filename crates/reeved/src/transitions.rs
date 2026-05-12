//! Column-transition side effects. Decides what container ops to invoke
//! when a card moves between columns. This is the only module that
//! encodes the transition matrix.

use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{Card, Column};

#[async_trait]
pub trait TransitionEffects: Send + Sync {
    /// Card needs a running container (spawn if not already).
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    /// Card no longer needs a container (teardown if running).
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

pub async fn on_column_change(
    old: Column,
    new: Column,
    card: &Card,
    fx: &dyn TransitionEffects,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    match new {
        Column::Implementing => fx.ensure_running(card).await,
        Column::Testing | Column::Review => {
            // Container stays warm if it was already running; otherwise
            // we don't spawn one. Skipping straight from Backlog to Review
            // is allowed but the user has nothing to run there yet.
            Ok(())
        }
        Column::Done => fx.ensure_stopped(&card.id).await,
        Column::Backlog | Column::Planning => {
            // Defensive: if a container exists from a prior Implementing
            // run, tear it down. Normal flow doesn't go back here, but
            // we tolerate it.
            fx.ensure_stopped(&card.id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockFx {
        running: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl TransitionEffects for MockFx {
        async fn ensure_running(&self, card: &Card) -> Result<()> {
            self.running.lock().await.push(card.id.clone());
            Ok(())
        }
        async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
            self.stopped.lock().await.push(card_id.into());
            Ok(())
        }
    }

    fn sample(id: &str, col: Column) -> Card {
        Card {
            id: id.into(),
            household: "h".into(),
            brief: "b".into(),
            column: col,
            created_at: 0,
            updated_at: 0,
            needs_heavy: false,
            parent_id: None,
        }
    }

    async fn run_one(old: Column, new: Column) -> Arc<MockFx> {
        let fx = Arc::new(MockFx::default());
        let card = sample("c1", new);
        on_column_change(old, new, &card, &*fx).await.unwrap();
        fx
    }

    #[tokio::test]
    async fn backlog_to_implementing_spawns() {
        let fx = run_one(Column::Backlog, Column::Implementing).await;
        assert_eq!(*fx.running.lock().await, vec!["c1".to_string()]);
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_review_is_noop() {
        let fx = run_one(Column::Implementing, Column::Review).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_testing_is_noop() {
        let fx = run_one(Column::Implementing, Column::Testing).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_done_tears_down() {
        let fx = run_one(Column::Implementing, Column::Done).await;
        assert!(fx.running.lock().await.is_empty());
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn review_to_done_tears_down() {
        let fx = run_one(Column::Review, Column::Done).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn done_back_to_backlog_defensively_tears_down() {
        let fx = run_one(Column::Done, Column::Backlog).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn same_column_is_noop() {
        let fx = run_one(Column::Implementing, Column::Implementing).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }
}
