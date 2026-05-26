//! Persistence port + SQLite adapter for the Household aggregate.
//! Also implements `reeve_card::HouseholdLookup`.

use crate::domain::Household;
use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Persistence port for Household. Owned by `reeve-household`.
#[async_trait]
pub trait HouseholdRepo: Send + Sync {
    async fn insert(&self, row: HouseholdRow) -> Result<()>;
    async fn remove(&self, name: &str) -> Result<bool>;
    async fn list(&self) -> Result<Vec<HouseholdRow>>;
    async fn get(&self, name: &str) -> Result<Option<HouseholdRow>>;
}

#[derive(Debug, Clone)]
pub struct HouseholdRow {
    pub name: String,
    pub meta_repo_path: PathBuf,
    pub added_at: i64,
}

impl From<&HouseholdRow> for Household {
    fn from(r: &HouseholdRow) -> Self {
        Household {
            name: r.name.clone(),
            meta_repo_path: r.meta_repo_path.clone(),
        }
    }
}

pub(crate) struct SqliteHouseholdRepo {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteHouseholdRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl HouseholdRepo for SqliteHouseholdRepo {
    async fn insert(&self, row: HouseholdRow) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO households (name, meta_repo_path, added_at) VALUES (?1, ?2, ?3)",
                params![row.name, row.meta_repo_path.to_string_lossy(), row.added_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove(&self, name: &str) -> Result<bool> {
        let conn = self.conn.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = conn.blocking_lock();
            let n = conn.execute("DELETE FROM households WHERE name = ?1", [name])?;
            Ok(n > 0)
        })
        .await?
    }

    async fn list(&self) -> Result<Vec<HouseholdRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<HouseholdRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare("SELECT name, meta_repo_path, added_at FROM households ORDER BY name")?;
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
        })
        .await?
    }

    async fn get(&self, name: &str) -> Result<Option<HouseholdRow>> {
        let conn = self.conn.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<HouseholdRow>> {
            let conn = conn.blocking_lock();
            let row = conn
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
            Ok(row)
        })
        .await?
    }
}

/// The same SQLite repo also implements reeve_card's HouseholdLookup port.
/// Two trait views, one backing struct.
#[async_trait]
impl reeve_card::HouseholdLookup for SqliteHouseholdRepo {
    async fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.get(name).await?.is_some())
    }

    async fn card_paths(&self, name: &str) -> Result<reeve_card::CardPaths> {
        let row = self.get(name).await?.ok_or_else(|| {
            anyhow::anyhow!("household '{}' missing — cannot resolve card paths", name)
        })?;
        let household = crate::Household {
            name: row.name,
            meta_repo_path: row.meta_repo_path,
        };
        let manifest = crate::load_household_manifest(&household)?;
        Ok(reeve_card::CardPaths::from_optional(
            manifest.spec_path.as_deref(),
            manifest.plan_path.as_deref(),
        ))
    }
}

/// Factory returning both trait views from one underlying connection.
/// Reeved holds both Arcs; they point at the same SqliteHouseholdRepo instance.
pub fn make_household_components(
    conn: Arc<Mutex<Connection>>,
) -> (Arc<dyn HouseholdRepo>, Arc<dyn reeve_card::HouseholdLookup>) {
    let concrete = Arc::new(SqliteHouseholdRepo::new(conn));
    (concrete.clone(), concrete)
}

/// Initialise the households table. Called by reeved's migrate orchestrator.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS households (
            name           TEXT PRIMARY KEY,
            meta_repo_path TEXT NOT NULL,
            added_at       INTEGER NOT NULL
         );",
    )
    .context("init_schema for households")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<dyn HouseholdRepo> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (repo, _) = make_household_components(Arc::new(Mutex::new(conn)));
        repo
    }

    #[tokio::test]
    async fn insert_then_get() {
        let repo = fresh();
        repo.insert(HouseholdRow {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/x"),
            added_at: 1,
        })
        .await
        .unwrap();
        let got = repo.get("test").await.unwrap().unwrap();
        assert_eq!(got.name, "test");
    }

    #[tokio::test]
    async fn remove_returns_true_if_existed() {
        let repo = fresh();
        repo.insert(HouseholdRow {
            name: "x".into(),
            meta_repo_path: PathBuf::from("/x"),
            added_at: 1,
        })
        .await
        .unwrap();
        assert!(repo.remove("x").await.unwrap());
        assert!(!repo.remove("x").await.unwrap());
    }

    #[tokio::test]
    async fn lookup_impl_returns_existence() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (repo, lookup) = make_household_components(Arc::new(Mutex::new(conn)));
        repo.insert(HouseholdRow {
            name: "a".into(),
            meta_repo_path: PathBuf::from("/a"),
            added_at: 0,
        })
        .await
        .unwrap();
        assert!(lookup.exists("a").await.unwrap());
        assert!(!lookup.exists("nope").await.unwrap());
    }

    #[tokio::test]
    async fn card_paths_reads_manifest_fields() {
        use reeve_card::CardPaths;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{
              "meta_repo": "ws",
              "spec_path": "docs/foo/specs",
              "plan_path": "docs/foo/plans",
              "repos": [{ "name": "ws" }]
            }"#,
        )
        .unwrap();

        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (repo, lookup) = make_household_components(Arc::new(Mutex::new(conn)));
        repo.insert(HouseholdRow {
            name: "h".into(),
            meta_repo_path: meta.clone(),
            added_at: 0,
        })
        .await
        .unwrap();

        let paths = lookup.card_paths("h").await.unwrap();
        assert_eq!(
            paths,
            CardPaths {
                spec_path: "docs/foo/specs".into(),
                plan_path: "docs/foo/plans".into(),
            }
        );
    }

    #[tokio::test]
    async fn card_paths_returns_defaults_when_manifest_omits_them() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let meta = dir.path().join("ws");
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(
            meta.join("household.json"),
            r#"{ "meta_repo": "ws", "repos": [{ "name": "ws" }] }"#,
        )
        .unwrap();

        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (repo, lookup) = make_household_components(Arc::new(Mutex::new(conn)));
        repo.insert(HouseholdRow {
            name: "h".into(),
            meta_repo_path: meta.clone(),
            added_at: 0,
        })
        .await
        .unwrap();

        let paths = lookup.card_paths("h").await.unwrap();
        assert_eq!(paths.spec_path, "docs/agent/specs");
        assert_eq!(paths.plan_path, "docs/agent/plans");
    }

    #[tokio::test]
    async fn card_paths_errors_when_household_missing() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let (_repo, lookup) = make_household_components(Arc::new(Mutex::new(conn)));
        let err = lookup.card_paths("missing").await.unwrap_err();
        assert!(format!("{err:#}").contains("missing"));
    }
}
