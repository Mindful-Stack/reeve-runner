//! Workspace aggregate: per-card materialization (clones + container + listener).

pub mod clones;
pub mod container;
mod domain;
pub mod fetch;
pub mod git;
mod handler;
mod lifecycle;
pub mod progress;
mod repo;

pub use clones::clone_all_for_card;
pub use container::{ContainerOps, ContainerOpsHandle, make_container_ops};
pub use domain::{Container, Workspace};
pub use git::{FfOutcome, GitOps, GitOpsHandle, make_git_ops};
pub use handler::{handle_attach, handle_push};
pub use lifecycle::{ListenerHandle, ListenerOps, LiveWorkspaceLifecycle};
pub use progress::{ProgressHandle, ProgressReporter, run_stage};
pub use repo::{ContainerRow, WorkspaceRepo, init_schema, make_workspace_repo};
