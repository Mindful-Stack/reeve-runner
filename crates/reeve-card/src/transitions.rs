//! Kanban transition policy + ports the Card aggregate needs from
//! other aggregates. Pure decision logic — no infra deps.

use crate::domain::Card;
use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::Column;

/// Port the Card aggregate uses to drive workspace state.
/// Implemented in `reeve-workspace`.
#[async_trait]
pub trait WorkspaceLifecycle: Send + Sync {
    async fn ensure_running(&self, card: &Card) -> Result<()>;
    async fn ensure_stopped(&self, card_id: &str) -> Result<()>;
}

/// Resolved per-card output paths for the planning agent. Returned by
/// `HouseholdLookup::card_paths`. Plugin-agnostic defaults are applied
/// here so consumers (the session-init handler) don't need to know
/// the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPaths {
    /// Relative to the meta-repo clone — where spec markdown lives.
    /// Default `docs/agent/specs`.
    pub spec_path: String,
    /// Relative to the meta-repo clone — where plan markdown lives.
    /// Default `docs/agent/plans`.
    pub plan_path: String,
}

impl CardPaths {
    pub const DEFAULT_SPEC_PATH: &'static str = "docs/agent/specs";
    pub const DEFAULT_PLAN_PATH: &'static str = "docs/agent/plans";

    /// Apply defaults to optional manifest fields.
    pub fn from_optional(spec: Option<&str>, plan: Option<&str>) -> Self {
        Self {
            spec_path: spec.unwrap_or(Self::DEFAULT_SPEC_PATH).to_string(),
            plan_path: plan.unwrap_or(Self::DEFAULT_PLAN_PATH).to_string(),
        }
    }
}

/// Port the Card aggregate uses to check household existence and read
/// the card-output paths a household declares. Implemented in
/// `reeve-household`.
#[async_trait]
pub trait HouseholdLookup: Send + Sync {
    async fn exists(&self, name: &str) -> Result<bool>;
    /// Resolve the household's `spec_path`/`plan_path` from its manifest,
    /// applying `CardPaths` defaults where the manifest leaves them unset.
    /// Returns Err if the household is not registered or its manifest
    /// cannot be loaded.
    async fn card_paths(&self, name: &str) -> Result<CardPaths>;
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
        Card::from_persisted(id.into(), "h".into(), "b".into(), col, 0, 0, false, None).unwrap()
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

    #[test]
    fn card_paths_defaults_when_both_absent() {
        let p = CardPaths::from_optional(None, None);
        assert_eq!(p.spec_path, "docs/agent/specs");
        assert_eq!(p.plan_path, "docs/agent/plans");
    }

    #[test]
    fn card_paths_uses_provided_values() {
        let p = CardPaths::from_optional(
            Some("docs/superpowers/specs"),
            Some("docs/superpowers/plans"),
        );
        assert_eq!(p.spec_path, "docs/superpowers/specs");
        assert_eq!(p.plan_path, "docs/superpowers/plans");
    }

    #[test]
    fn card_paths_mixed_provided_and_default() {
        let p = CardPaths::from_optional(Some("custom/specs"), None);
        assert_eq!(p.spec_path, "custom/specs");
        assert_eq!(p.plan_path, "docs/agent/plans");
    }
}
