//! Per-card repo clones. Implements the `--reference --dissociate` topology
//! from the parent design: `git clone --reference <host> --dissociate <host>
//! <card-workspace>/<repo-name>/`. The clone borrows objects from the host at
//! clone time (cheap) then severs the dependency so the card survives
//! host-side `git gc --prune=now` or rebase drops.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;
use tracing::warn;

use reeve_household::Household;
use reeve_household::manifest::Manifest;

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
    /// Run `git clone <url> <dest>` — no --reference, no --dissociate.
    /// For adding a remote household whose meta-repo lives off-host.
    async fn clone_url(&self, url: &str, dest: &Path) -> Result<()>;
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

    async fn clone_url(&self, url: &str, dest: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["clone", url, dest.to_str().context("dest path not utf-8")?])
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

/// Clone the workspace meta-repo plus every separate-sibling repo declared in
/// the manifest into the card workspace, using `--reference --dissociate` per
/// the parent design's poly-repo topology. Inline entries (directories tracked
/// inside the meta-repo without their own `.git/`) require no separate clone —
/// they arrive with the workspace clone.
///
/// The card workspace dir is created by `git clone` itself in step 1 (git
/// refuses to clone into a non-empty directory). The caller must NOT
/// `create_dir_all(workspace)` before calling.
///
/// On any failure, the partial workspace is removed (best-effort) and the
/// error is returned.
pub async fn clone_all_for_card(
    ops: &dyn GitOps,
    household: &Household,
    manifest: &Manifest,
    workspace: &Path,
) -> Result<()> {
    let meta = &household.meta_repo_path;

    // Pre-flight: validate every NON-workspace entry exists on disk in some
    // form. Missing entirely => error. Existing without .git/ => inline.
    // Existing with .git/ => sibling. Workspace entry doesn't need this check.
    for entry in manifest.siblings() {
        let dir = meta.join(&entry.name);
        if !dir.exists() {
            anyhow::bail!(
                "sibling '{}' declared in household.json but not cloned at {} — \
                 run your workspace's bootstrap (e.g. `make setup`) first",
                entry.name,
                dir.display(),
            );
        }
    }

    // Step 1: clone the workspace entry. Its on-disk source is the meta-repo
    // root itself. `git clone` creates `<workspace>/`.
    if let Err(e) = ops.clone(meta, workspace).await {
        // Workspace dir may have been partially created by git; clean it up.
        cleanup(workspace);
        return Err(e.context("clone workspace meta-repo"));
    }

    // Step 2: for each non-workspace entry, dispatch on disk state.
    for entry in manifest.siblings() {
        let source = meta.join(&entry.name);
        let dest = workspace.join(&entry.name);
        if source.join(".git").exists() {
            // Separate sibling repo.
            if let Err(e) = ops.clone(&source, &dest).await {
                cleanup(workspace);
                return Err(e.context(format!("clone sibling '{}'", entry.name)));
            }
        } else {
            // Inline directory. No clone needed — it travelled with the
            // workspace clone. Verify it actually arrived (defensive).
            if !dest.exists() {
                cleanup(workspace);
                anyhow::bail!(
                    "inline entry '{}' is missing from workspace clone — \
                     is it correctly tracked in the meta-repo?",
                    entry.name,
                );
            }
        }
    }
    Ok(())
}

