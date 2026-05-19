//! RPC handlers for the Card aggregate.

use crate::domain::Card;
use crate::repo::CardRepo;
use crate::transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
use reeve_protocol::{
    Column, LsResult, MvParams, MvResult, MvSelfParams, MvSelfResult,
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

pub async fn handle_ls(cards: &dyn CardRepo) -> std::result::Result<Value, RpcError> {
    let entities = cards.list().await.map_err(internal)?;
    let dtos: Vec<reeve_protocol::Card> = entities.into_iter().map(Into::into).collect();
    serde_json::to_value(LsResult { cards: dtos }).map_err(internal)
}

pub async fn handle_new(
    cards: &dyn CardRepo,
    params: NewParams,
) -> std::result::Result<Value, RpcError> {
    let card = Card::new(params.household, params.brief, params.needs_heavy)
        .map_err(|e| RpcError {
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

    if params.column == Column::Implementing
        && !households.exists(existing.household()).await.map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household()),
        });
    }

    let now = now_secs();
    let card = cards
        .update_column(&params.id, params.column, now)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&params.id))?;

    on_column_change(old_column, params.column, &card, workspace)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;

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

    if params.column == Column::Implementing
        && !households.exists(existing.household()).await.map_err(internal)?
    {
        return Err(RpcError {
            code: RpcError::INTERNAL,
            message: format!("household '{}' not configured", existing.household()),
        });
    }

    let now = now_secs();
    let card = cards
        .update_column(&card_id, params.column, now)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found(&card_id))?;

    on_column_change(old_column, params.column, &card, workspace)
        .await
        .map_err(|e| RpcError {
            code: RpcError::INTERNAL,
            message: format!("{e:#}"),
        })?;

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
