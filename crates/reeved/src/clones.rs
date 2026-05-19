//! Re-exports the per-card repo clone workflow from reeve-workspace.
//! The GitOps trait, RealGitOps adapter, and clone_all_for_card workflow
//! now live in reeve_workspace::{git, clones}.

pub use reeve_workspace::GitOpsHandle;
// GitOps re-exported for use in handlers.rs tests (crate::clones::GitOps).
#[allow(unused_imports)]
pub use reeve_workspace::GitOps;
