//! Household domain entity.

use crate::manifest::{self, Manifest};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub name: String,
    pub meta_repo_path: PathBuf,
}

/// Load and validate the meta-repo's manifest.
pub fn load_household_manifest(household: &Household) -> Result<Manifest> {
    manifest::load_manifest(&household.meta_repo_path)
        .with_context(|| format!("load manifest for household '{}'", household.name))
}
