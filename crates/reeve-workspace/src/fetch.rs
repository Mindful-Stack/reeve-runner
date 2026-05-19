//! M4 fetch worker. Periodically fetches each running card's clones into
//! `refs/reeve/<card-id>/*` on the host repos, surfacing in-container
//! commits in the user's git tooling without modifying `refs/heads/*`.

use crate::git::GitOpsHandle;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

/// Hardcoded for v1; configurable later.
pub const FETCH_INTERVAL: Duration = Duration::from_secs(5);

/// A running card entry as returned by `FetchDeps::list_running_cards`.
pub struct RunningCard {
    pub card_id: String,
    pub household: String,
}

/// Household-level information needed by the fetch worker.
pub struct HouseholdInfo {
    pub name: String,
    pub meta_repo_path: PathBuf,
}

/// Abstract interface over the stateful dependencies the fetch worker needs.
/// Implemented by `reeved::effects::DaemonState` (and by test doubles).
#[async_trait::async_trait]
pub trait FetchDeps: Send + Sync {
    /// List cards that are currently Running or Spawning.
    async fn list_running_cards(&self) -> Result<Vec<RunningCard>>;

    /// Look up a household by name. Returns `None` if not registered.
    async fn get_household(&self, name: &str) -> Result<Option<HouseholdInfo>>;

    /// The git backend to use for fetching.
    fn git_ops(&self) -> GitOpsHandle;
}

/// Run one fetch iteration: for each `Running` or `Spawning` card, walk
/// its meta-repo + manifest repos and fetch from the card workspace into
/// `refs/reeve/<card-id>/*` on the host repo. Per-fetch failures log at
/// `warn` and continue; this function never returns `Err` from a fetch
/// failure (the DB iteration may still error).
pub async fn tick(state: &dyn FetchDeps) -> Result<()> {
    let cards = state.list_running_cards().await?;

    for card in cards {
        if let Err(e) = tick_one_card(state, &card.card_id, &card.household).await {
            warn!("fetch tick for card {}: {e:#}", card.card_id);
        }
    }
    Ok(())
}

/// Fetch for a single card. Logs per-fetch errors at warn; returns Err only
/// on household lookup failure. Missing household and broken manifest are
/// warn-and-continue rather than errors.
async fn tick_one_card(
    state: &dyn FetchDeps,
    card_id: &str,
    household_name: &str,
) -> Result<()> {
    // Look up the household.
    let row = state.get_household(household_name).await?;
    let Some(row) = row else {
        warn!(
            "fetch tick: household '{}' for card {} not configured",
            household_name, card_id
        );
        return Ok(());
    };
    let household = reeve_household::Household {
        name: row.name,
        meta_repo_path: row.meta_repo_path,
    };

    // Load the manifest. Failure to parse skips this card until the user fixes
    // household.json; next tick re-tries.
    let manifest = match reeve_household::load_household_manifest(&household) {
        Ok(m) => m,
        Err(e) => {
            warn!("fetch tick: manifest for card {}: {e:#}", card_id);
            return Ok(());
        }
    };

    let workspace = reeve_protocol::paths::card_workspace_path(card_id);
    let git = state.git_ops();
    for (repo_name, host_path) in reeve_household::iter_card_repos(&household, &manifest) {
        // The card-side source path mirrors the host topology:
        //   - workspace repo: <workspace>/
        //   - sibling repo:   <workspace>/<name>/
        let card_clone = if repo_name == manifest.meta_repo {
            workspace.clone()
        } else {
            workspace.join(repo_name)
        };
        if let Err(e) = git.fetch(&host_path, &card_clone, card_id).await {
            warn!("fetch {}/{}: {e:#}", card_id, repo_name);
        }
    }
    Ok(())
}

