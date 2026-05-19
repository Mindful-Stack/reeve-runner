//! Workspace aggregate: per-card materialization (clones + container + listener).

pub mod container;
pub mod fetch;
pub mod git;

pub use container::{ContainerOps, ContainerOpsHandle, make_container_ops};
pub use git::{FfOutcome, GitOps, GitOpsHandle, make_git_ops};
