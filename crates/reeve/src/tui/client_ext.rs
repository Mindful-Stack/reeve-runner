//! Typed RPC wrappers around the raw `client::call` primitive.
//!
//! Each function loads the daemon socket path, serializes the params,
//! calls the transport layer, and deserializes the response into a concrete
//! protocol type. This keeps all `serde_json::Value` handling in one place
//! so that action handlers and future tasks deal only with typed values.
//!
//! Return type convention:
//!   `Ok(Ok(T))` — success
//!   `Ok(Err(RpcError))` — daemon returned an error response
//!   `Err(anyhow)` — transport or (de)serialization failure

use anyhow::Result;
use reeve_protocol::{
    AttachParams, AttachResult, Card, CardProgressParams, CardProgressResult, DeleteParams,
    EditParams, EditResult, HouseholdSummary, LsResult, MvParams, MvResult, NewParams, NewResult,
    RpcError, paths,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Internal helper: serialize `params`, call the daemon, deserialize `R`.
async fn call<P, R>(method: &str, params: P) -> Result<std::result::Result<R, RpcError>>
where
    P: Serialize,
    R: DeserializeOwned,
{
    let socket = paths::socket_path();
    let value = serde_json::to_value(params)?;
    match crate::client::call(&socket, method, value).await? {
        Ok(v) => Ok(Ok(serde_json::from_value(v)?)),
        Err(e) => Ok(Err(e)),
    }
}

pub async fn rpc_ls() -> Result<std::result::Result<LsResult, RpcError>> {
    call("ls", serde_json::json!({})).await
}

pub async fn rpc_new(params: NewParams) -> Result<std::result::Result<NewResult, RpcError>> {
    call("new", params).await
}

/// Move a card to a new column. Unwraps the one-field `MvResult` for convenience,
/// returning the updated `Card` directly.
pub async fn rpc_mv(params: MvParams) -> Result<std::result::Result<Card, RpcError>> {
    let result: Result<std::result::Result<MvResult, RpcError>> = call("mv", params).await;
    result.map(|r| r.map(|r| r.card))
}

pub async fn rpc_edit(params: EditParams) -> Result<std::result::Result<EditResult, RpcError>> {
    call("edit", params).await
}

/// Delete a card. The daemon returns null on success; we discard it and return `()`.
pub async fn rpc_delete(params: DeleteParams) -> Result<std::result::Result<(), RpcError>> {
    let result: Result<std::result::Result<serde_json::Value, RpcError>> =
        call("delete", params).await;
    result.map(|r| r.map(|_| ()))
}

pub async fn rpc_attach(
    params: AttachParams,
) -> Result<std::result::Result<AttachResult, RpcError>> {
    call("attach", params).await
}

pub async fn rpc_card_progress(
    params: CardProgressParams,
) -> Result<std::result::Result<CardProgressResult, RpcError>> {
    call("card_progress", params).await
}

pub async fn rpc_household_ls() -> Result<std::result::Result<Vec<HouseholdSummary>, RpcError>> {
    call("household.ls", serde_json::json!({})).await
}
