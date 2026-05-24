//! DaemonState — composition of port handles passed through dispatch.

use crate::clones::GitOpsHandle;
use crate::transport::listener::ListenerRegistry;
use crate::transport::server::DbHandle;
use anyhow::Result;
use async_trait::async_trait;
use reeve_workspace::ContainerOpsHandle;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    pub progress: Arc<crate::app::progress::InMemoryProgress>,
    /// Per-card serialization for column-state-changing operations
    /// (mv, mv-self, delete). The outer Mutex guards the HashMap;
    /// the inner Arc<Mutex<()>> is per-card. Acquire the outer briefly
    /// to get-or-insert the inner, then drop outer while holding inner.
    pub card_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl DaemonState {
    /// Acquire the per-card serialization lock. Returns an owned guard
    /// that releases on drop. Use this to serialize column-state-changing
    /// RPCs (mv, mv-self, delete) per card.
    pub async fn lock_card(&self, card_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let card_mutex = {
            let mut map = self.card_locks.lock().await;
            map.entry(card_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        // Outer guard dropped; we now hold only the per-card inner mutex.
        card_mutex.lock_owned().await
    }
}

/// Implement `FetchDeps` for `DaemonState` so the fetch worker (now in
/// `reeve-workspace`) can drive the live daemon state without a circular dep.
#[async_trait]
impl reeve_workspace::fetch::FetchDeps for DaemonState {
    async fn list_running_cards(&self) -> Result<Vec<reeve_workspace::fetch::RunningCard>> {
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
                    card_id: card.id().to_string(),
                    household: card.household().to_string(),
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
