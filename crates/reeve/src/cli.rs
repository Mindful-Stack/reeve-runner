//! CLI subcommands handlers.

use crate::client;
use anyhow::{Context, Result};
use clap::Subcommand;
use reeve_protocol::{
    AttachResult, BranchPushStatus, Card, Column, HouseholdAddParams, HouseholdDetail,
    HouseholdNewParams, HouseholdSource, HouseholdSummary, LsResult, MvResult, MvSelfResult,
    NewResult, PushResult, RepoOnDisk, SessionInitSelfResult, StatusSelfResult, paths,
};
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
    /// Drop into the running claude session inside a card's container.
    /// The card must be in an active column (Planning, Implementing, Testing,
    /// or Review). Uses tmux for detach/reattach.
    /// Ctrl-B D detaches; claude keeps running.
    Attach { id: String },
    /// List cards. With `--household NAME`, scope the listing to one household.
    Ls {
        #[arg(long)]
        household: Option<String>,
    },
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
    /// (in-container only) Move the calling card to a different column.
    MvSelf {
        #[arg(value_parser = parse_column)]
        column: Column,
    },
    /// Promote namespaced card branches to refs/heads/ and push to origin.
    Push { id: String },
    /// (in-container only) Print the calling card and its container status.
    StatusSelf,
    /// (in-container only) Print the SessionStart prompt for the calling
    /// card. Consumed by claude's SessionStart hook to seed the agent.
    SessionInitSelf,
    /// Household management subcommands.
    #[command(subcommand)]
    Household(HouseholdCommand),
    /// Daemon management subcommands.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Subcommand, Debug)]
