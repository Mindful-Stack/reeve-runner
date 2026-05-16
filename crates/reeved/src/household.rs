//! M3.5 household: a thin sqlite-backed registry mapping `name → meta_repo_path`.
//! The manifest is read from the meta-repo on demand (see `manifest.rs`); no
//! cached state lives in this module. M3's TOML-based `HouseholdProfile` is gone.

use crate::db;
use crate::manifest::Manifest;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub name: String,
    pub meta_repo_path: PathBuf,
}

pub type HouseholdMap = Arc<HashMap<String, Household>>;

pub fn load_households(conn: &rusqlite::Connection) -> Result<HouseholdMap> {
    let rows = db::list_households(conn).context("list households from db")?;
    let mut map = HashMap::new();
    for row in rows {
        map.insert(
            row.name.clone(),
            Household {
                name: row.name,
                meta_repo_path: row.meta_repo_path,
            },
        );
    }
    Ok(Arc::new(map))
}

/// Load and validate the meta-repo's manifest. Convenience wrapper over
/// `manifest::load_manifest`. Used by handlers that want the manifest
/// in their hot path.
pub fn load_household_manifest(household: &Household) -> Result<Manifest> {
    crate::manifest::load_manifest(&household.meta_repo_path)
        .with_context(|| format!("load manifest for household '{}'", household.name))
}
