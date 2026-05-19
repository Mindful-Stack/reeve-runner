//! Temporary port impls for reeve-card during the staged refactor.
//!
//! - `StubWorkspaceLifecycle` delegates to today's `effects::LiveEffects`.
//!   Replaced by `reeve-workspace`'s real impl in Stage 4.
//!
//! Delete this file after Stage 4.

use crate::effects::{DaemonState, LiveEffects};
use crate::transitions::TransitionEffects;
use anyhow::Result;
use async_trait::async_trait;
use reeve_card::WorkspaceLifecycle;
use reeve_protocol::Card;

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
