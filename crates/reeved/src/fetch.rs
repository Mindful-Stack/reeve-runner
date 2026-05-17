//! M4 fetch worker. Periodically fetches each running card's clones into
//! `refs/reeve/<card-id>/*` on the host repos, surfacing in-container
//! commits in the user's git tooling without modifying `refs/heads/*`.

use crate::effects::DaemonState;
use anyhow::Result;
use reeve_protocol::ContainerStatus;
use std::time::Duration;
use tracing::warn;

/// Hardcoded for v1; configurable later.
pub const FETCH_INTERVAL: Duration = Duration::from_secs(5);

/// Run one fetch iteration: for each `Running` or `Spawning` card, walk
/// its meta-repo + manifest repos and fetch from the card workspace into
/// `refs/reeve/<card-id>/*` on the host repo. Per-fetch failures log at
/// `warn` and continue; this function never returns `Err` from a fetch
/// failure (the DB iteration may still error).
pub async fn tick(state: &DaemonState) -> Result<()> {
    let containers = {
        let db = state.db.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<crate::db::ContainerRow>> {
            let conn = db.blocking_lock();
            crate::db::list_containers(&conn)
        })
        .await??
    };

    for container in containers {
        if !matches!(container.status, ContainerStatus::Running | ContainerStatus::Spawning) {
            continue;
        }
        if let Err(e) = tick_one_card(state, &container.card_id).await {
            warn!("fetch tick for card {}: {e:#}", container.card_id);
        }
    }
    Ok(())
}

