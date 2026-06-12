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
/// 1. The workspace entry — `manifest.meta_repo` — first, with host_path
///    = `household.meta_repo_path` itself.
/// 2. Then every entry in `manifest.repos` in declaration order, skipping
///    the one whose `name == manifest.meta_repo` (already yielded as #1).
///    For each, host_path = `household.meta_repo_path.join(entry.name)`.
pub fn iter_card_repos<'a>(
    household: &'a Household,
    manifest: &'a Manifest,
) -> impl Iterator<Item = (&'a str, PathBuf)> + 'a {
    let workspace_name = manifest.meta_repo.as_str();
    let meta = household.meta_repo_path.clone();
    let workspace_entry = std::iter::once((workspace_name, meta.clone()));
    let siblings = manifest
        .repos
        .iter()
        .filter(move |r| r.name != workspace_name)
        .map(move |r| (r.name.as_str(), meta.join(&r.name)));
    workspace_entry.chain(siblings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, RepoEntry};
    use std::path::PathBuf;

    fn manifest(workspace: &str, repos: &[&str]) -> Manifest {
        Manifest {
            meta_repo: workspace.to_string(),
            knowledge_base: None,
            shared_knowledge_bases: vec![],
            spec_path: None,
            plan_path: None,
            repos: repos
                .iter()
                .map(|name| RepoEntry {
                    name: (*name).to_string(),
                    url: None,
                    tags: vec![],
                    description: None,
                })
                .collect(),
        }
    }

    #[test]
    fn iter_card_repos_yields_workspace_first_then_siblings_in_order() {
        let h = Household {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/meta"),
        };
        let m = manifest("ws", &["ws", "backend", "lore"]);
        let yielded: Vec<(String, PathBuf)> = iter_card_repos(&h, &m)
            .map(|(n, p)| (n.to_string(), p))
            .collect();
        assert_eq!(
            yielded,
            vec![
                ("ws".to_string(), PathBuf::from("/tmp/meta")),
                ("backend".to_string(), PathBuf::from("/tmp/meta/backend")),
                ("lore".to_string(), PathBuf::from("/tmp/meta/lore")),
            ]
        );
    }

    #[test]
    fn iter_card_repos_skips_workspace_entry_in_siblings_section() {
        // Manifest where the workspace entry isn't first in repos[]; ensure
        // the helper still emits it first and doesn't double-emit.
        let h = Household {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/meta"),
        };
        let m = manifest("ws", &["backend", "ws", "lore"]);
        let yielded: Vec<String> = iter_card_repos(&h, &m)
            .map(|(n, _)| n.to_string())
            .collect();
        assert_eq!(yielded, vec!["ws", "backend", "lore"]);
    }

    #[test]
    fn iter_card_repos_with_only_workspace_yields_one_entry() {
        let h = Household {
            name: "test".into(),
            meta_repo_path: PathBuf::from("/tmp/meta"),
        };
        let m = manifest("ws", &["ws"]);
        let yielded: Vec<String> = iter_card_repos(&h, &m)
            .map(|(n, _)| n.to_string())
            .collect();
        assert_eq!(yielded, vec!["ws"]);
    }
}
