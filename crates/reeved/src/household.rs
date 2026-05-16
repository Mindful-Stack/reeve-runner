//! M3.5 household: a thin sqlite-backed registry mapping `name → meta_repo_path`.
//! The manifest is read from the meta-repo on demand (see `manifest.rs`); no
//! cached state lives in this module. M3's TOML-based `HouseholdProfile` is gone.

use crate::manifest::Manifest;
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub name: String,
    pub meta_repo_path: PathBuf,
}

/// Load and validate the meta-repo's manifest. Convenience wrapper over
/// `manifest::load_manifest`. Used by handlers that want the manifest
/// in their hot path.
pub fn load_household_manifest(household: &Household) -> Result<Manifest> {
    crate::manifest::load_manifest(&household.meta_repo_path)
        .with_context(|| format!("load manifest for household '{}'", household.name))
}
