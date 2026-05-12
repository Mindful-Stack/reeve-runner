//! Household profile loader. M2 reads a minimal schema (image + cmd).
//! Unknown fields are tolerated so M3+ can extend without breaking M2 profiles.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HouseholdProfile {
    pub image: String,
    pub cmd: Vec<String>,
}

pub type HouseholdMap = Arc<HashMap<String, HouseholdProfile>>;

/// Load every `<name>.toml` from `dir` into a map keyed by `<name>`.
/// A missing dir returns an empty map; malformed TOML is an error.
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
        let profile: HouseholdProfile =
            toml::from_str(&body).with_context(|| format!("parse {}", path.display()))?;
        out.insert(name, profile);
    }
    Ok(Arc::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_minimal_profile() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "image = \"alpine:latest\"\ncmd = [\"sleep\", \"infinity\"]\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        let p = map.get("test").unwrap();
        assert_eq!(p.image, "alpine:latest");
        assert_eq!(p.cmd, vec!["sleep".to_string(), "infinity".to_string()]);
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
        std::fs::write(
            dir.path().join("test.toml"),
            "image = \"a\"\ncmd = [\"b\"]\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("test"));
    }

    #[test]
    fn tolerates_unknown_fields() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("test.toml"),
            "image = \"a\"\ncmd = [\"b\"]\nfuture_field = \"ignored\"\n",
        )
        .unwrap();
        let map = load_all(dir.path()).unwrap();
        assert!(map.contains_key("test"));
    }

    #[test]
    fn malformed_toml_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("test.toml"), "image = (not valid").unwrap();
        assert!(load_all(dir.path()).is_err());
    }
}
