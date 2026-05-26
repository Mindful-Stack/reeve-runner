mod cli;
mod client;
mod tui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "reeve", version, about = "Reeve TUI orchestrator")]
struct Args {
    /// Scope the TUI's kanban (and the periodic `ls` poll) to a single
    /// household. Ignored when a subcommand is provided.
    #[arg(long, global = true)]
    household: Option<String>,

    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        None => tui::run(args.household),
        Some(cli::Command::Daemon(sub)) => cli::run_daemon(sub),
        Some(cmd) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(cli::run(cmd))
        }
    }
}
