//! Persistence port + SQLite adapter for the Workspace aggregate.
//! Owns the containers table (no FK to cards — see spec §"Schema independence").

use anyhow::{Context, Result};
use async_trait::async_trait;
use reeve_protocol::ContainerStatus;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRow {
    pub card_id: String,
    pub container_name: String,
    pub status: ContainerStatus,
    pub started_at: i64,
    pub updated_at: i64,
}

#[async_trait]
pub trait WorkspaceRepo: Send + Sync {
    async fn upsert_container(
        &self,
        card_id: &str,
        container_name: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()>;
    async fn update_container_status(
        &self,
        card_id: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()>;
    async fn get_container(&self, card_id: &str) -> Result<Option<ContainerRow>>;
    async fn list_by_status(&self, status: ContainerStatus) -> Result<Vec<ContainerRow>>;
    async fn remove_container(&self, card_id: &str) -> Result<()>;
}

pub(crate) struct SqliteWorkspaceRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteWorkspaceRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl WorkspaceRepo for SqliteWorkspaceRepo {
    async fn upsert_container(
        &self,
        card_id: &str,
        container_name: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        let container_name = container_name.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
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
        })
        .await?
    }

    async fn update_container_status(
        &self,
        card_id: &str,
        status: ContainerStatus,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "UPDATE containers SET status = ?1, updated_at = ?2 WHERE card_id = ?3",
                params![status.as_wire_str(), now, card_id],
            )?;
            Ok(())
        })
        .await?
    }

    async fn get_container(&self, card_id: &str) -> Result<Option<ContainerRow>> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<ContainerRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT card_id, container_name, status, started_at, updated_at
                 FROM containers WHERE card_id = ?1",
            )?;
            let row = stmt
                .query_row(params![card_id], row_to_container)
                .optional()?;
            Ok(row)
        })
        .await?
    }

    async fn list_by_status(&self, status: ContainerStatus) -> Result<Vec<ContainerRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<ContainerRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT card_id, container_name, status, started_at, updated_at
                 FROM containers WHERE status = ?1",
            )?;
            let rows = stmt
                .query_map(params![status.as_wire_str()], row_to_container)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?
    }

    async fn remove_container(&self, card_id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let card_id = card_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute("DELETE FROM containers WHERE card_id = ?1", params![card_id])?;
            Ok(())
        })
        .await?
    }
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

pub fn make_workspace_repo(conn: Arc<Mutex<Connection>>) -> Arc<dyn WorkspaceRepo> {
    Arc::new(SqliteWorkspaceRepo::new(conn))
}

/// Initialise the containers table. NO foreign key to cards — schemas are
/// per-context independent (spec §"Schema independence").
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS containers (
            card_id        TEXT PRIMARY KEY,
            container_name TEXT NOT NULL,
            status         TEXT NOT NULL CHECK (status IN ('spawning','running','stopped','failed')),
            started_at     INTEGER NOT NULL,
            updated_at     INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_containers_status ON containers(status);",
    )
    .context("init_schema for containers")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<dyn WorkspaceRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_workspace_repo(Arc::new(Mutex::new(conn)))
    }

    #[tokio::test]
    async fn upsert_then_get() {
        let repo = fresh();
        repo.upsert_container("c1", "reeve-c1", ContainerStatus::Spawning, 1000)
            .await
            .unwrap();
        let row = repo.get_container("c1").await.unwrap().unwrap();
        assert_eq!(row.container_name, "reeve-c1");
        assert_eq!(row.status, ContainerStatus::Spawning);
    }

    #[tokio::test]
    async fn no_fk_no_cards_row_needed() {
        // Critical: dropping the FK means we can insert a containers row
        // without any cards row existing. Verifies the spec's stated benefit.
        let repo = fresh();
        repo.upsert_container("nonexistent-card", "x", ContainerStatus::Running, 0)
            .await
            .unwrap();
        let row = repo.get_container("nonexistent-card").await.unwrap();
        assert!(row.is_some());
    }
}
