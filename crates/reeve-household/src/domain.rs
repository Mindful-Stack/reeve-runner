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

/// Yields (repo_name, host_path) for every repo participating in a card.
/// Used by both the fetch worker and the push handler.
///
/// Iteration order is part of the contract (pinned for testability):
/// 1. The workspace entry — `manifest.workspace` — first, with host_path
///    = `household.meta_repo_path` itself.
/// 2. Then every entry in `manifest.repos` in declaration order, skipping
///    the one whose `name == manifest.workspace` (already yielded as #1).
///    For each, host_path = `household.meta_repo_path.join(entry.name)`.
pub fn iter_card_repos<'a>(
    household: &'a Household,
    manifest: &'a Manifest,
) -> impl Iterator<Item = (&'a str, PathBuf)> + 'a {
    let workspace_name = manifest.workspace.as_str();
    let meta = household.meta_repo_path.clone();
    let workspace_entry = std::iter::once((workspace_name, meta.clone()));
    let siblings = manifest
        .repos
        .iter()
        .filter(move |r| r.name != workspace_name)
        .map(move |r| (r.name.as_str(), meta.join(&r.name)));
    workspace_entry.chain(siblings)
}
