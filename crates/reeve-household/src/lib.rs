//! Household aggregate: registered project bundle.

mod domain;
mod handler;
pub mod manifest;
mod repo;
mod scaffold;

pub use domain::{iter_card_repos, load_household_manifest, Household};
pub use handler::{
    handle_household_add, handle_household_ls, handle_household_new,
    handle_household_rm, handle_household_show, NameParam,
};
pub use manifest::{Manifest, RepoEntry};
pub use repo::{init_schema, make_household_components, HouseholdRepo, HouseholdRow};
pub use scaffold::{make_scaffold, HouseholdScaffold};
