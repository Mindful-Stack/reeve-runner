//! Household aggregate: registered project bundle.

mod domain;
pub mod manifest;

pub use domain::{load_household_manifest, Household};
