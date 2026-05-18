//! Household aggregate: registered project bundle.

mod domain;
pub mod manifest;
mod repo;
mod scaffold;

pub use domain::{load_household_manifest, Household};
pub use repo::{init_schema, make_household_components, HouseholdRepo, HouseholdRow};
pub use scaffold::{make_scaffold, HouseholdScaffold};