/// Start the fetch worker loop. Returns when the shutdown receiver fires.
/// Each iteration calls `tick(state)`; sleeps `FETCH_INTERVAL` between
/// iterations. Either the timer or shutdown can win the `select!`.
pub async fn run(
    state: impl FetchDeps + 'static,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("fetch worker: shutdown received");
                return;
            }
            _ = tokio::time::sleep(FETCH_INTERVAL) => {
                if let Err(e) = tick(&state).await {
                    tracing::warn!("fetch worker tick: {e:#}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{FfOutcome, GitOps};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Mock git that records every fetch call.
    struct RecordingGit {
        calls: Mutex<Vec<(PathBuf, PathBuf, String)>>,
    }

    impl RecordingGit {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait::async_trait]
    impl GitOps for RecordingGit {
        async fn clone(&self, _s: &Path, _d: &Path) -> Result<()> {
            Ok(())
        }
        async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()> {
            self.calls.lock().unwrap().push((
                workdir.to_path_buf(),
                source.to_path_buf(),
                namespace.to_string(),
            ));
            Ok(())
        }
        async fn list_namespace_refs(&self, _workdir: &Path, _namespace: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn fast_forward(
            &self,
            _workdir: &Path,
            _target: &str,
            _source: &str,
        ) -> Result<FfOutcome> {
            Ok(FfOutcome::Updated)
        }
        async fn push(&self, _workdir: &Path, _branch: &str) -> Result<()> {
            Ok(())
        }
    }

    /// Test double for FetchDeps.
    struct TestDeps {
        cards: Mutex<Vec<(String, String)>>, // (card_id, household_name)
        households: Mutex<std::collections::HashMap<String, PathBuf>>,
        git: Arc<RecordingGit>,
    }

    impl TestDeps {
        fn new(git: Arc<RecordingGit>) -> Self {
            Self {
                cards: Mutex::new(vec![]),
                households: Mutex::new(std::collections::HashMap::new()),
                git,
            }
        }

        fn add_card(&self, card_id: &str, household: &str) {
            self.cards
                .lock()
                .unwrap()
                .push((card_id.to_string(), household.to_string()));
        }

        fn add_household(&self, name: &str, path: PathBuf) {
            self.households.lock().unwrap().insert(name.to_string(), path);
        }
    }

    #[async_trait::async_trait]
    impl FetchDeps for TestDeps {
        async fn list_running_cards(&self) -> Result<Vec<RunningCard>> {
            Ok(self
                .cards
                .lock()
                .unwrap()
                .iter()
                .map(|(id, hh)| RunningCard {
                    card_id: id.clone(),
                    household: hh.clone(),
                })
                .collect())
        }

        async fn get_household(&self, name: &str) -> Result<Option<HouseholdInfo>> {
            Ok(self
                .households
                .lock()
                .unwrap()
                .get(name)
                .map(|p| HouseholdInfo {
                    name: name.to_string(),
                    meta_repo_path: p.clone(),
                }))
        }

        fn git_ops(&self) -> GitOpsHandle {
            self.git.clone()
        }
    }

    fn write_manifest(meta: &Path, workspace: &str, repos: &[&str]) {
        std::fs::create_dir_all(meta).unwrap();
        let repos_json: Vec<String> = repos
            .iter()
            .map(|n| format!(r#"{{"name":"{n}"}}"#))
            .collect();
        let body = format!(
            r#"{{"workspace":"{workspace}","repos":[{}]}}"#,
            repos_json.join(",")
        );
        std::fs::write(meta.join("household.json"), body).unwrap();
    }

    #[tokio::test]
    async fn tick_fetches_once_per_running_card_and_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws", "backend"]);

        let git = Arc::new(RecordingGit::new());
        let deps = TestDeps::new(git.clone());

        deps.add_card("card-a", "h");
        deps.add_household("h", meta.clone());

        tick(&deps).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            2,
            "one fetch per (running card × repo): {calls:?}"
        );
        let workspace = reeve_protocol::paths::card_workspace_path("card-a");
        assert_eq!(calls[0].2, "card-a");
        assert_eq!(calls[0].0, meta);
        assert_eq!(calls[0].1, workspace);
        assert_eq!(calls[1].2, "card-a");
        assert_eq!(calls[1].0, meta.join("backend"));
        assert_eq!(calls[1].1, workspace.join("backend"));
    }

    #[tokio::test]
    async fn tick_continues_after_missing_household() {
        let dir = tempfile::tempdir().unwrap();
        let meta_b = dir.path().join("ws-b");
        write_manifest(&meta_b, "ws-b", &["ws-b"]);

        let git = Arc::new(RecordingGit::new());
        let deps = TestDeps::new(git.clone());

        // Household "b" exists; "a" does not.
        deps.add_card("card-a", "a-missing");
        deps.add_card("card-b", "b");
        deps.add_household("b", meta_b.clone());

        tick(&deps).await.unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only card-b should produce a fetch");
        assert_eq!(calls[0].2, "card-b");
    }

    #[tokio::test]
    async fn run_exits_on_shutdown_signal() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("ws");
        write_manifest(&meta, "ws", &["ws"]);

        let git = Arc::new(RecordingGit::new());
        let deps = TestDeps::new(git);
        let (tx, rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(super::run(deps, rx));

        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(()).unwrap();

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("worker did not exit within 1 second")
            .expect("worker panicked");
    }
}
