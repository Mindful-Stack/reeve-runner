//! SQLite-backed card storage.

use anyhow::{Context, Result};
use reeve_protocol::{Card, Column};
use rusqlite::{Connection, OptionalExtension, params};

/// Open a SQLite connection at `path` and run any pending migrations.
/// Pass `":memory:"` for an in-memory database (used in tests).
pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("open db at {path}"))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cards (
            id          TEXT PRIMARY KEY,
            household   TEXT NOT NULL,
            brief       TEXT NOT NULL,
            column_     TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            needs_heavy INTEGER NOT NULL DEFAULT 0,
            parent_id   TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);",
    )?;
    Ok(())
}

pub fn insert_card(conn: &Connection, card: &Card) -> Result<()> {
    conn.execute(
        "INSERT INTO cards (id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            card.id,
            card.household,
            card.brief,
            column_as_str(card.column),
            card.created_at,
            card.updated_at,
            card.needs_heavy as i64,
            card.parent_id,
        ],
    )?;
    Ok(())
}

pub fn list_cards(conn: &Connection) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
         FROM cards ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], row_to_card)?;
    let cards: Result<Vec<_>, _> = rows.collect();
    Ok(cards?)
}

pub fn get_card(conn: &Connection, id: &str) -> Result<Option<Card>> {
    let mut stmt = conn.prepare(
        "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
         FROM cards WHERE id = ?1",
    )?;
    let card = stmt.query_row(params![id], row_to_card).optional()?;
    Ok(card)
}

pub fn update_card_column(conn: &Connection, id: &str, column: Column, now: i64) -> Result<Option<Card>> {
    let updated = conn.execute(
        "UPDATE cards SET column_ = ?1, updated_at = ?2 WHERE id = ?3",
        params![column_as_str(column), now, id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    get_card(conn, id)
}

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let column_str: String = row.get(3)?;
    let column = parse_column(&column_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Card {
        id: row.get(0)?,
        household: row.get(1)?,
        brief: row.get(2)?,
        column,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        needs_heavy: row.get::<_, i64>(6)? != 0,
        parent_id: row.get(7)?,
    })
}

fn column_as_str(column: Column) -> &'static str {
    match column {
        Column::Backlog => "backlog",
        Column::Planning => "planning",
        Column::Implementing => "implementing",
        Column::Testing => "testing",
        Column::Review => "review",
        Column::Done => "done",
    }
}

fn parse_column(s: &str) -> Result<Column, ParseColumnError> {
    match s {
        "backlog" => Ok(Column::Backlog),
        "planning" => Ok(Column::Planning),
        "implementing" => Ok(Column::Implementing),
        "testing" => Ok(Column::Testing),
        "review" => Ok(Column::Review),
        "done" => Ok(Column::Done),
        _ => Err(ParseColumnError(s.to_owned())),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid column value: {0}")]
struct ParseColumnError(String);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card(id: &str) -> Card {
        Card {
            id: id.into(),
            household: "myarchy".into(),
            brief: "test".into(),
            column: Column::Backlog,
            created_at: 1000,
            updated_at: 1000,
            needs_heavy: false,
            parent_id: None,
        }
    }

    #[test]
    fn open_initialises_schema() {
        let conn = open(":memory:").unwrap();
        // Table exists if list_cards returns empty.
        let cards = list_cards(&conn).unwrap();
        assert!(cards.is_empty());
    }

    #[test]
    fn insert_then_list_returns_card() {
        let conn = open(":memory:").unwrap();
        let card = sample_card("a");
        insert_card(&conn, &card).unwrap();
        let cards = list_cards(&conn).unwrap();
        assert_eq!(cards, vec![card]);
    }

    #[test]
    fn update_card_column_changes_column_and_returns_updated() {
        let conn = open(":memory:").unwrap();
        let mut card = sample_card("a");
        insert_card(&conn, &card).unwrap();
        let updated = update_card_column(&conn, "a", Column::Implementing, 2000)
            .unwrap()
            .unwrap();
        card.column = Column::Implementing;
        card.updated_at = 2000;
        assert_eq!(updated, card);
    }

    #[test]
    fn update_missing_card_returns_none() {
        let conn = open(":memory:").unwrap();
        let result = update_card_column(&conn, "nonexistent", Column::Done, 2000).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_card_returns_none_for_missing() {
        let conn = open(":memory:").unwrap();
        assert!(get_card(&conn, "missing").unwrap().is_none());
    }

    #[test]
    fn column_db_string_matches_serde_for_all_variants() {
        // Pins the invariant that the DB column string (column_as_str /
        // parse_column) stays in sync with the JSON wire format from
        // #[serde(rename_all = "lowercase")] on Column.
        for col in Column::ALL {
            let db_str = column_as_str(col);
            let json_str = serde_json::to_value(col).unwrap();
            assert_eq!(serde_json::Value::String(db_str.into()), json_str);
            assert_eq!(parse_column(db_str).unwrap(), col);
        }
    }
}
