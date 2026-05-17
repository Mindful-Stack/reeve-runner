//! Live wiring of `TransitionEffects` for the running daemon.
//! Bundled with the `DaemonState` that the dispatcher closure captures.

use crate::clones::{self, GitOpsHandle};
use crate::container::{self, ContainerOpsHandle};
use crate::db;
use crate::household;
use crate::listeners::ListenerRegistry;
use crate::server::DbHandle;
use crate::transitions::TransitionEffects;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::{Card, ContainerStatus};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Everything the live transition effects need to do their job.
#[derive(Clone)]
pub struct DaemonState {
    pub db: DbHandle,
    pub containers: ContainerOpsHandle,
    pub git: GitOpsHandle,
    pub listeners: ListenerRegistry,
    pub claude_dir: PathBuf,
    pub api_key: Option<String>,
}

pub struct LiveEffects {
    state: DaemonState,
}

impl LiveEffects {
    pub fn new(state: DaemonState) -> Self {
        Self { state }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl TransitionEffects for LiveEffects {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        let card_id = card.id.clone();

        // Query the DB for the household so that households registered after
        // daemon start (via `household new/add`) are visible immediately.
        let household_name = card.household.clone();
        let db = self.state.db.clone();
        let row = tokio::task::spawn_blocking(move || -> Result<Option<db::HouseholdRow>> {
            let conn = db.blocking_lock();
            db::get_household(&conn, &household_name)
        })
        .await??;
        let row =
            row.ok_or_else(|| anyhow::anyhow!("household '{}' not configured", card.household))?;
        let household = household::Household {
            name: row.name,
            meta_repo_path: row.meta_repo_path,
        };

        let socket = reeve_protocol::paths::card_socket_path(&card_id);
        let cards_socket_dir = reeve_protocol::paths::cards_socket_dir();
        let workspace = reeve_protocol::paths::card_workspace_path(&card_id);

        // Ensure the cards SOCKET directory exists so docker bind-mounts the
        // dir (not a fresh empty one). M2 contract.
        std::fs::create_dir_all(&cards_socket_dir)
            .with_context(|| format!("create cards socket dir {}", cards_socket_dir.display()))?;

        // If a container with our label is already running, adopt it and exit.
        // Discovery: the label is unique per card.
        if let Ok(Some(existing)) = self
            .state
            .containers
            .discover_container_name(&card_id)
            .await
        {
            if let Some(state) = self.state.containers.inspect_status(&existing).await? {
                if state == "running" {
                    let id_for_update = card_id.clone();
                    let name_for_update = existing.clone();
                    let db = self.state.db.clone();
                    let now = now_secs();
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        let conn = db.blocking_lock();
                        db::upsert_container(
                            &conn,
                            &id_for_update,
                            &name_for_update,
                            ContainerStatus::Running,
                            now,
                        )
                    })
                    .await??;
                    self.state.listeners.spawn(&card_id, socket).await;
                    return Ok(());
                }
            }
        }

        let manifest = household::load_household_manifest(&household)
            .with_context(|| format!("load manifest for '{}'", household.name))?;

        // Ensure the cards ROOT dir exists. clone_all_for_card refuses to clone
        // into a pre-existing workspace, so we MUST NOT create the workspace itself.
        if let Some(parent) = workspace.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create cards root {}", parent.display()))?;
        }

        clones::clone_all_for_card(&*self.state.git, &household, &manifest, &workspace).await?;

        let devcontainer_path = workspace.join(".devcontainer").join("devcontainer.json");
        if !devcontainer_path.exists() {
            anyhow::bail!(
                "devcontainer.json missing at {} — every witan-household must ship one at the conventional location",
                devcontainer_path.display(),
            );
        }

        let argv = container::devcontainer_up_command(
            &card_id,
            &workspace,
            &devcontainer_path,
            &cards_socket_dir,
            &self.state.claude_dir,
            self.state.api_key.as_deref(),
        );
        self.state.containers.spawn(argv).await?;

        let container_name = discover_with_deadline(&self.state, &card_id).await?;

        let id_for_update = card_id.clone();
        let name_for_update = container_name.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::upsert_container(
                &conn,
                &id_for_update,
                &name_for_update,
                ContainerStatus::Running,
                now,
            )
        })
        .await??;

        self.state.listeners.spawn(&card_id, socket).await;

        // M5: start the per-card claude session inside the container's tmux.
        // Defensive `|| bash` so a broken claude install doesn't lock the user out.
        // Warn-not-bail: if tmux isn't installed in the container, the column
        // transition still succeeds; the user discovers the issue when they
        // run `reeve attach`.
        let tmux_cmd = format!("cd /workspaces/{card_id} && (claude || bash)");
        if let Err(e) = self
            .state
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
        // Tear down listener first so the agent can't issue more calls.
        self.state.listeners.stop(card_id).await?;

        // Look up the container name from the containers row (the name was
        // recorded post-discovery in ensure_running).
        let id_for_lookup = card_id.to_string();
        let db = self.state.db.clone();
        let row = tokio::task::spawn_blocking(move || -> Result<Option<db::ContainerRow>> {
            let conn = db.blocking_lock();
            db::get_container(&conn, &id_for_lookup)
        })
        .await??;

        let name = match row {
            Some(r) => r.container_name,
            None => {
                // Card moved Backlog → Done with no prior Implementing.
                // No container ever existed; nothing to do.
                return Ok(());
            }
        };

        // Container may not actually exist (e.g., daemon crashed mid-spawn
        // and the row is Spawning with a "pending-..." name). Treat absence
        // as "already done".
        if self.state.containers.inspect_status(&name).await?.is_none() {
            let id_for_remove = card_id.to_string();
            let db = self.state.db.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let conn = db.blocking_lock();
                db::remove_container(&conn, &id_for_remove)
            })
            .await??;
            return Ok(());
        }

        self.state.containers.teardown(&name).await?;
        let id_for_remove = card_id.to_string();
        let db = self.state.db.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::remove_container(&conn, &id_for_remove)
        })
        .await??;
        Ok(())
    }
}

/// Poll `containers.discover_container_name` until it returns a name, with a
/// 2-second deadline. Returns `Err` if the deadline expires without a result.
/// `devcontainer up` may return success a moment before docker registers the
/// container, so this poll absorbs the race.
async fn discover_with_deadline(state: &DaemonState, card_id: &str) -> Result<String> {
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(100);
    loop {
        if let Some(name) = state.containers.discover_container_name(card_id).await? {
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
