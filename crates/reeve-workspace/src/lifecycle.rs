//! LiveWorkspaceLifecycle — concrete impl of reeve_card::WorkspaceLifecycle.
//! Composes the workspace's git, container, and repo ports to provision and
//! tear down a card's workspace.

use crate::clones;
use crate::container::{self, ContainerOps};
use crate::git::GitOps;
use crate::repo::WorkspaceRepo;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_card::{Card, WorkspaceLifecycle};
use reeve_protocol::ContainerStatus;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wrapper trait abstracting over reeved::transport::listener::ListenerRegistry
/// so reeve-workspace can hold a handle without depending on reeved.
/// Defined here; production type lives in reeved.
#[async_trait]
pub trait ListenerOps: Send + Sync {
    async fn spawn(&self, card_id: &str, socket: PathBuf);
    async fn stop(&self, card_id: &str) -> Result<()>;
}

pub type ListenerHandle = Arc<dyn ListenerOps>;

pub struct LiveWorkspaceLifecycle {
    workspaces: Arc<dyn WorkspaceRepo>,
    households: Arc<dyn reeve_household::HouseholdRepo>,
    git: Arc<dyn GitOps>,
    containers: Arc<dyn ContainerOps>,
    listeners: ListenerHandle,
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
        claude_dir: PathBuf,
        api_key: Option<String>,
    ) -> Self {
        Self {
            workspaces,
            households,
            git,
            containers,
            listeners,
            claude_dir,
            api_key,
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl WorkspaceLifecycle for LiveWorkspaceLifecycle {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        // Look up the household via the repo port.
        let household_name = card.household().to_string();
        let row = self
            .households
            .get(&household_name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("household '{}' not configured", card.household()))?;
        let household = reeve_household::Household {
            name: row.name,
            meta_repo_path: row.meta_repo_path,
        };

        let card_id = card.id();
        let socket = reeve_protocol::paths::card_socket_path(card_id);
        let cards_socket_dir = reeve_protocol::paths::cards_socket_dir();
        let workspace_path = reeve_protocol::paths::card_workspace_path(card_id);

        std::fs::create_dir_all(&cards_socket_dir)
            .with_context(|| format!("create cards socket dir {}", cards_socket_dir.display()))?;

        // Adopt existing running container if any.
        if let Ok(Some(existing)) = self.containers.discover_container_name(card_id).await {
            if let Some(state) = self.containers.inspect_status(&existing).await? {
                if state == "running" {
                    self.workspaces
                        .upsert_container(card_id, &existing, ContainerStatus::Running, now_secs())
                        .await?;
                    self.listeners.spawn(card_id, socket).await;
                    return Ok(());
                }
            }
        }

        let manifest = reeve_household::load_household_manifest(&household)
            .with_context(|| format!("load manifest for '{}'", household.name))?;

        if let Some(parent) = workspace_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create cards root {}", parent.display()))?;
        }

        clones::clone_all_for_card(&*self.git, &household, &manifest, &workspace_path).await?;

        let devcontainer_path = workspace_path.join(".devcontainer").join("devcontainer.json");
        if !devcontainer_path.exists() {
            anyhow::bail!(
                "devcontainer.json missing at {} — every witan-household must ship one at the conventional location",
                devcontainer_path.display(),
            );
        }

        let argv = container::devcontainer_up_command(
            card_id,
            &workspace_path,
            &devcontainer_path,
            &cards_socket_dir,
            &self.claude_dir,
            self.api_key.as_deref(),
        );
        self.containers.spawn(argv).await?;

        let container_name = discover_with_deadline(&*self.containers, card_id).await?;
        self.workspaces
            .upsert_container(card_id, &container_name, ContainerStatus::Running, now_secs())
            .await?;
        self.listeners.spawn(card_id, socket).await;

        // M5: start the per-card claude session inside the container's tmux.
        // Defensive `|| bash` so a broken claude install doesn't lock the user out.
        // Warn-not-bail: if tmux isn't installed in the container, the column
        // transition still succeeds; the user discovers the issue when they
        // run `reeve attach`.
        let tmux_cmd = format!("cd /workspaces/{card_id} && (claude || bash)");
        if let Err(e) = self
            .containers
            .exec_in_tmux(&container_name, "reeve", &tmux_cmd)
            .await
        {
            tracing::warn!(
                "tmux session not started in {container_name}: {e:#} — \
                 `reeve attach` will fail with 'session not found'; \
                 install tmux in the container (add to devcontainer features)",
            );
        }

        Ok(())
    }

    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        self.listeners.stop(card_id).await?;

        let row = match self.workspaces.get_container(card_id).await? {
            Some(r) => r,
            None => return Ok(()), // never had one
        };

        if self.containers.inspect_status(&row.container_name).await?.is_none() {
            self.workspaces.remove_container(card_id).await?;
            return Ok(());
        }

        self.containers.teardown(&row.container_name).await?;
        self.workspaces.remove_container(card_id).await?;
        Ok(())
    }
}

async fn discover_with_deadline(
    ops: &dyn ContainerOps,
    card_id: &str,
) -> Result<String> {
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(100);
    loop {
        if let Some(name) = ops.discover_container_name(card_id).await? {
            return Ok(name);
        }
        if start.elapsed() >= deadline {
            anyhow::bail!("container not discoverable after devcontainer up");
        }
        tokio::time::sleep(backoff).await;
        if backoff < std::time::Duration::from_millis(400) {
            backoff *= 2;
        }
    }
}
