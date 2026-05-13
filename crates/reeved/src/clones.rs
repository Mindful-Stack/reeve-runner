//! Per-card repo clones. Implements the `--reference --dissociate` topology
//! from the parent design: `git clone --reference <host> --dissociate <host>
//! <card-workspace>/<repo-name>/`. The clone borrows objects from the host at
//! clone time (cheap) then severs the dependency so the card survives
//! host-side `git gc --prune=now` or rebase drops.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tracing::warn;

/// Spec for one repo to clone into a card workspace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RepoSpec {
    pub name: String,
    pub path: PathBuf,
}

/// Abstract git-subprocess backend. The trait exists so unit tests can mock
/// out the subprocess and assert on the argv that would have been issued.
#[async_trait]
pub trait GitOps: Send + Sync {
    /// Run `git clone --reference <source> --dissociate <source> <dest>`.
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()>;
}

/// Convenience handle used by the rest of the daemon.
pub type GitOpsHandle = Arc<dyn GitOps>;

/// Real implementation. Shells out to `git` via tokio.
pub struct RealGitOps;

#[async_trait]
impl GitOps for RealGitOps {
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()> {
        // `--reference X --dissociate X` is by design: `--reference` borrows
        // objects from X (cheap clone), trailing X is the clone source URL.
        // Both point at the host repo; `--dissociate` then severs the borrow.
        let output = Command::new("git")
            .args([
                "clone",
                "--reference",
                source.to_str().context("source path not utf-8")?,
                "--dissociate",
                source.to_str().context("source path not utf-8")?,
                dest.to_str().context("dest path not utf-8")?,
            ])
            .output()
            .await
            .context("exec git clone")?;
        if !output.status.success() {
            anyhow::bail!(
                "git clone failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}

/// Clone every repo in `repos` into `<workspace>/<repo.name>/` serially.
/// On any clone failure, removes the partial workspace and returns the error.
/// Caller is responsible for creating the workspace dir before this call.
pub async fn clone_all_repos(ops: &dyn GitOps, workspace: &Path, repos: &[RepoSpec]) -> Result<()> {
    for repo in repos {
        let dest = workspace.join(&repo.name);
        if let Err(e) = ops.clone(&repo.path, &dest).await {
            // Best-effort cleanup of the partial workspace.
            if let Err(rm_err) = std::fs::remove_dir_all(workspace) {
                warn!(
                    "failed to clean partial workspace {}: {rm_err:#}",
                    workspace.display()
                );
            }
            return Err(e.context(format!("clone repo '{}'", repo.name)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Records every `clone` call. Configurable failure on the Nth call.
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fail_after: Option<usize>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fail_after: None,
            }
        }

        fn failing_after(n: usize) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fail_after: Some(n),
            }
        }
    }

    #[async_trait]
    impl GitOps for MockGitOps {
        async fn clone(&self, source: &Path, dest: &Path) -> Result<()> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((source.to_path_buf(), dest.to_path_buf()));
            if let Some(n) = self.fail_after {
                if calls.len() > n {
                    anyhow::bail!("simulated clone failure");
                }
            }
            // Pretend the clone succeeded by creating the dest dir.
            std::fs::create_dir_all(dest).unwrap();
            Ok(())
        }
    }

    fn repo(name: &str, path: &Path) -> RepoSpec {
        RepoSpec {
            name: name.into(),
            path: path.to_path_buf(),
        }
    }

    #[tokio::test]
    async fn clone_all_repos_issues_one_clone_per_repo() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::new();
        let host_a = dir.path().join("host-a");
        let host_b = dir.path().join("host-b");
        clone_all_repos(&ops, &workspace, &[repo("a", &host_a), repo("b", &host_b)])
            .await
            .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, host_a);
        assert_eq!(calls[0].1, workspace.join("a"));
        assert_eq!(calls[1].0, host_b);
        assert_eq!(calls[1].1, workspace.join("b"));
    }

