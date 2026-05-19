//! Household aggregate: registered project bundle.

mod domain;
mod handler;
pub mod manifest;
mod repo;
mod scaffold;

pub use domain::{Household, iter_card_repos, load_household_manifest};
pub use handler::{
    NameParam, handle_household_add, handle_household_ls, handle_household_new,
    handle_household_rm, handle_household_show,
};
pub use manifest::{Manifest, RepoEntry};
pub use repo::{HouseholdRepo, HouseholdRow, init_schema, make_household_components};
pub use scaffold::{HouseholdScaffold, make_scaffold};
