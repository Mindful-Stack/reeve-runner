//! Minimal read-only Ratatui kanban viewer for M1.

mod app;
mod render;

use anyhow::Result;

pub fn run() -> Result<()> {
    app::run()
}
