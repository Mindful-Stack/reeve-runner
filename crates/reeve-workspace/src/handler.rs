//! RPC handlers for the Workspace aggregate.

use crate::container::ContainerOps;
use crate::git::GitOps;
use reeve_card::CardRepo;
use reeve_protocol::{
    AttachParams, AttachResult, BranchPushResult, BranchPushStatus, PushParams, PushResult,
    RpcError,
};
use serde_json::Value;

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

pub async fn handle_attach(
    cards: &dyn CardRepo,
    containers: &dyn ContainerOps,
    params: AttachParams,
) -> std::result::Result<Value, RpcError> {
    // 1. Fetch the card.
    let card = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card '{}' not found", params.id),
        })?;

    // 2. Column check — container is present in any active column.
    if !card.column().is_active() {
        return Err(RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: format!(
                "card is in {}; attach only works in active columns (Planning, Implementing, Testing, Review)",
                card.column().label()
            ),
        });
    }

    // 3. Container discovery by label.
    let container_name = containers
        .discover_container_name(card.id())
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("container discovery failed: {e:#}"),
        })?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!(
                "container for card '{}' not running (try mv {} implementing again)",
                card.id(),
                card.id()
            ),
        })?;

    // 4. Return the address.
    let result = AttachResult {
        container_name,
        session_name: "reeve".to_string(),
        in_container_workspace: format!("/workspaces/{}", card.id()),
    };
    serde_json::to_value(result).map_err(internal)
}

