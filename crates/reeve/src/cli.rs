//! CLI subcommands handlers.

use crate::client;
use anyhow::{Context, Result};
use clap::Subcommand;
use reeve_protocol::{Card, Column, LsResult, MvResult, NewResult, paths};
use std::path::PathBuf;
use std::process::Stdio;

fn reeved_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locate current_exe")?;
    let parent = exe.parent().context("current_exe has no parent")?;
    let candidate = parent.join("reeved");
    if candidate.exists() {
        Ok(candidate)
    } else if let Ok(p) = std::env::var("REEVED_PATH") {
        Ok(PathBuf::from(p))
    } else {
        anyhow::bail!(
            "reeved binary not found at {}; set REEVED_PATH if it lives elsewhere",
            candidate.display()
        )
    }
}

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

pub fn run_daemon(cmd: DaemonCommand) -> Result<()> {
    match cmd {
        DaemonCommand::Start => start(),
        DaemonCommand::Stop => stop(),
        DaemonCommand::Status => status(),
    }
}

fn start() -> Result<()> {
    let pid_path = paths::pid_path();
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            if process_running(pid as u32) {
                anyhow::bail!("daemon already running as PID {pid}");
            }
        }
    }

    let reeved = reeved_path()?;
    let child = std::process::Command::new(&reeved)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", reeved.display()))?;

    // Give the daemon a moment to write its pid file.
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < std::time::Duration::from_secs(2) {
        if let Some(pid) = read_pid_from(&pid_path)? {
            if pid == child.id() {
                println!("daemon started (PID {pid})");
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    anyhow::bail!("daemon did not write a PID file within 2s");
}

fn stop() -> Result<()> {
    let pid_path = paths::pid_path();
    let pid = read_pid_from(&pid_path)?
        .context("daemon not running (no pid file)")?;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::SIGTERM,
    )
    .with_context(|| format!("send SIGTERM to PID {pid}"))?;

    // Wait up to 5s for the process to exit.
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < std::time::Duration::from_secs(5) {
        if !process_running(pid) {
            println!("daemon stopped (was PID {pid})");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("daemon (PID {pid}) did not exit within 5s of SIGTERM");
}

fn status() -> Result<()> {
    let pid_path = paths::pid_path();
    match read_pid_from(&pid_path)? {
        Some(pid) if process_running(pid) => {
            println!("running (PID {pid})");
            Ok(())
        }
        Some(pid) => {
            println!("not running (stale PID file references {pid})");
            Ok(())
        }
        None => {
            println!("not running");
            Ok(())
        }
    }
}

fn read_pid_from(path: &std::path::Path) -> Result<Option<u32>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s.trim().parse::<u32>()?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e)),
    }
}

fn process_running(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}
