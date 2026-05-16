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

use crate::household::Household;
use crate::manifest::Manifest;

/// Abstract git-subprocess backend. The trait exists so unit tests can mock
/// out the subprocess and assert on the argv that would have been issued.
#[async_trait]
pub trait GitOps: Send + Sync {
    /// Run `git clone --reference <source> --dissociate <source> <dest>`.
    async fn clone(&self, source: &Path, dest: &Path) -> Result<()>;
    /// Run `git clone <url> <dest>` — no --reference, no --dissociate.
    /// For adding a remote household whose meta-repo lives off-host.
    async fn clone_url(&self, url: &str, dest: &Path) -> Result<()>;
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
            .args([
                "clone",
                url,
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
                "sibling '{}' declared in repos.json but not cloned at {} — \
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
    use crate::household::Household;
    use crate::manifest::{Manifest, RepoEntry};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Records every `clone` call.
    struct MockGitOps {
        calls: Mutex<Vec<(PathBuf, PathBuf)>>,
    }

    impl MockGitOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
            }
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
        }
        let ops = InlineSimulatingOps {
            inner: MockGitOps::new(),
            inline_to_seed: vec![(meta.join("lore"), workspace.join("lore"))],
        };
        clone_all_for_card(&ops, &household, &manifest, &workspace)
            .await
            .unwrap();
        let calls = ops.inner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "only the workspace was cloned; lore is inline");
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
            msg.contains("declared in repos.json but not cloned"),
            "unexpected error: {msg}",
        );
        assert_eq!(ops.calls.lock().unwrap().len(), 0, "no clones issued");
        assert!(!workspace.exists(), "workspace not created on pre-flight failure");
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
}

#[cfg(test)]
mod integration_tests {
    //! Real-git tests. Require `git` in $PATH. Hit the filesystem.
    use super::*;
    use crate::household::Household;
    use crate::manifest::{Manifest, RepoEntry};
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

        assert!(workspace.join(".git").exists(), "workspace meta-repo cloned");
        assert!(workspace.join("backend/.git").exists(), "sibling cloned");
    }
}