pub async fn handle_push(
    cards: &dyn CardRepo,
    households: &dyn reeve_household::HouseholdRepo,
    git: &dyn GitOps,
    params: PushParams,
) -> std::result::Result<Value, RpcError> {
    // 1. Fetch the card from the DB.
    let card = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card '{}' not found", params.id),
        })?;

    // 2. Resolve the household from the household_repo.
    let row = households
        .get(card.household())
        .await
        .map_err(internal)?
        .ok_or_else(|| RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", card.household()),
        })?;
    let household = reeve_household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };

    // 3. Load + validate the manifest.
    let manifest = reeve_household::load_household_manifest(&household).map_err(|e| RpcError {
        code: RpcError::INTERNAL,
        message: format!("load manifest: {e:#}"),
    })?;

    // 4. For each repo, for each branch in the namespace, FF + push.
    let mut branches = Vec::new();
    for (repo_name, host_path) in reeve_household::iter_card_repos(&household, &manifest) {
        // Inline entries (subdirs of meta-repo without their own .git/, e.g.
        // a `lore/` knowledge-base directory) aren't independently pushable.
        // Skip them — they travel with the meta-repo and ship via its push.
        if !host_path.join(".git").exists() {
            continue;
        }
        let listed = git
            .list_namespace_refs(&host_path, card.id())
            .await
            .map_err(|e| RpcError {
                code: RpcError::INTERNAL,
                message: format!(
                    "list refs/reeve/{}/* in {}: {e:#}",
                    card.id(),
                    host_path.display()
                ),
            })?;
        for branch in listed {
            let target = format!("refs/heads/{branch}");
            let source = format!("refs/reeve/{}/{}", card.id(), branch);
            let status = match git.fast_forward(&host_path, &target, &source).await {
                Ok(crate::git::FfOutcome::Updated) => match git.push(&host_path, &branch).await {
                    Ok(()) => BranchPushStatus::Pushed,
                    Err(e) => BranchPushStatus::PushFailed {
                        message: format!("{e:#}"),
                    },
                },
                Ok(crate::git::FfOutcome::NotFastForward) => BranchPushStatus::NotFastForward,
                Err(e) => BranchPushStatus::PushFailed {
                    message: format!("fast-forward check failed: {e:#}"),
                },
            };
            branches.push(BranchPushResult {
                repo: repo_name.to_string(),
                branch,
                status,
            });
        }
    }
    serde_json::to_value(PushResult { branches }).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use reeve_card::{Card, CardRepo};
    use reeve_protocol::{CardReference, Column};
    use std::sync::Mutex;

    /// Minimal in-memory CardRepo holding at most one card.
    struct OneCardRepo {
        card: Mutex<Option<reeve_card::Card>>,
    }
    impl OneCardRepo {
        fn new(card: reeve_card::Card) -> Self {
            Self {
                card: Mutex::new(Some(card)),
            }
        }
    }
    #[async_trait]
    impl CardRepo for OneCardRepo {
        async fn get(&self, _id: &str) -> Result<Option<Card>> {
            Ok(self.card.lock().unwrap().clone())
        }
        async fn list(&self, _household: Option<&str>) -> Result<Vec<Card>> {
            Ok(self.card.lock().unwrap().iter().cloned().collect())
        }
        async fn insert(&self, _card: &Card) -> Result<()> {
            Ok(())
        }
        async fn update_column(&self, _id: &str, _col: Column, _now: i64) -> Result<Option<Card>> {
            Ok(None)
        }
        async fn update_brief(&self, _id: &str, _brief: &str, _now: i64) -> Result<Option<Card>> {
            Ok(None)
        }
        async fn delete(&self, _id: &str) -> Result<bool> {
            Ok(true)
        }
        async fn for_household(&self, _name: &str) -> Result<Vec<CardReference>> {
            Ok(vec![])
        }
    }

    /// ContainerOps that always returns a fixed container name.
    struct FoundContainer {
        name: String,
    }
    #[async_trait]
    impl ContainerOps for FoundContainer {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> {
            Ok(())
        }
        async fn teardown(&self, _container_name: &str) -> Result<()> {
            Ok(())
        }
        async fn inspect_status(&self, _container_name: &str) -> Result<Option<String>> {
            Ok(Some("running".into()))
        }
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> {
            Ok(Some(self.name.clone()))
        }
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> {
            Ok(())
        }
    }

    /// ContainerOps whose discover_container_name returns None (no container running).
    struct NoContainer;
    #[async_trait]
    impl ContainerOps for NoContainer {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> {
            Ok(())
        }
        async fn teardown(&self, _container_name: &str) -> Result<()> {
            Ok(())
        }
        async fn inspect_status(&self, _container_name: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn exec_in_tmux(
            &self,
            _container_name: &str,
            _session_name: &str,
            _command: &str,
        ) -> Result<()> {
            Ok(())
        }
    }

    fn planning_card() -> Card {
        // Create in Backlog first (only constructor), then we expose via repo
        // with column set via from_persisted.
        Card::from_persisted(
            "plan-id-1".into(),
            "household".into(),
            "some work".into(),
            Column::Planning,
            0,
            0,
            false,
            None,
        )
        .unwrap()
    }

    fn backlog_card() -> Card {
        Card::from_persisted(
            "backlog-id-1".into(),
            "household".into(),
            "not started".into(),
            Column::Backlog,
            0,
            0,
            false,
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn attach_accepts_planning_column() {
        let card = planning_card();
        let repo = OneCardRepo::new(card.clone());
        let containers = FoundContainer {
            name: "my-container".into(),
        };
        let params = AttachParams {
            id: card.id().to_string(),
        };
        let result = handle_attach(&repo, &containers, params).await;
        assert!(
            result.is_ok(),
            "Planning card should be accepted by attach, got: {:?}",
            result
        );
        let val: AttachResult = serde_json::from_value(result.unwrap()).unwrap();
        assert_eq!(val.container_name, "my-container");
    }

    #[tokio::test]
    async fn attach_rejects_backlog_column() {
        let card = backlog_card();
        let repo = OneCardRepo::new(card.clone());
        let containers = NoContainer;
        let params = AttachParams {
            id: card.id().to_string(),
        };
        let err = handle_attach(&repo, &containers, params).await.unwrap_err();
        assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
        assert!(
            err.message.contains("active columns"),
            "error message should mention 'active columns', got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn attach_rejects_done_column() {
        let card = Card::from_persisted(
            "done-id-1".into(),
            "household".into(),
            "finished".into(),
            Column::Done,
            0,
            0,
            false,
            None,
        )
        .unwrap();
        let repo = OneCardRepo::new(card.clone());
        let containers = NoContainer;
        let params = AttachParams {
            id: card.id().to_string(),
        };
        let err = handle_attach(&repo, &containers, params).await.unwrap_err();
        assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
    }
}
