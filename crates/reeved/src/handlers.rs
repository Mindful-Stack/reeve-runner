//! Method dispatch and per-method handlers.

use crate::db;
use crate::effects::{DaemonState, LiveEffects};
use crate::server::{CallerContext, DbHandle};
use crate::transitions;
use anyhow::Result;
use reeve_protocol::{
    Card, Column, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult, NewParams, NewResult,
    RpcError, RpcRequest, StatusSelfResult,
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn dispatch(
    req: RpcRequest,
    state: DaemonState,
    caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => handle_ls(state.db).await,
        "new" => {
            let params = parse_params::<NewParams>(req.params)?;
            handle_new(state.db, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            handle_mv(state, params).await
        }
        "mv-self" => {
            let card_id = require_card(&caller)?;
            let params = parse_params::<MvSelfParams>(req.params)?;
            handle_mv_self(state, card_id, params).await
        }
        "status-self" => {
            let card_id = require_card(&caller)?;
            handle_status_self(state.db, card_id).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
}

fn require_card(caller: &CallerContext) -> std::result::Result<String, RpcError> {
    caller.card_id.clone().ok_or_else(|| RpcError {
        code: RpcError::FORBIDDEN,
        message: "self-scoped methods require per-card socket".into(),
    })
}

fn parse_params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError {
        code: RpcError::INVALID_PARAMS,
        message: format!("invalid params: {e}"),
    })
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn to_internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError {
        code: RpcError::INTERNAL,
        message: e.to_string(),
    }
}

async fn handle_ls(db: DbHandle) -> std::result::Result<Value, RpcError> {
    let cards = tokio::task::spawn_blocking(move || -> Result<Vec<Card>> {
        let conn = db.blocking_lock();
        db::list_cards(&conn)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    serde_json::to_value(LsResult { cards }).map_err(to_internal)
}

async fn handle_new(db: DbHandle, params: NewParams) -> std::result::Result<Value, RpcError> {
    let now = now_secs();
    let card = Card {
        id: uuid::Uuid::new_v4().to_string(),
        household: params.household,
        brief: params.brief,
        column: Column::Backlog,
        created_at: now,
        updated_at: now,
        needs_heavy: params.needs_heavy,
        parent_id: None,
    };
    let card_for_insert = card.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = db.blocking_lock();
        db::insert_card(&conn, &card_for_insert)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    serde_json::to_value(NewResult { card }).map_err(to_internal)
}

async fn handle_mv(state: DaemonState, params: MvParams) -> std::result::Result<Value, RpcError> {
    let id = params.id.clone();
    let new_column = params.column;
    let now = now_secs();

    // Read the existing card so we know the old column AND the household name.
    let db = state.db.clone();
    let id_for_query = id.clone();
    let existing = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::get_card(&conn, &id_for_query)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {}", params.id),
    })?;
    let old_column = existing.column;

    // Precheck: a move that requires a container must have a configured household.
    // Reject BEFORE the column is mutated so the user can fix their config and retry.
    if new_column == Column::Implementing && !state.households.contains_key(&existing.household) {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }

    let db = state.db.clone();
    let id_for_update = id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id_for_update, new_column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {}", params.id),
    })?;

    let fx = LiveEffects::new(state);
    transitions::on_column_change(old_column, new_column, &card, &fx)
        .await
        .map_err(to_internal)?;
    serde_json::to_value(MvResult { card }).map_err(to_internal)
}

async fn handle_mv_self(
    state: DaemonState,
    card_id: String,
    params: MvSelfParams,
) -> std::result::Result<Value, RpcError> {
    let new_column = params.column;
    let now = now_secs();

    let db = state.db.clone();
    let id_for_query = card_id.clone();
    let existing = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::get_card(&conn, &id_for_query)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;
    let old_column = existing.column;

    if new_column == Column::Implementing && !state.households.contains_key(&existing.household) {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household),
        });
    }

    let db = state.db.clone();
    let id_for_update = card_id.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id_for_update, new_column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?
    .ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;

    let fx = LiveEffects::new(state);
    transitions::on_column_change(old_column, new_column, &card, &fx)
        .await
        .map_err(to_internal)?;
    serde_json::to_value(MvSelfResult { card }).map_err(to_internal)
}

