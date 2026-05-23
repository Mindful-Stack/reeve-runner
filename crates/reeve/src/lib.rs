//! Library facade for the `reeve` crate.
//!
//! Exposes the IPC client so that integration tests (and other crates that
//! need to talk to `reeved` directly without going through the CLI) can
//! `use reeve::client::call`.

pub mod client;
