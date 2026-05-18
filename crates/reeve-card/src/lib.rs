//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod handler;
mod repo;
mod transitions;

pub use domain::Card;
pub use handler::{
    handle_ls, handle_mv, handle_mv_self, handle_new, handle_status_self,
};
pub use repo::{init_schema, make_card_repo, CardRepo};
pub use transitions::{on_column_change, HouseholdLookup, WorkspaceLifecycle};
