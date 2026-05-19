//! Workspace aggregate: per-card materialization (clones + container + listener).

mod domain;
pub mod clones;
pub mod container;
pub mod fetch;
pub mod git;

pub use clones::clone_all_for_card;
pub use container::{ContainerOps, ContainerOpsHandle, make_container_ops};
pub use domain::{Container, Workspace};
pub use git::{FfOutcome, GitOps, GitOpsHandle, make_git_ops};