    #[tokio::test]
    async fn clone_all_repos_with_empty_repos_is_noop() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::new();
        clone_all_repos(&ops, &workspace, &[]).await.unwrap();
        assert_eq!(ops.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn clone_all_repos_removes_workspace_on_failure() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::failing_after(1);
        let host_a = dir.path().join("host-a");
        let host_b = dir.path().join("host-b");
        let result =
            clone_all_repos(&ops, &workspace, &[repo("a", &host_a), repo("b", &host_b)]).await;
        assert!(result.is_err());
        assert!(
            !workspace.exists(),
            "workspace should be removed after clone failure",
        );
    }

    #[tokio::test]
    async fn clone_all_repos_serial_order_preserved() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("card-1");
        std::fs::create_dir_all(&workspace).unwrap();
        let ops = MockGitOps::new();
        let host_a = dir.path().join("a");
        let host_b = dir.path().join("b");
        let host_c = dir.path().join("c");
        clone_all_repos(
            &ops,
            &workspace,
            &[repo("a", &host_a), repo("b", &host_b), repo("c", &host_c)],
        )
        .await
        .unwrap();
        let calls = ops.calls.lock().unwrap();
        let names: Vec<&str> = calls
            .iter()
            .map(|(s, _)| s.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}

#[cfg(test)]
mod integration_tests {
    //! Real-git tests. Require `git` in $PATH. Hit the filesystem.
    use super::*;
    use tempfile::tempdir;

    fn git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invoke");
        assert!(
            status.success(),
            "git {args:?} failed in {}",
            repo.display()
        );
    }

    /// Initialise a host repo with one commit. Returns the repo path.
    fn host_repo(dir: &Path, name: &str) -> PathBuf {
        let repo = dir.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "test"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        repo
    }

    #[tokio::test]
    async fn real_clone_produces_dissociated_clone() {
        let dir = tempdir().unwrap();
        let source = host_repo(dir.path(), "host");
        let dest = dir.path().join("dest");

        let ops = RealGitOps;
        ops.clone(&source, &dest).await.unwrap();

        // Dest exists and has a .git directory.
        assert!(dest.join(".git").exists());
        // `objects/info/alternates` must NOT exist — that's the file
        // --dissociate is supposed to remove. Its absence is the proof
        // the dependency on the host repo was severed.
        let alternates = dest.join(".git/objects/info/alternates");
        assert!(
            !alternates.exists(),
            "alternates file present — clone was not dissociated",
        );
        // git log should show the commit we created in the host.
        let log = std::process::Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(&dest)
            .output()
            .unwrap();
        assert!(log.status.success());
        let stdout = String::from_utf8_lossy(&log.stdout);
        assert!(stdout.contains("initial"), "unexpected log: {stdout}");
    }

    #[tokio::test]
    async fn real_clone_two_repos_via_clone_all_repos() {
        let dir = tempdir().unwrap();
        let a = host_repo(dir.path(), "host-a");
        let b = host_repo(dir.path(), "host-b");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let ops = RealGitOps;
        clone_all_repos(
            &ops,
            &workspace,
            &[
                RepoSpec {
                    name: "a".into(),
                    path: a,
                },
                RepoSpec {
                    name: "b".into(),
                    path: b,
                },
            ],
        )
        .await
        .unwrap();

        assert!(workspace.join("a/.git").exists());
        assert!(workspace.join("b/.git").exists());
    }

    #[tokio::test]
    async fn real_clone_fails_when_source_is_not_a_repo() {
        let dir = tempdir().unwrap();
        let not_a_repo = dir.path().join("garbage");
        std::fs::create_dir_all(&not_a_repo).unwrap();
        let dest = dir.path().join("dest");

        let ops = RealGitOps;
        let result = ops.clone(&not_a_repo, &dest).await;
        assert!(result.is_err(), "clone of non-repo should fail");
    }
}
