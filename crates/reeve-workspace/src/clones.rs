//! Per-card repo clones. Implements the `--reference --dissociate` topology
//! from the parent design: `git clone --reference <host> --dissociate <host>
//! <card-workspace>/<repo-name>/`. The clone borrows objects from the host at
//! clone time (cheap) then severs the dependency so the card survives
//! host-side `git gc --prune=now` or rebase drops.

use crate::git::GitOps;
use anyhow::{Context, Result};
use std::path::Path;

use reeve_household::Household;
use reeve_household::Manifest;

/// Idempotent clone of the workspace meta-repo + each sibling declared
/// in the manifest. Per-entry behaviour:
///
/// - Destination missing → `git clone` it.
/// - Destination exists with `.git/` → assumed already-cloned, skip.
/// - Destination exists without `.git/` → return error (corrupted state;
///   user must investigate / remove manually).
///
/// On failure of any clone, the partial workspace is left in place. The
/// caller may retry; previously-cloned entries will be skipped, only the
/// failed/missing ones re-attempted.
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

    // Step 1: meta-repo → workspace.
    ensure_cloned(ops, meta, workspace, "workspace meta-repo").await?;

    // Step 2: each sibling. Inline entries (subdirs of meta-repo with no
    // own .git/) are not cloned — they arrived in step 1.
    for entry in manifest.siblings() {
        let source = meta.join(&entry.name);
        let dest = workspace.join(&entry.name);
        if source.join(".git").exists() {
            // Separate sibling repo.
            ensure_cloned(ops, &source, &dest, &format!("sibling '{}'", entry.name)).await?;
        } else {
            // Inline directory. No clone needed — it travelled with the
            // workspace clone. Verify it actually arrived (defensive).
            if !dest.exists() {
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

/// Clone `source` into `dest` unless `dest` is already a git checkout.
///
/// - `dest` doesn't exist → `git clone source dest`.
/// - `dest` exists with `.git/` → no-op (already cloned).
/// - `dest` exists without `.git/` → error: dest is in an unknown state.
async fn ensure_cloned(
    ops: &dyn GitOps,
    source: &Path,
    dest: &Path,
    label: &str,
) -> Result<()> {
    if dest.exists() {
        if dest.join(".git").exists() {
            tracing::debug!("skipping {label}: already cloned at {}", dest.display());
            return Ok(());
        }
        anyhow::bail!(
            "{label}: {} exists but is not a git checkout (no .git/); \
             remove it manually if you want to re-clone",
            dest.display(),
        );
    }
    ops.clone(source, dest).await.context(format!("clone {label}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FfOutcome;
    use async_trait::async_trait;
    use reeve_household::Household;
    use reeve_household::{Manifest, RepoEntry};
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
            meta_repo: workspace_name.into(),
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
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &["lore"]);
        let workspace = dir.path().join("ws");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "lore"], Some("lore"));

        struct InlineSimulatingOps {
            inner: MockGitOps,
            inline_to_seed: Vec<(PathBuf, PathBuf)>,
        }
        #[async_trait]
        impl GitOps for InlineSimulatingOps {
            async fn clone(&self, source: &Path, dest: &Path) -> anyhow::Result<()> {
                self.inner.clone(source, dest).await?;
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
        let meta = host_meta(dir.path(), &[], &[]);
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
    async fn clone_all_for_card_is_idempotent_on_full_workspace() {
        // Pre-create workspace/.git and workspace/backend/.git to simulate a
        // fully-cloned workspace. clone_all_for_card must return Ok with zero
        // clone calls.
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &["backend"], &[]);
        let workspace = dir.path().join("ws");
        // Pre-create both destinations as if they were already cloned.
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        std::fs::create_dir_all(workspace.join("backend").join(".git")).unwrap();
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "backend"], None);
        let ops = MockGitOps::new();
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        assert_eq!(
            ops.calls.lock().unwrap().len(),
            0,
            "no clones issued when everything already exists"
        );
    }

    #[tokio::test]
    async fn clone_all_for_card_clones_only_missing_siblings() {
        // Pre-create workspace/.git (meta already done) but leave
        // workspace/backend/ absent. Only the sibling should be cloned (one
        // call, not two).
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &["backend"], &[]);
        let workspace = dir.path().join("ws");
        // Simulate meta-repo already cloned.
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "backend"], None);
        let ops = MockGitOps::new();
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only the missing sibling is cloned");
        assert_eq!(calls[0].0, meta.join("backend"), "source is the sibling");
        assert_eq!(calls[0].1, workspace.join("backend"), "dest is workspace/backend");
    }

    #[tokio::test]
    async fn clone_all_for_card_errors_on_workspace_without_git() {
        // workspace/ exists but has no .git/ — corrupted state.
        // Must return an error mentioning "not a git checkout"; zero clones.
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &[], &[]);
        let workspace = dir.path().join("ws");
        // Create workspace dir with a file but no .git/.
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("some-file.txt"), "data").unwrap();
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test"], None);
        let ops = MockGitOps::new();
        let err = clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not a git checkout"),
            "expected 'not a git checkout' in error; got: {msg}",
        );
        assert_eq!(
            ops.calls.lock().unwrap().len(),
            0,
            "no clone calls issued"
        );
    }

    #[tokio::test]
    async fn clone_all_for_card_errors_on_sibling_dest_without_git() {
        // workspace/.git exists (meta cloned) but workspace/backend/ is an
        // empty dir with no .git/. Must return an error; no extra clone call.
        let dir = tempdir().unwrap();
        let meta = host_meta(dir.path(), &["backend"], &[]);
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        // Sibling dest exists as a plain dir — no .git/.
        std::fs::create_dir_all(workspace.join("backend")).unwrap();
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = workspace_manifest("test", &["test", "backend"], None);
        let ops = MockGitOps::new();
        let err = clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not a git checkout"),
            "expected 'not a git checkout' in error; got: {msg}",
        );
        assert_eq!(
            ops.calls.lock().unwrap().len(),
            0,
            "no clone calls issued"
        );
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
    //! Real-git integration test for clone_all_for_card.
    use super::*;
    use crate::make_git_ops;
    use reeve_household::Household;
    use reeve_household::{Manifest, RepoEntry};
    use std::path::{Path, PathBuf};
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
    async fn real_clone_all_for_card_lands_workspace_and_siblings() {
        let dir = tempdir().unwrap();
        let meta = host_repo(dir.path(), "meta");
        let _sibling = host_repo(&meta, "backend");
        let household = Household {
            name: "test".into(),
            meta_repo_path: meta.clone(),
        };
        let manifest = Manifest {
            meta_repo: "meta".into(),
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
        let workspace = dir.path().join("workspace");

        let ops = make_git_ops();
        clone_all_for_card(&*ops, &household, &manifest, &workspace)
            .await
            .unwrap();

        assert!(
            workspace.join(".git").exists(),
            "workspace meta-repo cloned"
        );
        assert!(workspace.join("backend/.git").exists(), "sibling cloned");
    }
}
