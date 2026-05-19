//! Card aggregate: kanban metadata + column-transition policy.

mod domain;
mod handler;
mod repo;
mod transitions;

pub use domain::{Card, CardError};
pub use handler::{handle_delete, handle_edit, handle_ls, handle_mv, handle_mv_self, handle_new, handle_status_self};
pub use repo::{CardRepo, init_schema, make_card_repo};
pub use transitions::{HouseholdLookup, WorkspaceLifecycle, on_column_change};
