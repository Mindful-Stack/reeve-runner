//! Method dispatch and per-method handlers.

use crate::db;
use crate::server::{CallerContext, DbHandle};
use anyhow::Result;
use reeve_protocol::{
    Card, Column, LsResult, MvParams, MvResult, NewParams, NewResult, RpcError, RpcRequest,
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn dispatch(
    req: RpcRequest,
    db: DbHandle,
    _caller: CallerContext,
) -> std::result::Result<Value, RpcError> {
    match req.method.as_str() {
        "ls" => handle_ls(db).await,
        "new" => {
            let params = parse_params::<NewParams>(req.params)?;
            handle_new(db, params).await
        }
        "mv" => {
            let params = parse_params::<MvParams>(req.params)?;
            handle_mv(db, params).await
        }
        other => Err(RpcError {
            code: RpcError::METHOD_NOT_FOUND,
            message: format!("method not found: {other}"),
        }),
    }
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

async fn handle_mv(db: DbHandle, params: MvParams) -> std::result::Result<Value, RpcError> {
    let id = params.id.clone();
    let column = params.column;
    let now = now_secs();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
        let conn = db.blocking_lock();
        db::update_card_column(&conn, &id, column, now)
    })
    .await
    .map_err(to_internal)?
    .map_err(to_internal)?;
    match card {
        Some(card) => serde_json::to_value(MvResult { card }).map_err(to_internal),
        None => Err(RpcError {
            code: RpcError::NOT_FOUND,
            message: format!("card not found: {}", params.id),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::CallerContext;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn fresh_db() -> DbHandle {
        let conn = db::open(":memory:").unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test]
    async fn ls_on_empty_returns_empty_list() {
        let db = fresh_db();
        let req = RpcRequest {
            id: 1,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let value = dispatch(req, db, CallerContext::host()).await.unwrap();
        let result: LsResult = serde_json::from_value(value).unwrap();
        assert!(result.cards.is_empty());
    }

    #[tokio::test]
    async fn new_creates_card_then_ls_returns_it() {
        let db = fresh_db();
        let req_new = RpcRequest {
            id: 1,
            method: "new".into(),
            params: serde_json::json!({
                "household": "myarchy",
                "brief": "first card",
                "needs_heavy": false,
            }),
        };
        let new_value = dispatch(req_new, db.clone(), CallerContext::host()).await.unwrap();
        let new_result: NewResult = serde_json::from_value(new_value).unwrap();
        assert_eq!(new_result.card.brief, "first card");
        assert_eq!(new_result.card.column, Column::Backlog);

        let req_ls = RpcRequest {
            id: 2,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let ls_value = dispatch(req_ls, db, CallerContext::host()).await.unwrap();
        let ls_result: LsResult = serde_json::from_value(ls_value).unwrap();
        assert_eq!(ls_result.cards.len(), 1);
        assert_eq!(ls_result.cards[0], new_result.card);
    }

    #[tokio::test]
    async fn mv_changes_column_and_returns_updated() {
        let db = fresh_db();
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
            db.clone(),
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
                    "column": "implementing",
                }),
            },
            db,
            CallerContext::host(),
        )
        .await
        .unwrap();
        let updated = serde_json::from_value::<MvResult>(mv_value).unwrap().card;
        assert_eq!(updated.column, Column::Implementing);
    }

    #[tokio::test]
    async fn mv_unknown_id_returns_not_found() {
        let db = fresh_db();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "mv".into(),
                params: serde_json::json!({
                    "id": "missing",
                    "column": "done",
                }),
            },
            db,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let db = fresh_db();
        let err = dispatch(
            RpcRequest {
                id: 1,
                method: "nope".into(),
                params: serde_json::json!({}),
            },
            db,
            CallerContext::host(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, RpcError::METHOD_NOT_FOUND);
    }
}
