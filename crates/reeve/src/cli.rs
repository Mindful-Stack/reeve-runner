//! CLI subcommands handlers.

use crate::client;
use anyhow::{Context, Result};
use clap::Subcommand;
use reeve_protocol::{Card, Column, LsResult, MvResult, NewResult, paths};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List cards.
    Ls,
    /// Create a new card in the Backlog column.
    New {
        #[arg(long)]
        household: String,
        #[arg(long)]
        brief: String,
        #[arg(long, default_value_t = false)]
        needs_heavy: bool,
    },
    /// Move a card to a different column.
    Mv {
        id: String,
        #[arg(value_parser = parse_column)]
        column: Column,
    },
    /// Daemon management subcommands.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Start the daemon as a background process.
    Start,
    /// Stop the running daemon (SIGTERM).
    Stop,
    /// Check whether the daemon is running.
    Status,
}

fn parse_column(s: &str) -> Result<Column, String> {
    match s.to_lowercase().as_str() {
        "backlog" => Ok(Column::Backlog),
        "planning" => Ok(Column::Planning),
        "implementing" => Ok(Column::Implementing),
        "testing" => Ok(Column::Testing),
        "review" => Ok(Column::Review),
        "done" => Ok(Column::Done),
        other => Err(format!(
            "unknown column {other:?}; expected one of {:?}",
            Column::ALL.iter().map(|c| c.label()).collect::<Vec<_>>()
        )),
    }
}

pub async fn run(cmd: Command) -> Result<()> {
    match cmd {
        Command::Ls => ls().await,
        Command::New { household, brief, needs_heavy } => new(household, brief, needs_heavy).await,
        Command::Mv { id, column } => mv(id, column).await,
        Command::Daemon(_) => unreachable!("Daemon commands are handled in main"),
    }
}

async fn ls() -> Result<()> {
    let socket = paths::socket_path();
    let result = client::call(&socket, "ls", serde_json::json!({}))
        .await
        .context("calling ls")?;
    match result {
        Ok(value) => {
            let LsResult { cards } = serde_json::from_value(value)?;
            print_cards(&cards);
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn new(household: String, brief: String, needs_heavy: bool) -> Result<()> {
    let socket = paths::socket_path();
    let result = client::call(
        &socket,
        "new",
        serde_json::json!({
            "household": household,
            "brief": brief,
            "needs_heavy": needs_heavy,
        }),
    )
    .await
    .context("calling new")?;
    match result {
        Ok(value) => {
            let NewResult { card } = serde_json::from_value(value)?;
            println!("created {} ({})", card.id, card.column.label());
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn mv(id: String, column: Column) -> Result<()> {
    let socket = paths::socket_path();
    let result = client::call(
        &socket,
        "mv",
        serde_json::json!({ "id": id, "column": column }),
    )
    .await
    .context("calling mv")?;
    match result {
        Ok(value) => {
            let MvResult { card } = serde_json::from_value(value)?;
            println!("{} -> {}", card.id, card.column.label());
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

fn print_cards(cards: &[Card]) {
    if cards.is_empty() {
        println!("(no cards)");
        return;
    }
    println!("{:<36}  {:<14}  {:<12}  BRIEF", "ID", "COLUMN", "HOUSEHOLD");
    for card in cards {
        // Truncate by chars (not bytes) so multibyte UTF-8 in briefs is safe.
        let brief: String = if card.brief.chars().count() > 60 {
            let head: String = card.brief.chars().take(59).collect();
            format!("{head}…")
        } else {
            card.brief.clone()
        };
        println!(
            "{:<36}  {:<14}  {:<12}  {}",
            card.id,
            card.column.label(),
            card.household,
            brief,
        );
    }
}
