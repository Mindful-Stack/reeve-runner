//! Live wiring of `TransitionEffects` for the running daemon.
//! Bundled with the `DaemonState` that the dispatcher closure captures.

use crate::clones::GitOpsHandle;
use crate::container::ContainerOpsHandle;
use crate::db;
use crate::household::HouseholdMap;
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
    pub households: HouseholdMap,
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

        let _household = self
            .state
            .households
            .get(&card.household)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "household '{}' not configured",
                    card.household,
                )
            })?;

        let socket = reeve_protocol::paths::card_socket_path(&card_id);
        let cards_socket_dir = reeve_protocol::paths::cards_socket_dir();
        let _workspace = reeve_protocol::paths::card_workspace_path(&card_id);

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

        // M3.5 Task 9 will rewrite this to:
        //   1. Look up the manifest via load_household_manifest(&household).
        //   2. Use `<workspace>/.devcontainer/devcontainer.json` instead of the dropped
        //      `profile.devcontainer` path.
        //   3. Call clone_all_for_card(...) instead of clone_all_repos(...).
        todo!("M3.5 Task 9 — devcontainer path convention + clone_all_for_card");
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
