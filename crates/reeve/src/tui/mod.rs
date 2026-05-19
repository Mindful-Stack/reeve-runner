mod actions;
mod app;
mod client_ext;
mod events;
mod render;

use anyhow::Result;

pub fn run() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(app::run_loop())
}
