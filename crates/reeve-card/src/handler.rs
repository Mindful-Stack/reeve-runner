//! RPC handlers for the Card aggregate.

use crate::domain::Card;
use crate::repo::CardRepo;
use crate::transitions::{HouseholdLookup, WorkspaceLifecycle, on_column_change};
use reeve_protocol::{
    EditParams, EditResult, LsParams, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult,
    NewParams, NewResult, RpcError, StatusSelfResult,
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

fn not_found(id: &str) -> RpcError {
    RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {id}"),
    }
}

pub async fn handle_ls(
    cards: &dyn CardRepo,
    params: LsParams,
) -> std::result::Result<Value, RpcError> {
    let entities = cards
        .list(params.household.as_deref())
        .await
        .map_err(internal)?;
    let dtos: Vec<reeve_protocol::Card> = entities.into_iter().map(Into::into).collect();
    serde_json::to_value(LsResult { cards: dtos }).map_err(internal)
}

pub async fn handle_new(
    cards: &dyn CardRepo,
    params: NewParams,
) -> std::result::Result<Value, RpcError> {
    let card =
        Card::new(params.household, params.brief, params.needs_heavy).map_err(|e| RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: e.to_string(),
        })?;
    cards.insert(&card).await.map_err(internal)?;
    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(NewResult { card: dto }).map_err(internal)
}

pub async fn handle_mv(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    workspace: &dyn WorkspaceLifecycle,
    params: MvParams,
) -> std::result::Result<Value, RpcError> {
    let existing = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;
    let old_column = existing.column();

    // Friendly error before lifecycle if the transition is Inactive→Active
    // and the household isn't registered. Without this, the user sees an
    // opaque container-provisioning failure instead of a clear message.
    if crate::transitions::is_active(params.column)
        && !crate::transitions::is_active(old_column)
        && !households
            .exists(existing.household())
            .await
            .map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household()),
        });
    }

    // Run lifecycle BEFORE committing the column change. If ensure_running
    // (or ensure_stopped) fails, the card stays in its old column and the
    // caller sees an accurate error — instead of the card being stranded
    // with a column change the side-effects never materialised for.
    on_column_change(old_column, params.column, &existing, workspace)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;

    let now = now_secs();
    let card = cards
        .update_column(&params.id, params.column, now)
        .await
        .map_err(|e| {
            tracing::error!(
                "lifecycle succeeded but update_column failed for {}: {e:#} — \
                 the side-effects (container spawn/teardown) already ran; \
                 manual reconciliation may be required",
                params.id,
            );
            internal(e)
        })?
        .ok_or_else(|| not_found(&params.id))?;

    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(MvResult { card: dto }).map_err(internal)
}

pub async fn handle_mv_self(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    workspace: &dyn WorkspaceLifecycle,
    card_id: String,
    params: MvSelfParams,
) -> std::result::Result<Value, RpcError> {
    let existing = cards
        .get(&card_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;
    let old_column = existing.column();

    // Friendly error before lifecycle if the transition is Inactive→Active
    // and the household isn't registered. Without this, the user sees an
    // opaque container-provisioning failure instead of a clear message.
    if crate::transitions::is_active(params.column)
        && !crate::transitions::is_active(old_column)
        && !households
            .exists(existing.household())
            .await
            .map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household()),
        });
    }

    // See `handle_mv` — lifecycle runs before column commit so a failing
    // ensure_running/ensure_stopped leaves the card in its old column.
    on_column_change(old_column, params.column, &existing, workspace)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;

    let now = now_secs();
    let card = cards
        .update_column(&card_id, params.column, now)
        .await
        .map_err(|e| {
            tracing::error!(
                "lifecycle succeeded but update_column failed for {card_id}: {e:#} — \
                 the side-effects (container spawn/teardown) already ran; \
                 manual reconciliation may be required",
            );
            internal(e)
        })?
        .ok_or_else(|| not_found(&card_id))?;

    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(MvSelfResult { card: dto }).map_err(internal)
}