pub enum HouseholdCommand {
    /// Register an existing meta-repo (path) or clone-then-register one (URL).
    Add {
        name: String,
        /// Filesystem path OR git URL.
        source: String,
        /// Where to clone (only meaningful for URL sources). Default ~/Source/<name>/.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Scaffold a fresh household from the witan-household template.
    New {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        template: Option<String>,
    },
    /// List registered households.
    Ls,
    /// Unregister a household (does not delete the meta-repo).
    Rm { name: String },
    /// Show manifest + sibling presence + referencing cards.
    Show { name: String },
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
    Column::from_wire_str(&s.to_lowercase()).ok_or_else(|| {
        format!(
            "unknown column {s:?}; expected one of {:?}",
            Column::ALL.iter().map(|c| c.label()).collect::<Vec<_>>()
        )
    })
}

pub async fn run(cmd: Command) -> Result<()> {
    match cmd {
        Command::Attach { id } => attach(id).await,
        Command::Ls { household } => ls(household).await,
        Command::New {
            household,
            brief,
            needs_heavy,
        } => new(household, brief, needs_heavy).await,
        Command::Mv { id, column } => mv(id, column).await,
        Command::MvSelf { column } => mv_self(column).await,
        Command::Push { id } => push(id).await,
        Command::StatusSelf => status_self().await,
        Command::SessionInitSelf => session_init_self().await,
        Command::Household(cmd) => household_run(cmd).await,
        Command::Daemon(_) => unreachable!("Daemon commands are handled in main"),
    }
}

async fn attach(id: String) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "attach", serde_json::json!({ "id": id }))
        .await
        .context("calling attach")?;
    match result {
        Ok(value) => {
            let result: AttachResult = serde_json::from_value(value)?;
            println!(
                "attaching to {}:{} (ctrl-b d to detach, claude keeps running)",
                result.container_name, result.session_name
            );
            // Process-replace ourselves with `docker exec -u vscode -it <container>
            // tmux attach -t <session>`. The `-u vscode` matches what
            // `reeve_workspace::container::DockerOps::exec_in_tmux` uses on the
            // daemon side — tmux servers are per-uid, so client and daemon
            // must agree on the user or the client lands on a different
            // (empty) tmux socket and exits with 'no server running'.
            // `CommandExt::exec` either replaces the process (never returns) or
            // returns the error that prevented exec from succeeding.
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new("docker")
                .args([
                    "exec",
                    "-u",
                    "vscode",
                    "-e",
                    "TERM=xterm-256color",
                    "-e",
                    "LANG=C.UTF-8",
                    "-e",
                    "LC_ALL=C.UTF-8",
                    "-it",
                    &result.container_name,
                    "tmux",
                    "attach",
                    "-t",
                    &result.session_name,
                ])
                .exec();
            anyhow::bail!("docker exec failed to start: {err}");
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn ls(household: Option<String>) -> Result<()> {
    let socket = paths::detect_socket();
    let params = match household {
        Some(h) => serde_json::json!({ "household": h }),
        None => serde_json::json!({}),
    };
    let result = client::call(&socket, "ls", params)
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
    let socket = paths::detect_socket();
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
    let socket = paths::detect_socket();
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

async fn mv_self(column: Column) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "mv-self", serde_json::json!({"column": column}))
        .await
        .context("calling mv-self")?;
    match result {
        Ok(value) => {
            let result: MvSelfResult = serde_json::from_value(value)?;
            println!("{} -> {}", result.card.id, result.card.column.label());
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn status_self() -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "status-self", serde_json::json!({}))
        .await
        .context("calling status-self")?;
    match result {
        Ok(value) => {
            let r: StatusSelfResult = serde_json::from_value(value)?;
            println!("{} ({})", r.card.id, r.card.column.label(),);
            match r.container_status {
                Some(s) => println!("container: {}", s.as_wire_str()),
                None => println!("container: (none)"),
            }
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn session_init_self() -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "session-init-self", serde_json::json!({}))
        .await
        .context("calling session-init-self")?;
    match result {
        Ok(value) => {
            let r: SessionInitSelfResult = serde_json::from_value(value)?;
            print!("{}", r.prompt);
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn push(id: String) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "push", serde_json::json!({ "id": id }))
        .await
        .context("calling push")?;
    match result {
        Ok(value) => {
            let result: PushResult = serde_json::from_value(value)?;
            if result.branches.is_empty() {
                println!(
                    "(no branches in refs/reeve/{id}/* — has the agent committed anything yet?)"
                );
                return Ok(());
            }
            // Print a per-branch table.
            println!("{:<24}  {:<32}  STATUS", "REPO", "BRANCH");
            let mut any_failed = false;
            for b in &result.branches {
                let status_str = match &b.status {
                    BranchPushStatus::Pushed => "pushed".to_string(),
                    BranchPushStatus::NotFastForward => {
                        any_failed = true;
                        "not-fast-forward".to_string()
                    }
                    BranchPushStatus::PushFailed { message } => {
                        any_failed = true;
                        format!("push-failed: {message}")
                    }
                };
                println!("{:<24}  {:<32}  {}", b.repo, b.branch, status_str);
            }
            if any_failed {
                anyhow::bail!("one or more branches did not push successfully");
            }
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

fn is_git_url(s: &str) -> bool {
    s.starts_with("git@")
        || s.starts_with("git://")
        || s.starts_with("https://")
        || s.starts_with("http://")
        || s.starts_with("file://")
        || s.starts_with("ssh://")
}

async fn household_run(cmd: HouseholdCommand) -> Result<()> {
    match cmd {
        HouseholdCommand::Add { name, source, path } => household_add(name, source, path).await,
        HouseholdCommand::New {
            name,
            path,
            template,
        } => household_new(name, path, template).await,
        HouseholdCommand::Ls => household_ls().await,
        HouseholdCommand::Rm { name } => household_rm(name).await,
        HouseholdCommand::Show { name } => household_show(name).await,
    }
}

async fn household_add(name: String, source: String, path: Option<PathBuf>) -> Result<()> {
    let src = if is_git_url(&source) {
        HouseholdSource::Url { url: source }
    } else {
        HouseholdSource::Path {
            path: PathBuf::from(source),
        }
    };
    let socket = paths::detect_socket();
    let params = HouseholdAddParams {
        name,
        source: src,
        clone_into: path,
    };
    let result = client::call(&socket, "household.add", serde_json::to_value(params)?)
        .await
        .context("calling household.add")?;
    match result {
        Ok(value) => {
            let summary: HouseholdSummary = serde_json::from_value(value)?;
            println!(
                "Registered {} at {}",
                summary.name,
                summary.meta_repo_path.display()
            );
            println!("  workspace: {}", summary.workspace_name);
            if let Some(kb) = summary.knowledge_base {
                println!("  knowledge_base: {kb}");
            }
            println!("  repos: {}", summary.repo_count);
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn household_new(
    name: String,
    path: Option<PathBuf>,
    template: Option<String>,
) -> Result<()> {
    let socket = paths::detect_socket();
    let params = HouseholdNewParams {
        name,
        path,
        template,
    };
    let result = client::call(&socket, "household.new", serde_json::to_value(params)?)
        .await
        .context("calling household.new")?;
    match result {
        Ok(value) => {
            let summary: HouseholdSummary = serde_json::from_value(value)?;
            println!(
                "Scaffolded {} at {}",
                summary.name,
                summary.meta_repo_path.display()
            );
            println!("\nNext steps:");
            println!("  cd {}", summary.meta_repo_path.display());
            println!("  $EDITOR household.json   # declare your sibling repos");
            println!("  git remote add origin <url>   # if you want to push it to a remote");
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn household_ls() -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "household.ls", serde_json::json!({}))
        .await
        .context("calling household.ls")?;
    match result {
        Ok(value) => {
            let list: Vec<HouseholdSummary> = serde_json::from_value(value)?;
            if list.is_empty() {
                println!(
                    "(no households registered — run `reeve household add` or `reeve household new`)"
                );
            } else {
                for h in list {
                    println!(
                        "{:<24} {} ({} repos)",
                        h.name,
                        h.meta_repo_path.display(),
                        h.repo_count
                    );
                }
            }
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn household_rm(name: String) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(&socket, "household.rm", serde_json::json!({ "name": name }))
        .await
        .context("calling household.rm")?;
    match result {
        Ok(_) => {
            println!("Unregistered {} (meta-repo on disk untouched)", name);
            Ok(())
        }
        Err(e) => anyhow::bail!("daemon error: {e}"),
    }
}

async fn household_show(name: String) -> Result<()> {
    let socket = paths::detect_socket();
    let result = client::call(
        &socket,
        "household.show",
        serde_json::json!({ "name": name }),
    )
    .await
    .context("calling household.show")?;
    match result {
        Ok(value) => {
            let detail: HouseholdDetail = serde_json::from_value(value)?;
            println!(
                "{} ({})",
                detail.summary.name,
                detail.summary.meta_repo_path.display()
            );
            println!("  workspace: {}", detail.summary.workspace_name);
            if let Some(url) = detail.workspace_url {
                println!("  url: {}", url);
            }
            if let Some(kb) = &detail.summary.knowledge_base {
                println!("  knowledge_base: {} (→ repo entry of the same name)", kb);
            }
            println!("\n  Repos ({}):", detail.repos.len());
            for r in detail.repos {
                let glyph = match r.on_disk {
                    RepoOnDisk::Workspace => "W",
                    RepoOnDisk::Sibling => "✓",
                    RepoOnDisk::Inline => "·",
                    RepoOnDisk::Missing => "✗",
                };
                let kb_marker = if r.is_knowledge_base { " [KB]" } else { "" };
                println!("    {glyph} {}{kb_marker}", r.name);
            }
            if detail.cards.is_empty() {
                println!("\n  No cards reference this household.");
            } else {
                println!("\n  Cards referencing this household:");
                for c in detail.cards {
                    println!("    {} [{}] {}", c.id, c.column, c.brief);
                }
            }
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
    if let Ok(Some(pid)) = reeve_protocol::lifecycle::read_pid_file(&pid_path) {
        if reeve_protocol::lifecycle::process_running(pid) {
            anyhow::bail!("daemon already running as PID {pid}");
        }
    }

    let reeved = reeved_path()?;
    let child = std::process::Command::new(&reeved)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", reeved.display()))?;

    // Give the daemon a moment to write its pid file. Accept either the
    // child we just spawned or any other live PID — concurrent `daemon
    // start` invocations may race, and the survivor is fine to report.
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < std::time::Duration::from_secs(2) {
        if let Some(pid) = reeve_protocol::lifecycle::read_pid_file(&pid_path)? {
            if pid == child.id() || reeve_protocol::lifecycle::process_running(pid) {
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
    let pid = reeve_protocol::lifecycle::read_pid_file(&pid_path)?
        .context("daemon not running (no pid file)")?;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::SIGTERM,
    )
    .with_context(|| format!("send SIGTERM to PID {pid}"))?;

    // Wait up to 5s for the process to exit (matches server.rs SHUTDOWN_GRACE).
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < std::time::Duration::from_secs(5) {
        if !reeve_protocol::lifecycle::process_running(pid) {
            println!("daemon stopped (was PID {pid})");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("daemon (PID {pid}) did not exit within 5s of SIGTERM");
}

fn status() -> Result<()> {
    let pid_path = paths::pid_path();
    match reeve_protocol::lifecycle::read_pid_file(&pid_path)? {
        Some(pid) if reeve_protocol::lifecycle::process_running(pid) => {
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
