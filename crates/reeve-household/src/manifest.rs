//! Workspace manifest loader. A `household.json` at the meta-repo root declares the
//! workspace pointer, an optional knowledge-base pointer, and a unified list of
//! every repo (workspace meta-repo included). The schema lives here; consumers
//! (clones.rs, handlers.rs) read parsed `Manifest` values, not raw JSON.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

const MANIFEST_FILENAME: &str = "household.json";

/// Reserved directory names that cannot be used as a repo entry's `name`.
/// These collide with meta-repo internals or git itself.
const RESERVED_NAMES: &[&str] = &[".git", ".devcontainer"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Manifest {
    /// Name of the repos[] entry that IS the meta-repo.
    /// Renamed from `workspace` in 2026-05; the old field name is no
    /// longer accepted.
    pub meta_repo: String,

    #[serde(default)]
    pub knowledge_base: Option<String>,

    pub repos: Vec<RepoEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepoEntry {
    /// Manifest identifier; also the on-disk directory name for non-workspace entries.
    pub name: String,

    /// Canonical git URL. Optional; surfaced in `household show` and consumed by
    /// user-owned bootstrap scripts. NOT used by Reeve at card-spawn time.
    #[serde(default)]
    pub url: Option<String>,

    /// Free-form categorisation (`backend`, `core`, `docs`, ...). Neither the KB
    /// marker nor the workspace marker is a tag — both live in top-level fields.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Human-readable; surfaced in `household show`.
    #[serde(default)]
    pub description: Option<String>,
}

impl Manifest {
    /// The entry pointed to by `meta_repo`. Validated at load, so unwrap is safe here.
    pub fn workspace_entry(&self) -> &RepoEntry {
        self.repos
            .iter()
            .find(|r| r.name == self.meta_repo)
            .expect("workspace_entry: validation ensures meta_repo matches a repos[].name")
    }

    /// Every non-workspace entry, in declaration order.
    pub fn siblings(&self) -> impl Iterator<Item = &RepoEntry> {
        self.repos.iter().filter(|r| r.name != self.meta_repo)
    }
}

/// Read and validate `<meta_repo_path>/household.json`.
pub fn load_manifest(meta_repo_path: &Path) -> Result<Manifest> {
    let path = meta_repo_path.join(MANIFEST_FILENAME);
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read manifest at {}", path.display()))?;

    let manifest: Manifest = serde_json::from_str(&body)
        .with_context(|| format!("parse manifest at {}", path.display()))?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(m: &Manifest) -> Result<()> {
    if m.repos.is_empty() {
        bail!("manifest: repos[] must be non-empty (must contain at least the workspace entry)");
    }
    if m.meta_repo.is_empty() {
        bail!("manifest: meta_repo must be non-empty");
    }
    // Names non-empty + unique + not reserved.
    let mut seen = std::collections::HashSet::new();
    for entry in &m.repos {
        if entry.name.is_empty() {
            bail!("manifest: repos[] entry has empty name");
        }
        if RESERVED_NAMES.contains(&entry.name.as_str()) {
            bail!("manifest: repos[].name '{}' is reserved", entry.name);
        }
        if !seen.insert(&entry.name) {
            bail!("manifest: duplicate repos[].name '{}'", entry.name);
        }
    }
    // meta_repo points at exactly one repos[].name.
    if !m.repos.iter().any(|r| r.name == m.meta_repo) {
        bail!(
            "manifest: meta_repo = '{}' does not match any repos[].name",
            m.meta_repo
        );
    }
    // knowledge_base, if set, points at exactly one repos[].name.
    if let Some(kb) = &m.knowledge_base {
        if !m.repos.iter().any(|r| &r.name == kb) {
            bail!(
                "manifest: knowledge_base = '{}' does not match any repos[].name",
                kb
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn write_manifest(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join(MANIFEST_FILENAME);
        std::fs::write(&path, body).unwrap();
        dir.to_path_buf()
    }

    #[test]
    fn parses_full_manifest() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{
              "meta_repo": "myarchy-workspace",
              "knowledge_base": "lore",
              "repos": [
                { "name": "myarchy-workspace", "url": "git@github.com:Mindful-Stack/myarchy-workspace.git" },
                { "name": "backend",  "url": "git@github.com:Mindful-Stack/myarchy-backend.git", "tags": ["backend"] },
                { "name": "frontend", "url": "git@github.com:Mindful-Stack/myarchy-frontend.git", "tags": ["frontend"] },
                { "name": "lore",     "tags": ["docs"] }
              ]
            }"#,
        );
        let m = load_manifest(&root).unwrap();
        assert_eq!(m.meta_repo, "myarchy-workspace");
        assert_eq!(m.knowledge_base.as_deref(), Some("lore"));
        assert_eq!(m.repos.len(), 4);
        assert_eq!(m.workspace_entry().name, "myarchy-workspace");
        assert_eq!(
            m.workspace_entry().url.as_deref(),
            Some("git@github.com:Mindful-Stack/myarchy-workspace.git")
        );
        let sibs: Vec<&str> = m.siblings().map(|r| r.name.as_str()).collect();
        assert_eq!(sibs, vec!["backend", "frontend", "lore"]);
    }

    #[test]
    fn parses_single_repo_inline_kb_manifest() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{
              "meta_repo": "my-ws",
              "knowledge_base": "lore",
              "repos": [
                { "name": "my-ws", "url": "git@github.com:you/my-ws.git" },
                { "name": "lore",  "description": "Inline KB" }
              ]
            }"#,
        );
        let m = load_manifest(&root).unwrap();
        assert_eq!(m.meta_repo, "my-ws");
        assert_eq!(m.workspace_entry().name, "my-ws");
        let sibs: Vec<&str> = m.siblings().map(|r| r.name.as_str()).collect();
        assert_eq!(sibs, vec!["lore"]);
    }

    #[test]
    fn requires_workspace_to_match_a_repos_entry() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "meta_repo": "ghost", "repos": [{ "name": "real" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("meta_repo = 'ghost' does not match"));
    }

    #[test]
    fn requires_knowledge_base_to_match_a_repos_entry() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "meta_repo": "ws", "knowledge_base": "ghost", "repos": [{ "name": "ws" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("knowledge_base = 'ghost' does not match"));
    }

    #[test]
    fn rejects_duplicate_repo_names() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "meta_repo": "a", "repos": [{ "name": "a" }, { "name": "a" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("duplicate repos[].name 'a'"));
    }

    #[test]
    fn rejects_reserved_repo_names() {
        let dir = tempdir().unwrap();
        let root = write_manifest(
            dir.path(),
            r#"{ "meta_repo": "ws", "repos": [{ "name": "ws" }, { "name": ".git" }] }"#,
        );
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("is reserved"));
    }

    #[test]
    fn rejects_empty_repos_array() {
        let dir = tempdir().unwrap();
        let root = write_manifest(dir.path(), r#"{ "meta_repo": "ws", "repos": [] }"#);
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("repos[] must be non-empty"));
    }

    #[test]
    fn rejects_missing_workspace_field() {
        let dir = tempdir().unwrap();
        let root = write_manifest(dir.path(), r#"{ "repos": [{ "name": "a" }] }"#);
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("meta_repo"));
    }

    #[test]
    fn missing_manifest_file_errors() {
        let dir = tempdir().unwrap();
        let err = load_manifest(dir.path()).unwrap_err();
        assert!(format!("{err:#}").contains("read manifest at"));
    }

    #[test]
    fn malformed_json_errors() {
        let dir = tempdir().unwrap();
        let root = write_manifest(dir.path(), r#"{ this is not json"#);
        let err = load_manifest(&root).unwrap_err();
        assert!(format!("{err:#}").contains("parse manifest at"));
    }

}
