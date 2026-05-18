//! Household scaffolding port: clone-template → re-init → manifest-rewrite → commit.
//! Used by `handle_household_new`.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;

/// Port for the git operations household scaffolding needs.
#[async_trait]
pub trait HouseholdScaffold: Send + Sync {
    async fn clone_template(&self, template_url: &str, dest: &Path) -> Result<()>;
    async fn init_repo(&self, dest: &Path) -> Result<()>;
    async fn commit_all(&self, dest: &Path, message: &str) -> Result<()>;
}

pub(crate) struct RealHouseholdScaffold;

#[async_trait]
impl HouseholdScaffold for RealHouseholdScaffold {
    async fn clone_template(&self, template_url: &str, dest: &Path) -> Result<()> {
        let output = Command::new("git")
            .args([
                "clone",
                template_url,
                dest.to_str().context("dest path not utf-8")?,
            ])
            .output()
            .await
            .context("exec git clone (template)")?;
        if !output.status.success() {
            anyhow::bail!(
                "git clone {template_url} failed (exit {}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn init_repo(&self, dest: &Path) -> Result<()> {
        std::fs::remove_dir_all(dest.join(".git"))
            .with_context(|| format!("remove {}/.git", dest.display()))?;
        git_in(dest, &["init", "-b", "main", "--quiet"]).await
    }

    async fn commit_all(&self, dest: &Path, message: &str) -> Result<()> {
        git_in(dest, &["add", "-A"]).await?;
        git_in(dest, &["commit", "-m", message, "--quiet"]).await
    }
}

async fn git_in(dir: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "git {args:?} failed in {} (exit {}): {}",
            dir.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

pub fn make_scaffold() -> Arc<dyn HouseholdScaffold> {
    Arc::new(RealHouseholdScaffold)
}
