//! Household profile loader. M3 schema: devcontainer.json path + repo list.
//! Tilde expansion happens at load time. Unknown fields are tolerated so M4+
//! can extend without breaking M3 profiles.

use crate::clones::RepoSpec;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HouseholdProfile {
    /// Path to the household's devcontainer.json. Required.
    /// Tilde-expanded at load time.
    pub devcontainer: PathBuf,

    /// Repos to clone into each card's workspace. Optional; zero is allowed.
    /// Each repo's `path` is tilde-expanded at load time.
    #[serde(default)]
    pub repos: Vec<RepoSpec>,
}

fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}

pub type HouseholdMap = Arc<HashMap<String, HouseholdProfile>>;

/// Load every `<name>.toml` from `dir` into a map keyed by `<name>`.
/// A missing dir returns an empty map; malformed TOML is an error.
/// Tilde expansion is applied to `devcontainer` and every `repos[].path`.
pub fn load_all(dir: &Path) -> Result<HouseholdMap> {
    let mut out = HashMap::new();
    if !dir.exists() {
        return Ok(Arc::new(out));
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .with_context(|| format!("bad filename {}", path.display()))?
            .to_string();
        let body =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let mut profile: HouseholdProfile =
            toml::from_str(&body).with_context(|| format!("parse {}", path.display()))?;
        profile.devcontainer = expand_tilde(&profile.devcontainer);
        for repo in profile.repos.iter_mut() {
            repo.path = expand_tilde(&repo.path);
        }
        out.insert(name, profile);
    }
    Ok(Arc::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_profile_with_devcontainer_and_repos() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            r#"
devcontainer = "/opt/dc/devcontainer.json"

[[repos]]
name = "backend"
path = "/srv/backend"

[[repos]]
name = "frontend"
path = "/srv/frontend"
"#,
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        assert_eq!(p.devcontainer, PathBuf::from("/opt/dc/devcontainer.json"));
        assert_eq!(p.repos.len(), 2);
        assert_eq!(p.repos[0].name, "backend");
        assert_eq!(p.repos[0].path, PathBuf::from("/srv/backend"));
        assert_eq!(p.repos[1].name, "frontend");
    }

    #[test]
    fn missing_dir_yields_empty_map() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        let map = load_all(&missing).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn ignores_non_toml_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "hello").unwrap();
        std::fs::write(dir.path().join("test.toml"), "devcontainer = \"/x\"\n").unwrap();
        let map = load_all(dir.path()).unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("test"));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "devcontainer = \"/x\"\nfuture_field = \"ignored\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert!(map.contains_key("test"));
    }

    #[test]
    fn malformed_toml_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("test.toml"), "devcontainer = (not valid").unwrap();
        assert!(load_all(dir.path()).is_err());
    }

    #[test]
    fn missing_devcontainer_field_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "[[repos]]\nname = \"x\"\npath = \"/y\"\n",
        )
        .unwrap();
        let result = load_all(dir.path());
        assert!(result.is_err(), "missing devcontainer field should error");
    }

    #[test]
    fn empty_repos_is_valid() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "devcontainer = \"/x/devcontainer.json\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        assert!(p.repos.is_empty());
    }

    #[test]
    fn tilde_expands_in_devcontainer_and_repo_paths() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            r#"
devcontainer = "~/dc.json"

[[repos]]
name = "r"
path = "~/r"
"#,
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(p.devcontainer, PathBuf::from(&home).join("dc.json"));
        assert_eq!(p.repos[0].path, PathBuf::from(&home).join("r"));
    }
}
