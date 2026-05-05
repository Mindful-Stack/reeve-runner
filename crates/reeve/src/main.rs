mod cli;
mod client;
mod tui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "reeve", version, about = "Reeve TUI orchestrator")]
struct Args {
    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        None => tui::run(),
        Some(cli::Command::Daemon(sub)) => cli::run_daemon(sub),
        Some(cmd) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::run(cmd))
        }
    }
}
