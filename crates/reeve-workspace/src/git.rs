//! Git port + RealGitOps adapter.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;

/// Result of attempting a fast-forward update via `GitOps::fast_forward`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfOutcome {
    /// `target_ref` was updated to point at `source_ref`.
    Updated,
    /// `target_ref` is not an ancestor of `source_ref`; left unchanged.
    NotFastForward,
}

/// Abstract git-subprocess backend. The trait exists so unit tests can mock
/// out the subprocess and assert on the argv that would have been issued.
#[async_trait]
pub trait GitOps: Send + Sync {
    /// Run `git clone --reference <source> --dissociate <source> <dest>`.
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()>;
    /// Run `git fetch <source> '+refs/heads/*:refs/reeve/<namespace>/*'` in `workdir`.
    /// The leading `+` allows non-fast-forward updates so re-fetched namespace
    /// refs are always overwritten.
    /// Used by the M4 fetch worker to surface in-container commits on the host.
    /// `namespace` must be a valid git ref component (e.g. a UUID); the caller
    /// is responsible for ensuring it contains no whitespace or `..` sequences.
    async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()>;
    /// Return branch names found under `refs/reeve/<namespace>/`. The
    /// namespace prefix is stripped; consumers see plain branch names.
    /// Empty vec when there are no refs in that namespace.
    async fn list_namespace_refs(&self, workdir: &Path, namespace: &str) -> Result<Vec<String>>;
    /// Update `target_ref` to point at `source_ref` only if the move is a
    /// fast-forward (target is an ancestor of source). Returns `Updated` on
    /// success or `NotFastForward` if the move would lose history. Other
    /// failures (git crash, ref doesn't exist, etc.) return `Err`.
    async fn fast_forward(
        &self,
        workdir: &Path,
        target_ref: &str,
        source_ref: &str,
    ) -> Result<FfOutcome>;
    /// Run `git push origin <branch>` in `workdir`. The remote `origin`
    /// must be configured on the host repo; absence is reported as an
    /// `Err` returned from this method.
    async fn push(&self, workdir: &Path, branch: &str) -> Result<()>;
}

/// Convenience handle used by the rest of the daemon.
pub type GitOpsHandle = Arc<dyn GitOps>;