fn cleanup(workspace: &Path) {
    if workspace.exists() {
        if let Err(e) = std::fs::remove_dir_all(workspace) {
            warn!(
                "failed to clean partial workspace {}: {e:#}",
                workspace.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_household::Household;
    use reeve_household::manifest::{Manifest, RepoEntry};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Records every `clone` call.
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
        fetches: Mutex<Vec<(PathBuf, PathBuf, String)>>,
        list_namespace_calls: Mutex<Vec<(PathBuf, String)>>,
        list_namespace_response: Mutex<Vec<String>>,
        fast_forward_calls: Mutex<Vec<(PathBuf, String, String)>>,
        fast_forward_response: Mutex<FfOutcome>,
        push_calls: Mutex<Vec<(PathBuf, String)>>,
        push_response: Mutex<std::result::Result<(), String>>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fetches: Mutex::new(vec![]),
                list_namespace_calls: Mutex::new(vec![]),
                list_namespace_response: Mutex::new(vec![]),
                fast_forward_calls: Mutex::new(vec![]),
                fast_forward_response: Mutex::new(FfOutcome::Updated),
                push_calls: Mutex::new(vec![]),
                push_response: Mutex::new(Ok(())),
            }
        }

        fn set_list_namespace_refs_response(&self, branches: Vec<String>) {
            *self.list_namespace_response.lock().unwrap() = branches;
        }

        fn set_fast_forward_response(&self, outcome: FfOutcome) {
            *self.fast_forward_response.lock().unwrap() = outcome;
        }

        fn set_push_failure(&self, message: &str) {
            *self.push_response.lock().unwrap() = Err(message.to_string());
        }
    }

    #[async_trait]
    impl GitOps for MockGitOps {
        async fn clone(&self, source: &Path, dest: &Path) -> Result<()> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((source.to_path_buf(), dest.to_path_buf()));
            // Pretend the clone succeeded by creating the dest dir.
            std::fs::create_dir_all(dest).unwrap();
            Ok(())
        }

        async fn clone_url(&self, url: &str, dest: &Path) -> Result<()> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((PathBuf::from(url), dest.to_path_buf()));
            std::fs::create_dir_all(dest).unwrap();
            Ok(())
        }

        async fn fetch(&self, workdir: &Path, source: &Path, namespace: &str) -> Result<()> {
            self.fetches.lock().unwrap().push((
                workdir.to_path_buf(),
                source.to_path_buf(),
                namespace.to_string(),
            ));
            Ok(())
        }

        async fn list_namespace_refs(
            &self,
            workdir: &Path,
            namespace: &str,
        ) -> Result<Vec<String>> {
            self.list_namespace_calls
                .lock()
                .unwrap()
                .push((workdir.to_path_buf(), namespace.to_string()));
            Ok(self.list_namespace_response.lock().unwrap().clone())
        }

        async fn fast_forward(
            &self,
            workdir: &Path,
            target_ref: &str,
            source_ref: &str,
        ) -> Result<FfOutcome> {
            self.fast_forward_calls.lock().unwrap().push((
                workdir.to_path_buf(),
                target_ref.to_string(),
                source_ref.to_string(),
            ));
            Ok(*self.fast_forward_response.lock().unwrap())
        }

        async fn push(&self, workdir: &Path, branch: &str) -> Result<()> {
            self.push_calls
                .lock()
                .unwrap()
                .push((workdir.to_path_buf(), branch.to_string()));
            match self.push_response.lock().unwrap().clone() {
                Ok(()) => Ok(()),
                Err(msg) => anyhow::bail!("{msg}"),
            }
        }
    }

    /// Build a host meta-repo with workspace `.git/` + each named sibling.
    /// `siblings_with_git` get their own `.git/`; `inline_dirs` get a directory
    /// but no `.git/`.
    fn host_meta(dir: &Path, siblings_with_git: &[&str], inline_dirs: &[&str]) -> PathBuf {
        let meta = dir.join("meta");
        std::fs::create_dir_all(meta.join(".git")).unwrap();
        for name in siblings_with_git {
            std::fs::create_dir_all(meta.join(name).join(".git")).unwrap();
        }
        for name in inline_dirs {
            std::fs::create_dir_all(meta.join(name)).unwrap();
        }
        meta
    }

    fn workspace_manifest(workspace_name: &str, repos: &[&str], kb: Option<&str>) -> Manifest {
        Manifest {
            workspace: workspace_name.into(),
            knowledge_base: kb.map(str::to_string),
            repos: repos
                .iter()
                .map(|n| RepoEntry {
                    name: (*n).into(),
                    url: None,
                    tags: vec![],
                    description: None,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn clone_all_for_card_clones_workspace_then_separate_siblings() {
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &["backend", "lore"], &[]);
        let workspace = dir.path().join("ws"); // do NOT pre-create
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "backend", "lore"], Some("lore"));
        let ops = MockGitOps::new();
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "workspace + 2 separate siblings");
        assert_eq!(calls[0].0, meta, "workspace cloned first from meta path");
        assert_eq!(calls[0].1, workspace);
        assert_eq!(calls[1].0, meta.join("backend"));
        assert_eq!(calls[1].1, workspace.join("backend"));
        assert_eq!(calls[2].0, meta.join("lore"));
        assert_eq!(calls[2].1, workspace.join("lore"));
    }

    #[tokio::test]
    async fn clone_all_for_card_skips_inline_entries() {
        // Inline `lore/` exists in the meta-repo but has no .git/.
        // The mock's clone of the workspace creates the dest dir;
        // we have to seed it with the inline `lore/` to mimic the
        // real "it travels with the meta-repo clone" behaviour.
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &["lore"]);
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "lore"], Some("lore"));

        // Custom mock that creates the workspace AND copies inline dirs.
        struct InlineSimulatingOps {
            inner: MockGitOps,
            inline_to_seed: Vec<(PathBuf, PathBuf)>, // (src, dest-under-workspace)
        }
        #[async_trait]
        impl GitOps for InlineSimulatingOps {
            async fn clone(&self, source: &Path, dest: &Path) -> anyhow::Result<()> {
                self.inner.clone(source, dest).await?;
                // After the workspace clone, the real `git clone` would have
                // brought every tracked subdirectory along. Simulate that for
                // the workspace-level clone only.
                if dest
                    == self
                        .inline_to_seed
                        .first()
                        .map(|(_, d)| d.parent().unwrap())
                        .unwrap_or(dest)
                {
                    for (src_rel, _) in &self.inline_to_seed {
                        let abs_src = source.join(src_rel.file_name().unwrap());
                        let abs_dest = dest.join(src_rel.file_name().unwrap());
                        if abs_src.exists() {
                            std::fs::create_dir_all(&abs_dest)?;
                        }
                    }
                }
                Ok(())
            }

            async fn clone_url(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
                self.inner.clone_url(url, dest).await
            }

            async fn fetch(
                &self,
                workdir: &Path,
                source: &Path,
                namespace: &str,
            ) -> anyhow::Result<()> {
                self.inner.fetch(workdir, source, namespace).await
            }

            async fn list_namespace_refs(
                &self,
                workdir: &Path,
                namespace: &str,
            ) -> anyhow::Result<Vec<String>> {
                self.inner.list_namespace_refs(workdir, namespace).await
            }

            async fn fast_forward(
                &self,
                workdir: &Path,
                target_ref: &str,
                source_ref: &str,
            ) -> anyhow::Result<FfOutcome> {
                self.inner
                    .fast_forward(workdir, target_ref, source_ref)
                    .await
            }

            async fn push(&self, workdir: &Path, branch: &str) -> anyhow::Result<()> {
                self.inner.push(workdir, branch).await
            }
        }
        let ops = InlineSimulatingOps {
            inner: MockGitOps::new(),
            inline_to_seed: vec![(meta.join("lore"), workspace.join("lore"))],
        };
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.inner.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "only the workspace was cloned; lore is inline"
        );
        assert_eq!(calls[0].0, meta);
        assert!(
            workspace.join("lore").exists(),
            "inline lore/ landed via workspace clone"
        );
    }

    #[tokio::test]
    async fn clone_all_for_card_errors_on_missing_sibling() {
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &[]); // declare "backend" but don't create it
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta,
        };
        let manifest = workspace_manifest("test", &["test", "backend"], None);
        let ops = MockGitOps::new();
        let err = clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("declared in household.json but not cloned"),
            "unexpected error: {msg}",
        );
        assert_eq!(ops.calls.lock().unwrap().len(), 0, "no clones issued");
        assert!(
            !workspace.exists(),
            "workspace not created on pre-flight failure"
        );
    }

    #[tokio::test]
    async fn clone_all_for_card_with_workspace_only_clones_workspace() {
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &[]);
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test"], None);
        let ops = MockGitOps::new();
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, meta);
        assert_eq!(calls[0].1, workspace);
    }

    #[tokio::test]
    async fn mock_fetch_records_call() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let source = dir.path().join("card-clone");
        let ops = MockGitOps::new();
        ops.fetch(&workdir, &source, "deadbeef").await.unwrap();
        let fetches = ops.fetches.lock().unwrap();
        assert_eq!(fetches.len(), 1);
        assert_eq!(
            fetches[0],
            (workdir.clone(), source.clone(), "deadbeef".to_string())
        );
    }

    #[tokio::test]
    async fn mock_list_namespace_refs_returns_recorded() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let ops = MockGitOps::new();
        ops.set_list_namespace_refs_response(vec!["feat-a".into(), "feat-b".into()]);
        let result = ops.list_namespace_refs(&workdir, "card-x").await.unwrap();
        assert_eq!(result, vec!["feat-a".to_string(), "feat-b".to_string()]);
        let calls = ops.list_namespace_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (workdir.clone(), "card-x".to_string()));
    }

    #[tokio::test]
    async fn mock_fast_forward_returns_configured_outcome() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let ops = MockGitOps::new();
        ops.set_fast_forward_response(FfOutcome::Updated);
        let result = ops
            .fast_forward(&workdir, "refs/heads/main", "refs/reeve/card-x/main")
            .await
            .unwrap();
        assert_eq!(result, FfOutcome::Updated);
        let calls = ops.fast_forward_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            (
                workdir.clone(),
                "refs/heads/main".to_string(),
                "refs/reeve/card-x/main".to_string()
            )
        );
    }

    #[tokio::test]
    async fn mock_push_records_call() {
        let dir = tempdir().unwrap();
        let workdir = dir.path().join("host");
        let ops = MockGitOps::new();
        ops.push(&workdir, "feat-x").await.unwrap();
        let calls = ops.push_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (workdir.clone(), "feat-x".to_string()));
    }
}

