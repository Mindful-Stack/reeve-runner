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
}
