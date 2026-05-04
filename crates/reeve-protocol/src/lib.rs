//! Shared protocol types for Reeve daemon and clients.

use serde::{Deserialize, Serialize};

/// The kanban columns a card may occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Backlog,
    Planning,
    Implementing,
    Testing,
    Review,
    Done,
}

impl Column {
    /// All columns in canonical left-to-right kanban order.
    pub const ALL: [Column; 6] = [
        Column::Backlog,
        Column::Planning,
        Column::Implementing,
        Column::Testing,
        Column::Review,
        Column::Done,
    ];

    /// Display name for headings.
    pub fn label(&self) -> &'static str {
        match self {
            Column::Backlog => "Backlog",
            Column::Planning => "Planning",
            Column::Implementing => "Implementing",
            Column::Testing => "Testing",
            Column::Review => "Review",
            Column::Done => "Done",
        }
    }
}

/// A card in the kanban — one feature's worth of work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub household: String,
    pub brief: String,
    pub column: Column,
    pub created_at: i64,
    pub updated_at: i64,
    pub needs_heavy: bool,
    pub parent_id: Option<String>,
}

/// A JSON-line RPC request from client → daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

/// A JSON-line RPC response from daemon → client.
///
/// Exactly one of `result` or `error` is `Some` for any non-malformed response.
/// `id` is `None` only when the daemon could not parse the request well
/// enough to recover its id (e.g. malformed JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("rpc error {code}: {message}")]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;
    pub const NOT_FOUND: i32 = 404;
}

/// Parameters for the `ls` method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsParams {}

/// Result for the `ls` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsResult {
    pub cards: Vec<Card>,
}

/// Parameters for the `new` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewParams {
    pub household: String,
    pub brief: String,
    pub needs_heavy: bool,
}

/// Result for the `new` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewResult {
    pub card: Card,
}

/// Parameters for the `mv` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvParams {
    pub id: String,
    pub column: Column,
}

/// Result for the `mv` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvResult {
    pub card: Card,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_serializes_to_lowercase_string() {
        let json = serde_json::to_string(&Column::Implementing).unwrap();
        assert_eq!(json, "\"implementing\"");
    }

    #[test]
    fn column_deserializes_from_lowercase_string() {
        let col: Column = serde_json::from_str("\"review\"").unwrap();
        assert_eq!(col, Column::Review);
    }

    #[test]
    fn card_round_trips_through_json() {
        let card = Card {
            id: "abc".into(),
            household: "myarchy".into(),
            brief: "test brief".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let back: Card = serde_json::from_str(&json).unwrap();
        assert_eq!(back, card);
    }

    #[test]
    fn rpc_request_serializes_with_method_and_params() {
        let req = RpcRequest {
            id: 7,
            method: "ls".into(),
            params: serde_json::json!({}),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"id\":7"));
        assert!(json.contains("\"method\":\"ls\""));
    }

    #[test]
    fn rpc_response_with_result_omits_error_field() {
        let resp = RpcResponse {
            id: Some(1),
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn rpc_response_with_no_id_omits_id_field() {
        let resp = RpcResponse {
            id: None,
            result: None,
            error: Some(RpcError {
                code: RpcError::INVALID_PARAMS,
                message: "bad".into(),
            }),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"id\""));
        assert!(json.contains("\"error\""));
    }

    #[test]
    fn new_params_round_trip() {
        let p = NewParams {
            household: "myarchy".into(),
            brief: "do the thing".into(),
            needs_heavy: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: NewParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn mv_params_round_trip() {
        let p = MvParams {
            id: "abc".into(),
            column: Column::Implementing,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: MvParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
