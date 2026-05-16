//! SQLite-backed card storage.

use anyhow::{Context, Result};
use reeve_protocol::{Card, Column, ContainerStatus};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;

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
         CREATE INDEX IF NOT EXISTS idx_cards_column ON cards(column_);

         CREATE TABLE IF NOT EXISTS containers (
            card_id        TEXT PRIMARY KEY REFERENCES cards(id) ON DELETE CASCADE,
            container_name TEXT NOT NULL,
            status         TEXT NOT NULL CHECK (status IN ('spawning','running','stopped','failed')),
            started_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_containers_status ON containers(status);

         CREATE TABLE IF NOT EXISTS households (
            name           TEXT PRIMARY KEY,
            meta_repo_path TEXT NOT NULL,
            added_at       INTEGER NOT NULL
         );

         PRAGMA foreign_keys = ON;",
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
            card.column.as_wire_str(),
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

pub fn update_card_column(
    conn: &Connection,
    id: &str,
    column: Column,
    now: i64,
) -> Result<Option<Card>> {
    let updated = conn.execute(
        "UPDATE cards SET column_ = ?1, updated_at = ?2 WHERE id = ?3",
        params![column.as_wire_str(), now, id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    get_card(conn, id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRow {
    pub card_id: String,
    pub container_name: String,
    pub status: ContainerStatus,
    pub started_at: i64,
    pub updated_at: i64,
}

pub fn upsert_container(
    conn: &Connection,
    card_id: &str,
    container_name: &str,
    status: ContainerStatus,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO containers (card_id, container_name, status, started_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(card_id) DO UPDATE SET
            container_name = excluded.container_name,
            status         = excluded.status,
            updated_at     = excluded.updated_at",
        params![card_id, container_name, status.as_wire_str(), now],
    )?;
    Ok(())
}

pub fn update_container_status(
    conn: &Connection,
    card_id: &str,
    status: ContainerStatus,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE containers SET status = ?1, updated_at = ?2 WHERE card_id = ?3",
        params![status.as_wire_str(), now, card_id],
    )?;
    Ok(())
}

pub fn get_container(conn: &Connection, card_id: &str) -> Result<Option<ContainerRow>> {
    let mut stmt = conn.prepare(
        "SELECT card_id, container_name, status, started_at, updated_at
         FROM containers WHERE card_id = ?1",
    )?;
    let row = stmt
        .query_row(params![card_id], row_to_container)
        .optional()?;
    Ok(row)
}

pub fn list_containers_by_status(
    conn: &Connection,
    status: ContainerStatus,
) -> Result<Vec<ContainerRow>> {
    let mut stmt = conn.prepare(
        "SELECT card_id, container_name, status, started_at, updated_at
         FROM containers WHERE status = ?1",
    )?;
    let rows = stmt.query_map(params![status.as_wire_str()], row_to_container)?;
    let result: Result<Vec<_>, _> = rows.collect();
    Ok(result?)
}

pub fn remove_container(conn: &Connection, card_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM containers WHERE card_id = ?1",
        params![card_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct HouseholdRow {
    pub name: String,
    pub meta_repo_path: PathBuf,
    pub added_at: i64,
}

pub fn insert_household(conn: &Connection, row: &HouseholdRow) -> Result<()> {
    conn.execute(
        "INSERT INTO households (name, meta_repo_path, added_at) VALUES (?1, ?2, ?3)",
        params![row.name, row.meta_repo_path.to_string_lossy(), row.added_at],
    )?;
    Ok(())
}

pub fn remove_household(conn: &Connection, name: &str) -> Result<bool> {
    let n = conn.execute("DELETE FROM households WHERE name = ?1", [name])?;
    Ok(n > 0)
}

pub fn list_households(conn: &Connection) -> Result<Vec<HouseholdRow>> {
    let mut stmt =
        conn.prepare("SELECT name, meta_repo_path, added_at FROM households ORDER BY name")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HouseholdRow {
                name: r.get(0)?,
                meta_repo_path: PathBuf::from(r.get::<_, String>(1)?),
                added_at: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_household(conn: &Connection, name: &str) -> Result<Option<HouseholdRow>> {
    let result = conn
        .query_row(
            "SELECT name, meta_repo_path, added_at FROM households WHERE name = ?1",
            [name],
            |r| {
                Ok(HouseholdRow {
                    name: r.get(0)?,
                    meta_repo_path: PathBuf::from(r.get::<_, String>(1)?),
                    added_at: r.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(result)
}

/// Return all cards whose `household` column equals `name`. Used by
/// `handle_household_show` to surface the cards-referencing-this-household list.
pub fn cards_for_household(conn: &Connection, name: &str) -> Result<Vec<(String, String, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, brief, column_ FROM cards WHERE household = ?1 ORDER BY id")?;
    let rows = stmt
        .query_map([name], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_container(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContainerRow> {
    let status_str: String = row.get(2)?;
    let status = ContainerStatus::from_wire_str(&status_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(InvalidContainerStatus(status_str.clone())),
        )
    })?;
    Ok(ContainerRow {
        card_id: row.get(0)?,
        container_name: row.get(1)?,
        status,
        started_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("invalid container status: {0}")]
struct InvalidContainerStatus(String);

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let column_str: String = row.get(3)?;
    let column = Column::from_wire_str(&column_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(InvalidColumn(column_str.clone())),
        )
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

#[derive(Debug, thiserror::Error)]
#[error("invalid column value: {0}")]
struct InvalidColumn(String);

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
    fn db_round_trips_every_column_variant() {
        // Every Column variant must survive insert + list. This catches a
        // typo in either as_wire_str or from_wire_str (and both live in
        // reeve-protocol now), and exercises the row-decoding error path
        // by going through the real SQL.
        let conn = open(":memory:").unwrap();
        for (i, col) in Column::ALL.into_iter().enumerate() {
            let card = Card {
                id: format!("card-{i}"),
                household: "myarchy".into(),
                brief: "x".into(),
                column: col,
                created_at: 1000,
                updated_at: 1000,
                needs_heavy: false,
                parent_id: None,
            };
            insert_card(&conn, &card).unwrap();
        }
        let cards = list_cards(&conn).unwrap();
        let columns: Vec<Column> = cards.iter().map(|c| c.column).collect();
        assert_eq!(columns, Column::ALL.to_vec());
    }

    #[test]
    fn upsert_then_get_container_round_trip() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Spawning, 1000).unwrap();
        let row = get_container(&conn, "c1").unwrap().unwrap();
        assert_eq!(row.container_name, "reeve-c1");
        assert_eq!(row.status, ContainerStatus::Spawning);
        assert_eq!(row.started_at, 1000);
    }

    #[test]
    fn update_container_status_changes_status_and_updated_at() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Spawning, 1000).unwrap();
        update_container_status(&conn, "c1", ContainerStatus::Running, 2000).unwrap();
        let row = get_container(&conn, "c1").unwrap().unwrap();
        assert_eq!(row.status, ContainerStatus::Running);
        assert_eq!(row.updated_at, 2000);
    }

    #[test]
    fn list_containers_by_status_filters() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("a")).unwrap();
        insert_card(&conn, &sample_card("b")).unwrap();
        upsert_container(&conn, "a", "reeve-a", ContainerStatus::Running, 1).unwrap();
        upsert_container(&conn, "b", "reeve-b", ContainerStatus::Stopped, 1).unwrap();
        let running = list_containers_by_status(&conn, ContainerStatus::Running).unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].card_id, "a");
    }

    #[test]
    fn remove_container_deletes_row() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Running, 1).unwrap();
        remove_container(&conn, "c1").unwrap();
        assert!(get_container(&conn, "c1").unwrap().is_none());
    }

    #[test]
    fn cascade_delete_when_card_removed() {
        let conn = open(":memory:").unwrap();
        insert_card(&conn, &sample_card("c1")).unwrap();
        upsert_container(&conn, "c1", "reeve-c1", ContainerStatus::Running, 1).unwrap();
        conn.execute("DELETE FROM cards WHERE id = ?1", params!["c1"])
            .unwrap();
        assert!(get_container(&conn, "c1").unwrap().is_none());
    }

    #[test]
    fn insert_and_get_household() {
        let conn = open(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let row = HouseholdRow {
            name: "test".into(),
            meta_repo_path: dir.path().to_path_buf(),
            added_at: 1234,
        };
        insert_household(&conn, &row).unwrap();
        let got = get_household(&conn, "test").unwrap().unwrap();
        assert_eq!(got.name, "test");
        assert_eq!(got.meta_repo_path, dir.path());
    }

    #[test]
    fn duplicate_household_name_errors() {
        let conn = open(":memory:").unwrap();
        let row = HouseholdRow {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/x"),
            added_at: 1,
        };
        insert_household(&conn, &row).unwrap();
        assert!(insert_household(&conn, &row).is_err());
    }

    #[test]
    fn remove_household_returns_true_if_existed() {
        let conn = open(":memory:").unwrap();
        let row = HouseholdRow {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/x"),
            added_at: 1,
        };
        insert_household(&conn, &row).unwrap();
        assert!(remove_household(&conn, "test").unwrap());
        assert!(!remove_household(&conn, "test").unwrap());
    }

    #[test]
    fn list_households_sorted() {
        let conn = open(":memory:").unwrap();
        for name in ["c", "a", "b"] {
            insert_household(
                &conn,
                &HouseholdRow {
                    name: name.into(),
                    meta_repo_path: PathBuf::from(format!("/{name}")),
                    added_at: 0,
                },
            )
            .unwrap();
        }
        let rows = list_households(&conn).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"],
        );
    }
}
