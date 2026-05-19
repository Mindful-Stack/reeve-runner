//! DaemonState — composition of port handles passed through dispatch.

use crate::clones::GitOpsHandle;
use crate::transport::listener::ListenerRegistry;
use crate::transport::server::DbHandle;
use anyhow::Result;
use async_trait::async_trait;
use reeve_workspace::ContainerOpsHandle;
use std::sync::Arc;

/// Everything the daemon needs to serve requests.
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
}

/// Implement `FetchDeps` for `DaemonState` so the fetch worker (now in
/// `reeve-workspace`) can drive the live daemon state without a circular dep.
#[async_trait]
impl reeve_workspace::fetch::FetchDeps for DaemonState {
    async fn list_running_cards(
        &self,
    ) -> Result<Vec<reeve_workspace::fetch::RunningCard>> {
        use reeve_protocol::ContainerStatus;

        let mut rows = self
            .workspaces
            .list_by_status(ContainerStatus::Running)
            .await?;
        rows.extend(
            self.workspaces
                .list_by_status(ContainerStatus::Spawning)
                .await?,
        );

        let mut cards = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(card) = self.cards.get(&row.card_id).await? {
                cards.push(reeve_workspace::fetch::RunningCard {
                    card_id: card.id,
                    household: card.household,
                });
            }
        }
        Ok(cards)
    }

    async fn get_household(
        &self,
        name: &str,
    ) -> Result<Option<reeve_workspace::fetch::HouseholdInfo>> {
        let row = self.household_repo.get(name).await?;
        Ok(row.map(|r| reeve_workspace::fetch::HouseholdInfo {
            name: r.name,
            meta_repo_path: r.meta_repo_path,
        }))
    }

    fn git_ops(&self) -> reeve_workspace::GitOpsHandle {
        self.git.clone()
    }
}