/// Real implementation. Shells out to `git` via tokio.
pub(crate) struct RealGitOps;

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

    async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()> {
        let refspec = format!("+refs/heads/*:refs/reeve/{namespace}/*");
        let output = Command::new("git")
            .args([
                "fetch",
                source.to_str().context("source path not utf-8")?,
                &refspec,
            ])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git fetch")?;
        if !output.status.success() {
            anyhow::bail!(
                "git fetch failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn list_namespace_refs(&self, workdir: &Path, namespace: &str) -> Result<Vec<String>> {
        let prefix = format!("refs/reeve/{namespace}/");
        let output = Command::new("git")
            .args(["for-each-ref", "--format=%(refname)", &prefix])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git for-each-ref")?;
        if !output.status.success() {
            anyhow::bail!(
                "git for-each-ref failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let branches: Vec<String> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix).map(|s| s.to_string()))
            .collect();
        Ok(branches)
    }

    async fn fast_forward(
        &self,
        workdir: &Path,
        target_ref: &str,
        source_ref: &str,
    ) -> Result<FfOutcome> {
        // `git merge-base --is-ancestor` returns exit 0 if ancestor, exit 1 if not,
        // exit >1 on a real error (e.g. one ref doesn't exist).
        // If target_ref doesn't exist yet (newly-discovered branch), is-ancestor
        // returns >1 too. We treat that case as "FF allowed" — there's no
        // history to lose. Distinguish that via a separate `git rev-parse`
        // existence probe before running is-ancestor.
        let target_exists = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", target_ref])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git rev-parse for target_ref")?;

        if target_exists.status.success() {
            // Target exists; require strict FF.
            let ancestor = Command::new("git")
                .args(["merge-base", "--is-ancestor", target_ref, source_ref])
                .current_dir(workdir)
                .output()
                .await
                .context("exec git merge-base --is-ancestor")?;
            // Exit 0 = ancestor (FF OK); exit 1 = not ancestor (NotFF);
            // any other = real error.
            match ancestor.status.code() {
                Some(0) => {}
                Some(1) => return Ok(FfOutcome::NotFastForward),
                other => anyhow::bail!(
                    "git merge-base --is-ancestor exited {:?}: {}",
                    other,
                    String::from_utf8_lossy(&ancestor.stderr)
                ),
            }
        }
        // Either target didn't exist (new branch) or FF was OK — update.
        let update = Command::new("git")
            .args(["update-ref", target_ref, source_ref])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git update-ref")?;
        if !update.status.success() {
            anyhow::bail!(
                "git update-ref failed (exit {}): {}",
                update.status,
                String::from_utf8_lossy(&update.stderr)
            );
        }
        Ok(FfOutcome::Updated)
    }

    async fn push(&self, workdir: &Path, branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["push", "origin", branch])
            .current_dir(workdir)
            .output()
            .await
            .context("exec git push")?;
        if !output.status.success() {
            anyhow::bail!(
                "git push failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}

/// Factory: the only public way to obtain a `GitOpsHandle` backed by the real
/// git subprocess. `RealGitOps` itself is `pub(crate)`.
pub fn make_git_ops() -> Arc<dyn GitOps> {
    Arc::new(RealGitOps)
}

#[cfg(test)]
mod integration_tests {
    //! Real-git tests. Require `git` in $PATH. Hit the filesystem.
    use super::*;
    use std::path::PathBuf;
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
    /// Always creates the default branch as `main` regardless of the
    /// system-wide `init.defaultBranch` setting.
    fn host_repo(dir: &Path, name: &str) -> PathBuf {
        let repo = dir.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["-c", "init.defaultBranch=main", "init", "--quiet"]);
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
    async fn real_clone_fails_when_source_is_not_a_repo() {
        let dir = tempdir().unwrap();
        let not_a_repo = dir.path().join("garbage");
        std::fs::create_dir_all(&not_a_repo).unwrap();
        let dest = dir.path().join("dest");

        let ops = RealGitOps;
        let result = ops.clone(&not_a_repo, &dest).await;
        assert!(result.is_err(), "clone of non-repo should fail");
    }

    #[tokio::test]
    async fn real_list_namespace_refs_returns_branches() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Manually create two refs in the namespace.
        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(head.status.success());
        let head_sha = String::from_utf8_lossy(&head.stdout).trim().to_string();

        for branch in ["feat-a", "feat-b"] {
            let refname = format!("refs/reeve/card-x/{branch}");
            let status = std::process::Command::new("git")
                .args(["update-ref", &refname, &head_sha])
                .current_dir(&host)
                .status()
                .unwrap();
            assert!(status.success());
        }

        let ops = RealGitOps;
        let mut branches = ops.list_namespace_refs(&host, "card-x").await.unwrap();
        branches.sort();
        assert_eq!(branches, vec!["feat-a".to_string(), "feat-b".to_string()]);
    }

    #[tokio::test]
    async fn real_list_namespace_refs_returns_empty_when_no_match() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");
        let ops = RealGitOps;
        let result = ops
            .list_namespace_refs(&host, "missing-card")
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn real_fetch_populates_namespaced_refs() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");
        let card_clone = dir.path().join("card-clone");

        // Clone the host repo as the card workspace would.
        let ops = RealGitOps;
        ops.clone(&host, &card_clone).await.unwrap();

        // Make a new commit in the card clone on a feature branch.
        git(&card_clone, &["checkout", "-b", "feat-x"]);
        std::fs::write(card_clone.join("new.txt"), "hi\n").unwrap();
        git(&card_clone, &["add", "new.txt"]);
        git(
            &card_clone,
            &[
                "-c",
                "user.email=t@t.com",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "x",
            ],
        );

        // Fetch from host's perspective, populating refs/reeve/<id>/.
        ops.fetch(&host, &card_clone, "card-deadbeef")
            .await
            .unwrap();

        // Assert host now has refs/reeve/card-deadbeef/feat-x pointing at the new commit.
        let output = std::process::Command::new("git")
            .args(["rev-parse", "refs/reeve/card-deadbeef/feat-x"])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "rev-parse should find namespaced ref: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn real_fast_forward_advances_target_when_ancestor() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Set up a `refs/reeve/card-x/main` one commit ahead of `refs/heads/main`.
        std::fs::write(host.join("a.txt"), "a\n").unwrap();
        git(&host, &["add", "a.txt"]);
        git(
            &host,
            &[
                "-c",
                "user.email=t@t.com",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "a",
            ],
        );
        let advance_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        let advance_sha = String::from_utf8_lossy(&advance_sha.stdout)
            .trim()
            .to_string();
        git(
            &host,
            &["update-ref", "refs/reeve/card-x/main", &advance_sha],
        );
        // Rewind heads/main to the previous commit (HEAD~1).
        git(&host, &["update-ref", "refs/heads/main", "HEAD~1"]);

        let ops = RealGitOps;
        let outcome = ops
            .fast_forward(&host, "refs/heads/main", "refs/reeve/card-x/main")
            .await
            .unwrap();
        assert_eq!(outcome, FfOutcome::Updated);

        let after = std::process::Command::new("git")
            .args(["rev-parse", "refs/heads/main"])
            .current_dir(&host)
            .output()
            .unwrap();
        let after_sha = String::from_utf8_lossy(&after.stdout).trim().to_string();
        assert_eq!(
            after_sha, advance_sha,
            "main advanced to the namespaced ref"
        );
    }

    #[tokio::test]
    async fn real_fast_forward_rejects_non_ff() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        git(&host, &["checkout", "-b", "parallel"]);
        std::fs::write(host.join("p.txt"), "p\n").unwrap();
        git(&host, &["add", "p.txt"]);
        git(
            &host,
            &[
                "-c",
                "user.email=t@t.com",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "parallel",
            ],
        );
        let parallel_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        let parallel_sha = String::from_utf8_lossy(&parallel_sha.stdout)
            .trim()
            .to_string();
        git(&host, &["checkout", "main"]);
        std::fs::write(host.join("m.txt"), "m\n").unwrap();
        git(&host, &["add", "m.txt"]);
        git(
            &host,
            &[
                "-c",
                "user.email=t@t.com",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "main2",
            ],
        );
        git(
            &host,
            &["update-ref", "refs/reeve/card-x/main", &parallel_sha],
        );

        let ops = RealGitOps;
        let outcome = ops
            .fast_forward(&host, "refs/heads/main", "refs/reeve/card-x/main")
            .await
            .unwrap();
        assert_eq!(outcome, FfOutcome::NotFastForward);
    }

    #[tokio::test]
    async fn real_fast_forward_creates_new_branch_target() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        let head_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&host)
            .output()
            .unwrap();
        let head_sha = String::from_utf8_lossy(&head_sha.stdout).trim().to_string();
        git(
            &host,
            &["update-ref", "refs/reeve/card-x/feat-new", &head_sha],
        );

        let ops = RealGitOps;
        let outcome = ops
            .fast_forward(&host, "refs/heads/feat-new", "refs/reeve/card-x/feat-new")
            .await
            .unwrap();
        assert_eq!(outcome, FfOutcome::Updated);

        let result = std::process::Command::new("git")
            .args(["rev-parse", "refs/heads/feat-new"])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(result.status.success(), "new branch ref was created");
    }

    #[tokio::test]
    async fn real_push_to_bare_remote_succeeds() {
        let dir = tempdir().unwrap();
        let host = host_repo(dir.path(), "host");

        // Create a bare remote and wire it as `origin`.
        let bare = dir.path().join("origin.git");
        let status = std::process::Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(&bare)
            .status()
            .unwrap();
        assert!(status.success(), "git init --bare failed");

        let bare_url = format!("file://{}", bare.display());
        git(&host, &["remote", "add", "origin", &bare_url]);

        let ops = RealGitOps;
        ops.push(&host, "main").await.unwrap();

        // Verify the bare repo received the branch.
        let result = std::process::Command::new("git")
            .args(["rev-parse", "main"])
            .current_dir(&bare)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "bare repo has main: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
