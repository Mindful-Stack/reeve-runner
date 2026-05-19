mod app;
mod events;
mod render;

use anyhow::Result;

pub fn run() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(app::run_loop())
}
