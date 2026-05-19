//! Kanban transition policy + ports the Card aggregate needs from
//! other aggregates. Pure decision logic — no infra deps.

use anyhow::Result;
use async_trait::async_trait;
use crate::domain::Card;
use reeve_protocol::Column;

/// Port the Card aggregate uses to drive workspace state.
/// Implemented in `reeve-workspace`.
#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

/// Port the Card aggregate uses to check household existence before
/// a transition that needs one. Implemented in `reeve-household`.
#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
}

/// The kanban transition matrix. Decides which `WorkspaceLifecycle`
/// method (if any) to invoke when a card's column changes.
pub async fn on_column_change(
    old: Column,
    new: Column,
    card: &Card,
    workspace: &dyn WorkspaceLifecycle,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    match (old, new) {
        (_, Column::Implementing) => workspace.ensure_running(card).await,
        (_, Column::Testing | Column::Review) => Ok(()),
        (_, Column::Done) => workspace.ensure_stopped(card.id()).await,
        (
            Column::Implementing | Column::Testing | Column::Review,
            Column::Backlog | Column::Planning,
        ) => workspace.ensure_stopped(card.id()).await,
        (_, Column::Backlog | Column::Planning) => Ok(()),
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
    impl WorkspaceLifecycle for MockFx {
        async fn ensure_running(&self, card: &Card) -> Result<()> {
            self.running.lock().await.push(card.id().to_string());
            Ok(())
        }
        async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
            self.stopped.lock().await.push(card_id.into());
            Ok(())
        }
    }

    fn sample(id: &str, col: Column) -> Card {
        Card::from_persisted(
            id.into(),
            "h".into(),
            "b".into(),
            col,
            0,
            0,
            false,
            None,
        )
        .unwrap()
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
    async fn implementing_to_done_tears_down() {
        let fx = run_one(Column::Implementing, Column::Done).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn implementing_to_review_is_noop() {
        let fx = run_one(Column::Implementing, Column::Review).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn same_column_is_noop() {
        let fx = run_one(Column::Implementing, Column::Implementing).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }

    #[tokio::test]
    async fn implementing_to_backlog_defensively_tears_down() {
        let fx = run_one(Column::Implementing, Column::Backlog).await;
        assert_eq!(*fx.stopped.lock().await, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn backlog_to_planning_is_noop() {
        let fx = run_one(Column::Backlog, Column::Planning).await;
        assert!(fx.running.lock().await.is_empty());
        assert!(fx.stopped.lock().await.is_empty());
    }
}
