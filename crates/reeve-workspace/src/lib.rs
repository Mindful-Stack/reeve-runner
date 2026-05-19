//! Workspace aggregate: per-card materialization (clones + container + listener).

pub mod container;

pub use container::{ContainerOps, ContainerOpsHandle, make_container_ops};
