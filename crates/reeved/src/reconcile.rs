//! Startup reconcile pass for container rows.
//!
//! Examines every `Running`/`Spawning` row in the DB, inspects the actual
//! docker state, and either adopts the surviving container or marks it Failed.
//!
//! TODO(stage-4.x): move into `reeve-workspace` once Task 4.3 carves
//! `WorkspaceRepo` (which will own `ContainerRow`). Currently this stays in
//! reeved because it depends on `DaemonState`, `ContainerRow` (in
//! reeve-workspace), and `ListenerRegistry` (transport) — all reeved-internal
//! at this stage.

use crate::app::state::DaemonState;
use crate::transport::listener::ListenerRegistry;
use anyhow::{Context, Result};
use reeve_protocol::{ContainerStatus, paths};
use reeve_workspace::ContainerRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileDecision {
    Adopt,
    Orphan,
}

/// Decide what to do with a `Running`/`Spawning` row given the actual
/// docker state. Pure for testability.
pub fn reconcile_one(_row: &ContainerRow, docker_state: Option<&str>) -> ReconcileDecision {
    match docker_state {
        Some("running") => ReconcileDecision::Adopt,
        _ => ReconcileDecision::Orphan,
    }
}

/// Run the reconcile pass at daemon startup. For each row with status
/// `Running` or `Spawning` in the DB:
///
/// - container is `running` → adopt (re-bind per-card listener)
/// - otherwise → mark `Failed` in DB and unlink stale socket file
///
/// Containers that survived daemon shutdown keep running; their socket
/// files were left on disk and are unlinked here (the listener-bind in
/// `ListenerRegistry::spawn` would do it anyway, but doing it explicitly
/// is clearer in logs).
pub async fn reconcile_on_startup(state: &DaemonState) -> Result<()> {
    let mut rows = state
        .workspaces
        .list_by_status(ContainerStatus::Running)
        .await?;
    rows.extend(
        state
            .workspaces
            .list_by_status(ContainerStatus::Spawning)
            .await?,
    );

    for row in rows {
        // Propagate inspect errors — silently downgrading them to None would
        // misclassify a healthy container as orphaned during a transient
        // docker outage at startup.
        let docker_state = state
            .containers
            .inspect_status(&row.container_name)
            .await
            .with_context(|| format!("reconcile: inspect {}", row.container_name))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match reconcile_one(&row, docker_state.as_deref()) {
            ReconcileDecision::Adopt => {
                let socket = paths::card_socket_path(&row.card_id);
                ListenerRegistry::unlink_stale(&socket);
                state.listeners.spawn(&row.card_id, socket).await;
                // Reconcile Spawning → Running for rows whose write completed
                // just before a crash that didn't kill the container itself.
                if row.status == ContainerStatus::Spawning {
                    state
                        .workspaces
                        .upsert_container(
                            &row.card_id,
                            &row.container_name,
                            ContainerStatus::Running,
                            now,
                        )
                        .await?;
                }
                tracing::info!(card = %row.card_id, "adopted surviving container");
            }
            ReconcileDecision::Orphan => {
                let socket = paths::card_socket_path(&row.card_id);
                ListenerRegistry::unlink_stale(&socket);
                state
                    .workspaces
                    .update_container_status(&row.card_id, ContainerStatus::Failed, now)
                    .await?;
                tracing::warn!(card = %row.card_id, "orphaned container row → Failed");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;
    use reeve_protocol::ContainerStatus;
    use reeve_workspace::ContainerRow;

    fn row(card_id: &str, status: ContainerStatus) -> ContainerRow {
        ContainerRow {
            card_id: card_id.into(),
            container_name: format!("reeve-{card_id}"),
            status,
            started_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn reconcile_running_with_live_container_is_adopt() {
        let decision = reconcile_one(&row("c1", ContainerStatus::Running), Some("running"));
        assert_eq!(decision, ReconcileDecision::Adopt);
    }

    #[test]
    fn reconcile_running_with_no_container_is_orphan() {
        let decision = reconcile_one(&row("c1", ContainerStatus::Running), None);
        assert_eq!(decision, ReconcileDecision::Orphan);
    }

    #[test]
    fn reconcile_running_with_exited_container_is_orphan() {
        let decision = reconcile_one(&row("c1", ContainerStatus::Running), Some("exited"));
        assert_eq!(decision, ReconcileDecision::Orphan);
    }

    #[test]
    fn reconcile_spawning_with_no_container_is_orphan() {
        // Daemon died mid-spawn. We don't know whether docker started
        // it or not; safest is to mark it failed and let the user retry.
        let decision = reconcile_one(&row("c1", ContainerStatus::Spawning), None);
        assert_eq!(decision, ReconcileDecision::Orphan);
    }
}
