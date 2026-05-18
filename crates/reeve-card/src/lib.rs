//! Card aggregate: kanban metadata + column-transition policy.

mod repo;
mod transitions;

pub use repo::{init_schema, make_card_repo, CardRepo};
pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