#[cfg(test)]
mod integration_tests {
    //! Real-git tests. Require `git` in $PATH. Hit the filesystem.
    use super::*;
    use reeve_household::Household;
    use reeve_household::manifest::{Manifest, RepoEntry};
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

    /// Init a meta-repo with one sibling, both real git repos with one commit each.
    fn full_workspace(dir: &Path) -> (PathBuf, Household, Manifest) {
        let meta = host_repo(dir, "meta");
        // Carve out a sibling inside the meta-repo dir.
        let _sibling = host_repo(&meta, "backend");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = Manifest {
            workspace: "meta".into(),
            knowledge_base: None,
            repos: vec![
                RepoEntry {
                    name: "meta".into(),
                    url: None,
                    tags: vec![],
                    description: None,
                },
                RepoEntry {
                    name: "backend".into(),
                    url: None,
                    tags: vec![],
                    description: None,
                },
            ],
        };
        (meta, household, manifest)
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
    async fn real_clone_all_for_card_lands_workspace_and_siblings() {
        let dir = tempdir().unwrap();
        let (_meta, household, manifest) = full_workspace(dir.path());
        let workspace = dir.path().join("workspace"); // do NOT pre-create

        let ops = RealGitOps;
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();

        assert!(
            workspace.join(".git").exists(),
            "workspace meta-repo cloned"
        );
        assert!(workspace.join("backend/.git").exists(), "sibling cloned");
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

        // Create divergent histories: heads/main has one commit, refs/reeve/card-x/main
        // points at a DIFFERENT commit on a parallel branch with no shared base beyond
        // the initial commit.
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

        // Add a namespaced ref for a branch that doesn't yet exist under refs/heads.
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