async fn handle_status_self(db: DbHandle, card_id: String) -> std::result::Result<Value, RpcError> {
    let id = card_id.clone();
    let (card, container) = tokio::task::spawn_blocking(
        move || -> Result<(Option<Card>, Option<db::ContainerRow>)> {
            let conn = db.blocking_lock();
            let card = db::get_card(&conn, &id)?;
            let container = db::get_container(&conn, &id)?;
            Ok((card, container))
        },
    )
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    let card = card.ok_or_else(|| RpcError {
        code: RpcError::NOT_FOUND,
        message: format!("card not found: {card_id}"),
    })?;
    serde_json::to_value(StatusSelfResult {
        card,
        container_status: container.map(|c| c.status),
    })
    .map_err(to_internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{ContainerOps, ContainerOpsHandle};
    use crate::effects::DaemonState;
    use crate::listeners::{Dispatcher, ListenerRegistry};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// No-op container backend that pretends nothing exists.
    struct NoopOps;
    #[async_trait]
    impl ContainerOps for NoopOps {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> {
            Ok(())
        }
        async fn teardown(&self, _name: &str) -> Result<()> {
            Ok(())
        }
        async fn inspect_status(&self, _name: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    /// Dispatcher stub; transitions don't need to call back here in tests.
    struct NoopDispatcher;
    impl Dispatcher for NoopDispatcher {
        fn dispatch(
            &self,
            _r: RpcRequest,
            _d: DbHandle,
            _c: CallerContext,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>>
                    + Send,
            >,
        > {
            Box::pin(async { Ok(serde_json::Value::Null) })
        }
    }

    fn fresh_state() -> DaemonState {
        fresh_state_with_households(HashMap::new())
    }

    fn fresh_state_with_households(
        map: HashMap<String, crate::household::HouseholdProfile>,
    ) -> DaemonState {
        let conn = db::open(":memory:").unwrap();
        let db: DbHandle = Arc::new(Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as ContainerOpsHandle,
            listeners: ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            households: Arc::new(map),
            claude_dir: PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    #[tokio::test]
    async fn ls_on_empty_returns_empty_list() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let value = dispatch(req, state, CallerContext::host()).await.unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert!(result.cards.is_empty());
    }

    #[tokio::test]
    async fn new_creates_card_then_ls_returns_it() {
        let state = fresh_state();
        let req_new = RpcRequest {
            id: 1,
            method: "new".into(),
            params: serde_json::json!({
                "household": "myarchy",
                "brief": "first card",
                "needs_heavy": false,
            }),
        };
        let new_value = dispatch(req_new, state.clone(), CallerContext::host())
            .await
            .unwrap();
        let new_result: NewResult = serde_json::from_value(new_value).unwrap();
        assert_eq!(new_result.card.brief, "first card");
        assert_eq!(new_result.card.column, Column::Backlog);

        let req_ls = RpcRequest {
            id: 2,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let ls_value = dispatch(req_ls, state, CallerContext::host())
            .await
            .unwrap();
        let ls_result: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls_result.cards.len(), 1);
        assert_eq!(ls_result.cards[0], new_result.card);
    }

    #[tokio::test]
    async fn mv_changes_column_and_returns_updated() {
        let state = fresh_state();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "myarchy",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": card.id,
                    "column": "review",
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let updated = serde_json::from_value::<MvResult>(mv_value).unwrap().card;
        assert_eq!(updated.column, Column::Review);
    }

    #[tokio::test]
    async fn mv_unknown_id_returns_not_found() {
        let state = fresh_state();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": "missing",
                    "column": "done",
                }),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let state = fresh_state();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "nope".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn mv_self_on_host_socket_returns_forbidden() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "mv-self".into(),
            params: serde_json::json!({"column": "review"}),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::FORBIDDEN);
    }

    #[tokio::test]
    async fn mv_self_on_card_socket_moves_card() {
        let state = fresh_state();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "b",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv-self".into(),
                params: serde_json::json!({"column": "review"}),
            },
            state,
            CallerContext::card(&card.id),
        )
        .await
        .unwrap();
        let result: MvSelfResult = serde_json::from_value(mv_value).unwrap();
        assert_eq!(result.card.column, Column::Review);
        assert_eq!(result.card.id, card.id);
    }

    #[tokio::test]
    async fn status_self_on_host_socket_returns_forbidden() {
        let state = fresh_state();
        let req = RpcRequest {
            id: 1,
            method: "status-self".into(),
            params: serde_json::json!({}),
        };
        let err = dispatch(req, state, CallerContext::host())
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcError::FORBIDDEN);
    }

    #[tokio::test]
    async fn status_self_on_card_socket_returns_card_no_container() {
        let state = fresh_state();
        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "b",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let value = dispatch(
            RpcRequest {
                id: 2,
                method: "status-self".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::card(&card.id),
        )
        .await
        .unwrap();
        let result: StatusSelfResult = serde_json::from_value(value).unwrap();
        assert_eq!(result.card.id, card.id);
        assert!(result.container_status.is_none());
    }

    #[tokio::test]
    async fn mv_to_implementing_without_household_leaves_column_unchanged() {
        let state = fresh_state();

        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        let err = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::INTERNAL);
        assert!(err.message.contains("household 'h' not configured"));

        let ls_value = dispatch(
            RpcRequest {
                id: 3,
                method: "ls".into(),
                params: serde_json::json!({}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let ls: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls.cards[0].column, Column::Backlog);
    }

    #[tokio::test]
    async fn mv_to_implementing_with_configured_household_succeeds() {
        use crate::household::HouseholdProfile;
        let mut households = HashMap::new();
        households.insert(
            "h".to_string(),
            HouseholdProfile {
                image: "alpine:latest".into(),
                cmd: vec!["sleep".into(), "infinity".into()],
            },
        );
        let state = fresh_state_with_households(households);

        let new_value = dispatch(
            RpcRequest {
                id: 1,
                method: "new".into(),
                params: serde_json::json!({
                    "household": "h",
                    "brief": "x",
                    "needs_heavy": false,
                }),
            },
            state.clone(),
            CallerContext::host(),
        )
        .await
        .unwrap();
        let card = serde_json::from_value::<NewResult>(new_value).unwrap().card;

        // NoopOps reports no container exists; LiveEffects::ensure_running
        // then writes Spawning, calls NoopOps::spawn (Ok), writes Running,
        // and spawns the listener.
        let mv_value = dispatch(
            RpcRequest {
                id: 2,
                method: "mv".into(),
                params: serde_json::json!({"id": card.id, "column": "implementing"}),
            },
            state,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let result: MvResult = serde_json::from_value(mv_value).unwrap();
        assert_eq!(result.card.column, Column::Implementing);
    }
}
