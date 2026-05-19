//! Card domain entity. Private fields + invariants + factory constructors.
//! Conversion to/from the wire DTO `reeve_protocol::Card` lives below.

use reeve_protocol::Column;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CardError {
    #[error("brief must be non-empty")]
    EmptyBrief,
    #[error("brief too long ({0} > 200 chars)")]
    BriefTooLong(usize),
    #[error("id must be non-empty")]
    EmptyId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    id: String,
    household: String,
    brief: String,
    column: Column,
    created_at: i64,
    updated_at: i64,
    needs_heavy: bool,
    parent_id: Option<String>,
}

impl Card {
    /// Create a fresh card in Backlog with current time + fresh UUID.
    pub fn new(household: String, brief: String, needs_heavy: bool) -> Result<Self, CardError> {
        if brief.is_empty() {
            return Err(CardError::EmptyBrief);
        }
        let n = brief.chars().count();
        if n > 200 {
            return Err(CardError::BriefTooLong(n));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            household,
            brief,
            column: Column::Backlog,
            created_at: now,
            updated_at: now,
            needs_heavy,
            parent_id: None,
        })
    }

    // Getters
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn household(&self) -> &str {
        &self.household
    }
    pub fn brief(&self) -> &str {
        &self.brief
    }
    pub fn column(&self) -> Column {
        self.column
    }
    pub fn created_at(&self) -> i64 {
        self.created_at
    }
    pub fn updated_at(&self) -> i64 {
        self.updated_at
    }
    pub fn needs_heavy(&self) -> bool {
        self.needs_heavy
    }
    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    /// Return a new Card with the given brief and the provided updated_at.
    /// Validates the same brief constraints as `Card::new` (non-empty, ≤200 chars).
    /// The caller (handler) owns the clock; this method is pure validation + assembly.
    pub fn with_updated_brief(&self, brief: String, now: i64) -> Result<Self, CardError> {
        if brief.is_empty() {
            return Err(CardError::EmptyBrief);
        }
        let n = brief.chars().count();
        if n > 200 {
            return Err(CardError::BriefTooLong(n));
        }
        Ok(Self {
            brief,
            updated_at: now,
            ..self.clone()
        })
    }

    /// Construct a Card directly from validated fields. Used by the repo when
    /// reading from persisted SQL rows.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: String,
        household: String,
        brief: String,
        column: Column,
        created_at: i64,
        updated_at: i64,
        needs_heavy: bool,
        parent_id: Option<String>,
    ) -> Result<Self, CardError> {
        if id.is_empty() {
            return Err(CardError::EmptyId);
        }
        if brief.is_empty() {
            return Err(CardError::EmptyBrief);
        }
        let n = brief.chars().count();
        if n > 200 {
            return Err(CardError::BriefTooLong(n));
        }
        Ok(Self {
            id,
            household,
            brief,
            column,
            created_at,
            updated_at,
            needs_heavy,
            parent_id,
        })
    }
}

// Domain → wire (always succeeds).
impl From<Card> for reeve_protocol::Card {
    fn from(c: Card) -> Self {
        reeve_protocol::Card {
            id: c.id,
            household: c.household,
            brief: c.brief,
            column: c.column,
            created_at: c.created_at,
            updated_at: c.updated_at,
            needs_heavy: c.needs_heavy,
            parent_id: c.parent_id,
        }
    }
}

// Wire → domain (re-runs invariants).
impl TryFrom<reeve_protocol::Card> for Card {
    type Error = CardError;
    fn try_from(d: reeve_protocol::Card) -> Result<Self, CardError> {
        Self::from_persisted(
            d.id,
            d.household,
            d.brief,
            d.column,
            d.created_at,
            d.updated_at,
            d.needs_heavy,
            d.parent_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire() -> reeve_protocol::Card {
        reeve_protocol::Card {
            id: "abc".into(),
            household: "h".into(),
            brief: "test".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        }
    }

    #[test]
    fn new_card_in_backlog() {
        let c = Card::new("h".into(), "brief".into(), false).unwrap();
        assert_eq!(c.column(), Column::Backlog);
        assert!(!c.id().is_empty());
        assert_eq!(c.brief(), "brief");
    }

    #[test]
    fn new_card_rejects_empty_brief() {
        let err = Card::new("h".into(), "".into(), false).unwrap_err();
        assert!(matches!(err, CardError::EmptyBrief));
    }

    #[test]
    fn new_card_rejects_overlong_brief() {
        let long = "x".repeat(201);
        let err = Card::new("h".into(), long, false).unwrap_err();
        assert!(matches!(err, CardError::BriefTooLong(201)));
    }

    #[test]
    fn protocol_to_domain_to_protocol_round_trips() {
        let dto = sample_wire();
        let entity = Card::try_from(dto.clone()).unwrap();
        let dto2: reeve_protocol::Card = entity.into();
        assert_eq!(dto, dto2);
    }

    #[test]
    fn try_from_rejects_invalid_persisted() {
        let mut bad = sample_wire();
        bad.brief = String::new();
        let err = Card::try_from(bad).unwrap_err();
        assert!(matches!(err, CardError::EmptyBrief));
    }
}
