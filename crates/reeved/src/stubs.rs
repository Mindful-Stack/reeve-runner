//! Temporary port impls for reeve-card during the staged refactor.
//!
//! - `StubWorkspaceLifecycle` delegates to today's `effects::LiveEffects`.
//!   Replaced by `reeve-workspace`'s real impl in Stage 4.
//! - `StubHouseholdLookup` queries the households table directly.
//!   Replaced by `reeve-household`'s real impl in Stage 3.
//!
//! Delete this file after Stage 4.

use crate::db;
use crate::effects::{DaemonState, LiveEffects};
use crate::transitions::TransitionEffects;
use anyhow::Result;
use async_trait::async_trait;
use reeve_card::{HouseholdLookup, WorkspaceLifecycle};
use reeve_protocol::Card;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct StubWorkspaceLifecycle {
    pub state: DaemonState,
}

#[async_trait]
impl WorkspaceLifecycle for StubWorkspaceLifecycle {
    async fn ensure_running(&self, card: &Card) -> Result<()> {
        let fx = LiveEffects::new(self.state.clone());
        TransitionEffects::ensure_running(&fx, card).await
    }

    async fn ensure_stopped(&self, card_id: &str) -> Result<()> {
        let fx = LiveEffects::new(self.state.clone());
        TransitionEffects::ensure_stopped(&fx, card_id).await
    }
}

pub struct StubHouseholdLookup {
    pub db: Arc<Mutex<Connection>>,
}

#[async_trait]
impl HouseholdLookup for StubHouseholdLookup {
    async fn exists(&self, name: &str) -> Result<bool> {
        let db = self.db.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = db.blocking_lock();
            Ok(db::get_household(&conn, &name)?.is_some())
        })
        .await?
    }
}
