//! Persistence port + SQLite adapter for the Card aggregate.

use crate::domain::Card;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::{CardReference, Column};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait CardRepo: Send + Sync {
    async fn list(&self) -> Result<Vec<Card>>;
    async fn get(&self, id: &str) -> Result<Option<Card>>;
    async fn insert(&self, card: &Card) -> Result<()>;
    async fn update_column(&self, id: &str, col: Column, now: i64) -> Result<Option<Card>>;
    async fn update_brief(&self, id: &str, brief: &str, now: i64) -> Result<Option<Card>>;
    async fn delete(&self, id: &str) -> Result<bool>;
    async fn for_household(&self, name: &str) -> Result<Vec<CardReference>>;
}

pub(crate) struct SqliteCardRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCardRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl CardRepo for SqliteCardRepo {
    async fn list(&self) -> Result<Vec<Card>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<Card>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([], row_to_card)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?
    }

    async fn get(&self, id: &str) -> Result<Option<Card>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards WHERE id = ?1",
            )?;
            let card = stmt.query_row(params![id], row_to_card).optional()?;
            Ok(card)
        })
        .await?
    }

    async fn insert(&self, card: &Card) -> Result<()> {
        let conn = self.conn.clone();
        let card = card.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO cards (id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    card.id(), card.household(), card.brief(),
                    card.column().as_wire_str(), card.created_at(), card.updated_at(),
                    card.needs_heavy() as i64, card.parent_id(),
                ],
            )?;
            Ok(())
        })
        .await?
    }

    async fn update_column(&self, id: &str, col: Column, now: i64) -> Result<Option<Card>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
            let conn = conn.blocking_lock();
            let updated = conn.execute(
                "UPDATE cards SET column_ = ?1, updated_at = ?2 WHERE id = ?3",
                params![col.as_wire_str(), now, id],
            )?;
            if updated == 0 {
                return Ok(None);
            }
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards WHERE id = ?1",
            )?;
            Ok(stmt.query_row(params![id], row_to_card).optional()?)
        })
        .await?
    }

    async fn update_brief(&self, id: &str, brief: &str, now: i64) -> Result<Option<Card>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let brief = brief.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<Card>> {
            let conn = conn.blocking_lock();
            let affected = conn
                .execute(
                    "UPDATE cards SET brief = ?1, updated_at = ?2 WHERE id = ?3",
                    params![&brief, now, &id],
                )
                .context("update brief")?;
            if affected == 0 {
                return Ok(None);
            }
            let mut stmt = conn.prepare(
                "SELECT id, household, brief, column_, created_at, updated_at, needs_heavy, parent_id
                 FROM cards WHERE id = ?1",
            )?;
            let row = stmt.query_row(params![&id], row_to_card).optional()?;
            Ok(row)
        })
        .await?
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = conn.blocking_lock();
            let affected = conn
                .execute("DELETE FROM cards WHERE id = ?1", params![&id])
                .context("delete card")?;
            Ok(affected > 0)
        })
        .await?
    }

    async fn for_household(&self, name: &str) -> Result<Vec<CardReference>> {
        let conn = self.conn.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<CardReference>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare("SELECT id, brief, column_ FROM cards WHERE household = ?1 ORDER BY id")?;
            let rows = stmt.query_map([name], |r| {
                Ok(CardReference {
                    id: r.get::<_, String>(0)?,
                    brief: r.get::<_, String>(1)?,
                    column: r.get::<_, String>(2)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?
    }
}

fn row_to_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
    let column_str: String = row.get(3)?;
    let column = Column::from_wire_str(&column_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(InvalidColumn(column_str.clone())),
        )
    })?;
    Card::from_persisted(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        column,
        row.get(4)?,
        row.get(5)?,
        row.get::<_, i64>(6)? != 0,
        row.get(7)?,
    )
    .map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(InvalidPersistedCard(e.to_string())),
        )
    })
}

#[derive(Debug, thiserror::Error)]
#[error("invalid column value: {0}")]
struct InvalidColumn(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid persisted card: {0}")]
struct InvalidPersistedCard(String);

/// Construct the SQLite-backed CardRepo. Only `reeved` should call this.
pub fn make_card_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn CardRepo> {
    Arc::new(SqliteCardRepo::new(conn))
}

/// Initialise the cards table. Called by `reeved/src/app/lifecycle.rs::migrate`.
pub fn init_schema(conn: &Connection) -> Result<()> {
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
    )
    .context("init_schema for cards")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> Card {
        Card::from_persisted(
            id.into(),
            "h".into(),
            "test".into(),
            Column::Backlog,
            1000,
            1000,
            false,
            None,
        )
        .unwrap()
    }

    fn fresh_repo() -> Arc<dyn CardRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_card_repo(Arc::new(Mutex::new(conn)))
    }

    #[tokio::test]
    async fn insert_then_list() {
        let repo = fresh_repo();
        repo.insert(&sample("a")).await.unwrap();
        let cards = repo.list().await.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id(), "a");
    }

    #[tokio::test]
    async fn update_column_returns_updated() {
        let repo = fresh_repo();
        repo.insert(&sample("a")).await.unwrap();
        let updated = repo
            .update_column("a", Column::Implementing, 2000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.column(), Column::Implementing);
        assert_eq!(updated.updated_at(), 2000);
    }

    #[tokio::test]
    async fn update_missing_returns_none() {
        let repo = fresh_repo();
        let result = repo
            .update_column("missing", Column::Done, 2000)
            .await
            .unwrap();
        assert!(result.is_none());
    }
}
