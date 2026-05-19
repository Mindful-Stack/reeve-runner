//! Per-card repo clones. Implements the `--reference --dissociate` topology
//! from the parent design: `git clone --reference <host> --dissociate <host>
//! <card-workspace>/<repo-name>/`. The clone borrows objects from the host at
//! clone time (cheap) then severs the dependency so the card survives
//! host-side `git gc --prune=now` or rebase drops.

use crate::git::GitOps;
use anyhow::Result;
use std::path::Path;
use tracing::warn;

use reeve_household::Household;
use reeve_household::manifest::Manifest;

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
    use async_trait::async_trait;
    use crate::git::FfOutcome;
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
    use reeve_household::manifest::{Manifest, RepoEntry};
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invoke");
        assert!(status.success(), "git {args:?} failed in {}", repo.display());
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
            workspace: "meta".into(),
            knowledge_base: None,
            repos: vec![
                RepoEntry { name: "meta".into(), url: None, tags: vec![], description: None },
                RepoEntry { name: "backend".into(), url: None, tags: vec![], description: None },
            ],
        };
        let workspace = dir.path().join("workspace");

        let ops = make_git_ops();
        clone_all_for_card(&*ops, &household, &manifest, &workspace)
            .await
            .unwrap();

        assert!(workspace.join(".git").exists(), "workspace meta-repo cloned");
        assert!(workspace.join("backend/.git").exists(), "sibling cloned");
    }
}
