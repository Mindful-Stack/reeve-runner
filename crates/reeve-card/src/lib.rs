//! Card aggregate: kanban metadata + column-transition policy.

mod transitions;

pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