/// Fetch for a single card. Logs per-fetch errors at warn; returns Err only
/// on DB I/O failure or task panic. Missing card, missing household, and
/// broken manifest are warn-and-continue rather than errors.
async fn tick_one_card(state: &DaemonState, card_id: &str) -> Result<()> {
    // Look up the card to find its household.
    let card_id_for_query = card_id.to_string();
    let db = state.db.clone();
    let card = tokio::task::spawn_blocking(move || -> Result<Option<reeve_protocol::Card>> {
        let conn = db.blocking_lock();
        crate::db::get_card(&conn, &card_id_for_query)
    })
    .await??;
    let Some(card) = card else {
        warn!("fetch tick: card {} not found in DB", card_id);
        return Ok(());
    };

    // Look up the household.
    let household_name = card.household.clone();
    let db = state.db.clone();
    let row = tokio::task::spawn_blocking(move || -> Result<Option<crate::db::HouseholdRow>> {
        let conn = db.blocking_lock();
        crate::db::get_household(&conn, &household_name)
    })
    .await??;
    let Some(row) = row else {
        warn!("fetch tick: household '{}' for card {} not configured", card.household, card_id);
        return Ok(());
    };
    let household = crate::household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };

    // Load the manifest. Failure to parse skips this card until the user fixes
    // household.json; next tick re-tries.
    let manifest = match crate::household::load_household_manifest(&household) {
        Ok(m) => m,
        Err(e) => {
            warn!("fetch tick: manifest for card {}: {e:#}", card_id);
            return Ok(());
        }
    };

    let workspace = reeve_protocol::paths::card_workspace_path(card_id);
    for (repo_name, host_path) in crate::household::iter_card_repos(&household, &manifest) {
        // The card-side source path mirrors the host topology:
        //   - workspace repo: <workspace>/
        //   - sibling repo:   <workspace>/<name>/
        let card_clone = if repo_name == manifest.workspace {
            workspace.clone()
        } else {
            workspace.join(repo_name)
        };
        if let Err(e) = state.git.fetch(&host_path, &card_clone, card_id).await {
            warn!("fetch {}/{}: {e:#}", card_id, repo_name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clones::GitOps;
    use std::sync::Arc;

    /// Mock that records every fetch call.
    struct RecordingGit {
        calls: std::sync::Mutex<Vec<(std::path::PathBuf, std::path::PathBuf, String)>>,
    }

    impl RecordingGit {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::new(vec![]) }
        }
    }

    #[async_trait::async_trait]
    impl GitOps for RecordingGit {
        async fn clone(&self, _s: &std::path::Path, _d: &std::path::Path) -> Result<()> { Ok(()) }
        async fn clone_url(&self, _u: &str, _d: &std::path::Path) -> Result<()> { Ok(()) }
        async fn fetch(
            &self,
            workdir: &std::path::Path,
            source: &std::path::Path,
            namespace: &str,
        ) -> Result<()> {
            self.calls.lock().unwrap().push((
                workdir.to_path_buf(),
                source.to_path_buf(),
                namespace.to_string(),
            ));
            Ok(())
        }
        async fn list_namespace_refs(
            &self,
            _workdir: &std::path::Path,
            _namespace: &str,
        ) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn fast_forward(
            &self,
            _workdir: &std::path::Path,
            _target: &str,
            _source: &str,
        ) -> Result<crate::clones::FfOutcome> {
            Ok(crate::clones::FfOutcome::Updated)
        }
        async fn push(&self, _workdir: &std::path::Path, _branch: &str) -> Result<()> { Ok(()) }
    }

    struct NoopOps;
    #[async_trait::async_trait]
    impl crate::container::ContainerOps for NoopOps {
        async fn spawn(&self, _argv: Vec<String>) -> Result<()> { Ok(()) }
        async fn teardown(&self, _name: &str) -> Result<()> { Ok(()) }
        async fn inspect_status(&self, _name: &str) -> Result<Option<String>> { Ok(None) }
        async fn discover_container_name(&self, _card_id: &str) -> Result<Option<String>> { Ok(None) }
    }

    struct NoopDispatcher;
    impl crate::listeners::Dispatcher for NoopDispatcher {
        fn dispatch(
            &self,
            _req: reeve_protocol::RpcRequest,
            _db: crate::server::DbHandle,
            _caller: crate::server::CallerContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<serde_json::Value, reeve_protocol::RpcError>> + Send>> {
            Box::pin(async { Ok(serde_json::Value::Null) })
        }
    }

    fn write_manifest(meta: &std::path::Path, workspace: &str, repos: &[&str]) {
        std::fs::create_dir_all(meta).unwrap();
        let repos_json: Vec<String> = repos.iter().map(|n| format!(r#"{{"name":"{n}"}}"#)).collect();
        let body = format!(r#"{{"workspace":"{workspace}","repos":[{}]}}"#, repos_json.join(","));
        std::fs::write(meta.join("household.json"), body).unwrap();
    }

    fn state_with(git: Arc<RecordingGit>) -> DaemonState {
        let conn = crate::db::open(":memory:").unwrap();
        let db: crate::server::DbHandle = Arc::new(tokio::sync::Mutex::new(conn));
        DaemonState {
            db: db.clone(),
            containers: Arc::new(NoopOps) as crate::container::ContainerOpsHandle,
            git: git as crate::clones::GitOpsHandle,
            listeners: crate::listeners::ListenerRegistry::new(db, Arc::new(NoopDispatcher)),
            claude_dir: std::path::PathBuf::from("/tmp/nonexistent-test-claude-dir"),
            api_key: None,
        }
    }

    #[tokio::test]
    async fn tick_fetches_once_per_running_card_and_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws", "backend"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git.clone());

        {
            let conn = state.db.lock().await;
            crate::db::insert_household(
                &conn,
                &crate::db::HouseholdRow {
                    name: "h".into(),
                    meta_repo_path: meta.clone(),
                    added_at: 0,
                },
            )
            .unwrap();
            crate::db::insert_card(
                &conn,
                &reeve_protocol::Card {
                    id: "card-a".into(),
                    household: "h".into(),
                    brief: "a".into(),
                    column: reeve_protocol::Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
            crate::db::upsert_container(
                &conn,
                "card-a",
                "container-a",
                ContainerStatus::Running,
                0,
            )
            .unwrap();
        }

        tick(&state).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "one fetch per (running card × repo): {calls:?}");
        // Workspace first (host = meta, card_clone = workspace root), then
        // sibling (host = meta/backend, card_clone = workspace/backend).
        let workspace = reeve_protocol::paths::card_workspace_path("card-a");
        assert_eq!(calls[0].2, "card-a");
        assert_eq!(calls[0].0, meta);
        assert_eq!(calls[0].1, workspace);
        assert_eq!(calls[1].2, "card-a");
        assert_eq!(calls[1].0, meta.join("backend"));
        assert_eq!(calls[1].1, workspace.join("backend"));
    }

    #[tokio::test]
    async fn tick_skips_non_running_containers() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git.clone());

        {
            let conn = state.db.lock().await;
            crate::db::insert_household(
                &conn,
                &crate::db::HouseholdRow {
                    name: "h".into(),
                    meta_repo_path: meta.clone(),
                    added_at: 0,
                },
            )
            .unwrap();
            crate::db::insert_card(
                &conn,
                &reeve_protocol::Card {
                    id: "card-b".into(),
                    household: "h".into(),
                    brief: "b".into(),
                    column: reeve_protocol::Column::Implementing,
                    created_at: 0,
                    updated_at: 0,
                    needs_heavy: false,
                    parent_id: None,
                },
            )
            .unwrap();
            crate::db::upsert_container(
                &conn,
                "card-b",
                "container-b",
                ContainerStatus::Failed,
                0,
            )
            .unwrap();
        }

        tick(&state).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert!(calls.is_empty(), "Failed containers should be skipped");
    }

    #[tokio::test]
    async fn tick_continues_after_missing_household() {
        let dir = tempfile::tempdir().unwrap();
        let meta_b = dir.path().join("ws-b");
        write_manifest(&meta_b, "ws-b", &["ws-b"]);

        let git = Arc::new(RecordingGit::new());
        let state = state_with(git.clone());

        {
            let conn = state.db.lock().await;
            // Household "b" exists; "a" does not.
            crate::db::insert_household(
                &conn,
                &crate::db::HouseholdRow {
                    name: "b".into(),
                    meta_repo_path: meta_b.clone(),
                    added_at: 0,
                },
            )
            .unwrap();
            for (id, hh) in [("card-a", "a-missing"), ("card-b", "b")] {
                crate::db::insert_card(
                    &conn,
                    &reeve_protocol::Card {
                        id: id.into(),
                        household: hh.into(),
                        brief: id.into(),
                        column: reeve_protocol::Column::Implementing,
                        created_at: 0,
                        updated_at: 0,
                        needs_heavy: false,
                        parent_id: None,
                    },
                )
                .unwrap();
                crate::db::upsert_container(&conn, id, &format!("c-{id}"), ContainerStatus::Running, 0)
                    .unwrap();
            }
        }

        tick(&state).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only card-b should produce a fetch");
        assert_eq!(calls[0].2, "card-b");
    }
}
