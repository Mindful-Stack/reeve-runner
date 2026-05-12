//! Live wiring of `TransitionEffects` for the running daemon.
//! Bundled with the `DaemonState` that the dispatcher closure captures.

use crate::container::{self, ContainerOpsHandle};
use crate::db;
use crate::household::HouseholdMap;
use crate::listeners::ListenerRegistry;
use crate::server::DbHandle;
use crate::transitions::TransitionEffects;
use anyhow::Result;
use async_trait::async_trait;
use reeve_protocol::{Card, ContainerStatus};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Everything the live transition effects need to do their job.
#[derive(Clone)]
pub struct DaemonState {
    pub db: DbHandle,
    pub containers: ContainerOpsHandle,
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
        let name = container::container_name(&card_id);

        // Precheck on the host-side (handlers::handle_mv) already verified
        // the household exists. We re-look-up here because TransitionEffects
        // is the boundary the trait gives us.
        let profile = self
            .state
            .households
            .get(&card.household)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "household '{}' not configured (in ~/.config/reeve/households/)",
                    card.household,
                )
            })?;

        let socket = reeve_protocol::paths::card_socket_path(&card_id);

        // Already running? Just ensure the listener is up and return.
        if let Some(state) = self.state.containers.inspect_status(&name).await? {
            if state == "running" {
                self.state.listeners.spawn(&card_id, socket).await;
                return Ok(());
            }
        }

        // Record Spawning before launching docker.
        let id_for_record = card_id.clone();
        let name_for_record = name.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::upsert_container(
                &conn,
                &id_for_record,
                &name_for_record,
                ContainerStatus::Spawning,
                now,
            )
        })
        .await??;

        let argv = container::spawn_command(
            &card_id,
            &profile,
            &socket,
            &self.state.claude_dir,
            self.state.api_key.as_deref(),
        );
        let spawn_result = self.state.containers.spawn(argv).await;

        let new_status = match &spawn_result {
            Ok(()) => ContainerStatus::Running,
            Err(_) => ContainerStatus::Failed,
        };
        let id_for_update = card_id.clone();
        let db = self.state.db.clone();
        let now = now_secs();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = db.blocking_lock();
            db::update_container_status(&conn, &id_for_update, new_status, now)
        })
        .await??;
        spawn_result?;

        // Bring up the per-card listener so the agent can call back.
        self.state.listeners.spawn(&card_id, socket).await;
        Ok(())
    }

    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        // Tear down listener first so the agent can't issue more calls.
        self.state.listeners.stop(card_id).await?;

        let name = container::container_name(card_id);
        // Container may not exist (card moved Backlog → Done with no prior
        // Implementing). That's fine — just remove the row if present.
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
