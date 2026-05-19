//! DaemonState and its port impls.

use crate::clones::GitOpsHandle;
use reeve_workspace::ContainerOpsHandle;
use crate::transport::listener::ListenerRegistry;
use crate::transport::server::DbHandle;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

/// Everything the daemon needs to serve requests.
#[derive(Clone)]
pub struct DaemonState {
    pub db: DbHandle,
    pub containers: ContainerOpsHandle,
    pub git: GitOpsHandle,
    pub listeners: ListenerRegistry,
    pub claude_dir: PathBuf,
    pub api_key: Option<String>,
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
        // First fetch all container rows, then look up each card to get its
        // household name.
        let db = self.db.clone();
        let rows = tokio::task::spawn_blocking(move || -> Result<Vec<crate::db::ContainerRow>> {
            let conn = db.blocking_lock();
            crate::db::list_containers(&conn)
        })
        .await??;

        let running_ids: Vec<String> = rows
            .into_iter()
            .filter(|r| {
                matches!(
                    r.status,
                    ContainerStatus::Running | ContainerStatus::Spawning
                )
            })
            .map(|r| r.card_id)
            .collect();

        let mut cards = Vec::with_capacity(running_ids.len());
        for card_id in running_ids {
            let db = self.db.clone();
            let id = card_id.clone();
            let card = tokio::task::spawn_blocking(move || -> Result<Option<reeve_protocol::Card>> {
                let conn = db.blocking_lock();
                crate::db::get_card(&conn, &id)
            })
            .await??;
            if let Some(card) = card {
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