pub async fn handle_status_self(
    cards: &dyn CardRepo,
    workspace_status: Option<reeve_protocol::ContainerStatus>,
    card_id: String,
) -> std::result::Result<Value, RpcError> {
    let card = cards
        .get(&card_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;
    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(StatusSelfResult {
        card: dto,
        container_status: workspace_status,
    })
    .map_err(internal)
}

pub async fn handle_session_init_self(
    cards: &dyn CardRepo,
    households: &dyn HouseholdLookup,
    card_id: String,
) -> std::result::Result<Value, RpcError> {
    let card = cards
        .get(&card_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;
    let paths = households
        .card_paths(card.household())
        .await
        .map_err(internal)?;
    let prompt = crate::session_init::render(&card, &paths);
    serde_json::to_value(reeve_protocol::SessionInitSelfResult { prompt }).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{init_schema, make_card_repo};
    use reeve_protocol::Column;
    use rusqlite::Connection;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn fresh_repo() -> Arc<dyn CardRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_card_repo(Arc::new(Mutex::new(conn)))
    }

    /// Mock WorkspaceLifecycle that records ensure_stopped calls.
    struct RecordingLifecycle {
        stopped: std::sync::Mutex<Vec<String>>,
    }
    impl RecordingLifecycle {
        fn new() -> Self {
            Self {
                stopped: std::sync::Mutex::new(vec![]),
            }
        }
    }
    #[async_trait::async_trait]
    impl crate::WorkspaceLifecycle for RecordingLifecycle {
        async fn ensure_running(&self, _card: &Card) -> anyhow::Result<()> {
            Ok(())
        }
        async fn ensure_stopped(&self, card_id: &str) -> anyhow::Result<()> {
            self.stopped.lock().unwrap().push(card_id.to_string());
            Ok(())
        }
    }

    /// Mock WorkspaceLifecycle whose ensure_running always errors.
    struct FailingLifecycle;
    #[async_trait::async_trait]
    impl crate::WorkspaceLifecycle for FailingLifecycle {
        async fn ensure_running(&self, _card: &Card) -> anyhow::Result<()> {
            anyhow::bail!("simulated lifecycle failure")
        }
        async fn ensure_stopped(&self, _card_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// Mock HouseholdLookup that always reports the household exists.
    struct AlwaysExistsHouseholds;
    #[async_trait::async_trait]
    impl crate::HouseholdLookup for AlwaysExistsHouseholds {
        async fn exists(&self, _name: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn card_paths(&self, _name: &str) -> anyhow::Result<crate::CardPaths> {
            Ok(crate::CardPaths::from_optional(None, None))
        }
    }

    #[tokio::test]
    async fn delete_implementing_card_tears_down_container_then_removes_row() {
        let repo = fresh_repo();
        let lifecycle = RecordingLifecycle::new();
        let card = Card::new("h".into(), "doomed".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();
        repo.update_column(&id, Column::Implementing, 0)
            .await
            .unwrap();

        let req = reeve_protocol::DeleteParams { id: id.clone() };
        handle_delete(&*repo, &lifecycle, req).await.unwrap();

        let stopped = lifecycle.stopped.lock().unwrap();
        assert_eq!(stopped.as_slice(), &[id.clone()]);
        assert!(repo.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_backlog_card_skips_teardown() {
        let repo = fresh_repo();
        let lifecycle = RecordingLifecycle::new();
        let card = Card::new("h".into(), "ephemeral".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();
        // Card stays in Backlog — no column update needed.

        let req = reeve_protocol::DeleteParams { id: id.clone() };
        handle_delete(&*repo, &lifecycle, req).await.unwrap();

        // ensure_stopped must NOT have been called.
        assert!(lifecycle.stopped.lock().unwrap().is_empty());
        assert!(repo.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_planning_card_tears_down_container() {
        let repo = fresh_repo();
        let lifecycle = RecordingLifecycle::new();
        let card = Card::new("h".into(), "doomed".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();
        repo.update_column(&id, Column::Planning, 0).await.unwrap();

        let req = reeve_protocol::DeleteParams { id: id.clone() };
        handle_delete(&*repo, &lifecycle, req).await.unwrap();

        let stopped = lifecycle.stopped.lock().unwrap();
        assert_eq!(stopped.as_slice(), &[id.clone()]);
        assert!(repo.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_testing_card_tears_down_container() {
        let repo = fresh_repo();
        let lifecycle = RecordingLifecycle::new();
        let card = Card::new("h".into(), "doomed-testing".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();
        repo.update_column(&id, Column::Testing, 0).await.unwrap();

        let req = reeve_protocol::DeleteParams { id: id.clone() };
        handle_delete(&*repo, &lifecycle, req).await.unwrap();

        let stopped = lifecycle.stopped.lock().unwrap();
        assert_eq!(stopped.as_slice(), &[id.clone()]);
        assert!(repo.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_returns_not_found_for_unknown_card() {
        let repo = fresh_repo();
        let lifecycle = RecordingLifecycle::new();
        let req = reeve_protocol::DeleteParams {
            id: "no-such-id".into(),
        };
        let err = handle_delete(&*repo, &lifecycle, req).await.unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn edit_updates_brief_and_returns_card() {
        let repo = fresh_repo();
        let card = Card::new("h".into(), "old brief".into(), false).unwrap();
        let card_id = card.id().to_string();
        repo.insert(&card).await.unwrap();

        let req = EditParams {
            id: card_id.clone(),
            brief: "new brief".into(),
        };
        let value = handle_edit(&*repo, req).await.unwrap();
        let result: EditResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.card.brief, "new brief");
        assert_eq!(result.card.id, card_id);
    }

    #[tokio::test]
    async fn edit_returns_not_found_for_unknown_card() {
        let repo = fresh_repo();
        let req = EditParams {
            id: "no-such-card".into(),
            brief: "anything".into(),
        };
        let err = handle_edit(&*repo, req).await.unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn mv_to_implementing_with_failing_lifecycle_leaves_column_unchanged() {
        // Regression test: handle_mv must run on_column_change BEFORE
        // committing the column update, so that a failing ensure_running
        // doesn't strand the card in the new column without the
        // accompanying side-effects (container spawn).
        let repo = fresh_repo();
        let card = Card::new("h".into(), "test".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();
        // Card starts in Backlog (the default for new()).

        let req = MvParams {
            id: id.clone(),
            column: Column::Implementing,
        };
        let err = handle_mv(&*repo, &AlwaysExistsHouseholds, &FailingLifecycle, req)
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(
            err.message.contains("simulated lifecycle failure"),
            "error should surface the lifecycle failure: {}",
            err.message,
        );

        // The card must still be in Backlog — the column update was NOT
        // committed because lifecycle ran first and failed.
        let after = repo.get(&id).await.unwrap().unwrap();
        assert_eq!(after.column(), Column::Backlog);
    }

    #[tokio::test]
    async fn handle_ls_filters_by_household() {
        let repo = fresh_repo();
        let a = Card::new("h1".into(), "a".into(), false).unwrap();
        let b = Card::new("h2".into(), "b".into(), false).unwrap();
        repo.insert(&a).await.unwrap();
        repo.insert(&b).await.unwrap();

        let value = handle_ls(
            &*repo,
            LsParams {
                household: Some("h1".into()),
            },
        )
        .await
        .unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.cards.len(), 1);
        assert_eq!(result.cards[0].household, "h1");

        let value = handle_ls(&*repo, LsParams { household: None })
            .await
            .unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.cards.len(), 2);
    }

    #[tokio::test]
    async fn edit_rejects_empty_brief() {
        let repo = fresh_repo();
        let card = Card::new("h".into(), "old".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();
        let req = EditParams {
            id,
            brief: "".into(),
        };
        let err = handle_edit(&*repo, req).await.unwrap_err();
        assert_eq!(err.code, RpcError::INVALID_ARGUMENT);
    }

    #[tokio::test]
    async fn session_init_self_renders_prompt_from_card_and_default_paths() {
        let repo = fresh_repo();
        let card = Card::new("witan".into(), "Add OAuth login".into(), false).unwrap();
        let card_id = card.id().to_string();
        repo.insert(&card).await.unwrap();

        let value = handle_session_init_self(&*repo, &AlwaysExistsHouseholds, card_id.clone())
            .await
            .unwrap();
        let result: reeve_protocol::SessionInitSelfResult = serde_json::from_value(value).unwrap();
        assert!(result.prompt.contains(&format!("reeve card {card_id}")));
        assert!(result.prompt.contains("in the witan household"));
        assert!(result.prompt.contains("Brief: Add OAuth login"));
        assert!(
            result
                .prompt
                .contains(&format!("docs/agent/specs/{card_id}-design.md"))
        );
        assert!(
            result
                .prompt
                .contains(&format!("docs/agent/plans/{card_id}.md"))
        );
    }

    #[tokio::test]
    async fn session_init_self_returns_not_found_for_unknown_card() {
        let repo = fresh_repo();
        let err = handle_session_init_self(&*repo, &AlwaysExistsHouseholds, "no-such-card".into())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn mv_to_planning_with_missing_household_returns_friendly_error() {
        // Verify the Task-4 follow-up: the household-existence gate fires for
        // Inactive→Active transitions to ANY active column, not just Implementing.
        struct MissingHouseholds;
        #[async_trait::async_trait]
        impl crate::HouseholdLookup for MissingHouseholds {
            async fn exists(&self, _name: &str) -> anyhow::Result<bool> {
                Ok(false)
            }
            async fn card_paths(&self, _name: &str) -> anyhow::Result<crate::CardPaths> {
                Ok(crate::CardPaths::from_optional(None, None))
            }
        }

        let repo = fresh_repo();
        let lifecycle = RecordingLifecycle::new();
        let card = Card::new("ghost".into(), "test".into(), false).unwrap();
        let id = card.id().to_string();
        repo.insert(&card).await.unwrap();

        let req = MvParams {
            id: id.clone(),
            column: Column::Planning,
        };
        let err = handle_mv(&*repo, &MissingHouseholds, &lifecycle, req)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("'ghost' not configured"),
            "expected friendly household error, got: {}",
            err.message,
        );
        // Card must still be in Backlog — the column was never committed.
        let after = repo.get(&id).await.unwrap().unwrap();
        assert_eq!(after.column(), Column::Backlog);
    }
}

pub async fn handle_delete(
    cards: &dyn CardRepo,
    workspace: &dyn crate::WorkspaceLifecycle,
    params: reeve_protocol::DeleteParams,
) -> std::result::Result<Value, RpcError> {
    let existing = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;

    if existing.column().is_active() {
        workspace
            .ensure_stopped(existing.id())
            .await
            .map_err(|e| RpcError {
                code: RpcError::INTERNAL,
                message: format!("teardown before delete: {e:#}"),
            })?;
    }

    let removed = cards.delete(&params.id).await.map_err(internal)?;
    if !removed {
        // Race: card was deleted between get() and delete(). Treat as success.
    }
    Ok(Value::Null)
}

pub async fn handle_edit(
    cards: &dyn CardRepo,
    params: EditParams,
) -> std::result::Result<Value, RpcError> {
    let existing = cards
        .get(&params.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;
    let now = now_secs();
    // Validate via the domain entity; result is discarded (the sqlite UPDATE
    // is the source of truth for what the row becomes).
    existing
        .with_updated_brief(params.brief.clone(), now)
        .map_err(|e| RpcError {
            code: RpcError::INVALID_ARGUMENT,
            message: e.to_string(),
        })?;
    let card = cards
        .update_brief(&params.id, &params.brief, now)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;
    let dto: reeve_protocol::Card = card.into();
    serde_json::to_value(EditResult { card: dto }).map_err(internal)
}
