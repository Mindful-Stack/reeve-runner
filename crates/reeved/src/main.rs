mod clones;
mod dispatch;
mod reconcile;

mod app;
mod transport;

use anyhow::{Context, Result};
use nix::sys::stat::Mode;
use reeve_protocol::RpcRequest;
use transport::server::{CallerContext, DbHandle};
use transport::listener;
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, oneshot};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reeved=info".into()),
        )
        .init();

    warn_if_old_toml_exists();

    // Mask group/other on every file we create from here on. The Unix socket,
    // pid file, and runtime/data directories are all per-user; nobody else
    // should have access. Without this, the /tmp fallback in lifecycle::runtime_dir
    // would inherit the ambient umask and could leak the socket world-readable.
    nix::sys::stat::umask(Mode::from_bits_truncate(0o077));

    let socket_path = app::lifecycle::socket_path();
    let pid_path = app::lifecycle::pid_path();
    let db_path = app::lifecycle::db_path()?;

    app::lifecycle::ensure_parent_dir(&db_path)?;
    app::lifecycle::ensure_parent_dir(&socket_path)?;

    if let Some(existing_pid) = app::lifecycle::read_pid_file(&pid_path)? {
        if app::lifecycle::process_running(existing_pid) {
            anyhow::bail!(
                "reeved already running as PID {existing_pid} ({}). \
                 Stop it first with `reeve daemon stop`.",
                pid_path.display()
            );
        } else {
            // Stale PID file — overwrite.
            app::lifecycle::remove_pid_file(&pid_path)?;
        }
    }

    let conn = app::lifecycle::open_db(db_path.to_str().context("db path not utf-8")?)?;
    let db = Arc::new(Mutex::new(conn));

    app::lifecycle::write_pid_file(&pid_path)?;
    info!(socket = %socket_path.display(), "reeved started");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    spawn_signal_handler(shutdown_tx);

    let dispatcher_holder: Arc<MainDispatcher> = Arc::new(MainDispatcher {
        state: std::sync::OnceLock::new(),
    });

    let containers: reeve_workspace::ContainerOpsHandle = reeve_workspace::make_container_ops();
    let listeners = listener::ListenerRegistry::new(db.clone(), dispatcher_holder.clone());
    let claude_dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".claude"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent"));
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();

    let git: clones::GitOpsHandle = reeve_workspace::make_git_ops();
    let (household_repo, household_lookup) =
        reeve_household::make_household_components(db.clone());

    let cards = reeve_card::make_card_repo(db.clone());
    let workspaces = reeve_workspace::make_workspace_repo(db.clone());
    let listeners_arc: reeve_workspace::ListenerHandle = Arc::new(listeners.clone());

    let workspace_lifecycle: Arc<dyn reeve_card::WorkspaceLifecycle> = Arc::new(
        reeve_workspace::LiveWorkspaceLifecycle::new(
            workspaces.clone(),
            household_repo.clone(),
            git.clone(),
            containers.clone(),
            listeners_arc,
            claude_dir,
            api_key,
        )
    );

    let state = app::state::DaemonState {
        db: db.clone(),
        cards,
        workspaces,
        containers,
        git,
        listeners: listeners.clone(),
        household_repo,
        household_lookup,
        workspace_lifecycle,
    };
    dispatcher_holder
        .state
        .set(state.clone())
        .map_err(|_| anyhow::anyhow!("dispatcher state already initialised"))?;

    reconcile::reconcile_on_startup(&state).await?;

    // Spawn the fetch worker. Hold its shutdown sender; we'll fire it after
    // server::run returns.
    let (fetch_shutdown_tx, fetch_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let fetch_worker = tokio::spawn(reeve_workspace::fetch::run(state.clone(), fetch_shutdown_rx));

    let state_for_dispatch = state.clone();
    let dispatch = move |req: RpcRequest, _db: DbHandle, caller: CallerContext| {
        let state = state_for_dispatch.clone();
        async move { dispatch::dispatch(req, state, caller).await }
    };

    let result = transport::server::run(
        &socket_path,
        db,
        transport::server::CallerContext::host(),
        shutdown_rx,
        dispatch,
    )
    .await;

    // Shut the fetch worker down and wait for it to exit. Ignore send error
    // (channel may already be closed if the worker died early — e.g. panicked).
    let _ = fetch_shutdown_tx.send(());
    if let Err(e) = fetch_worker.await {
        warn!("fetch worker join error on shutdown: {e:#}");
    }

    listeners.stop_all().await;

    if let Err(e) = app::lifecycle::remove_pid_file(&pid_path) {
        warn!("failed to remove pid file on shutdown: {e:#}");
    }
    info!(socket = %socket_path.display(), "reeved stopped");
    result
}

fn warn_if_old_toml_exists() {
    let dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".config/reeve/households"))
        .ok();
    let Some(dir) = dir else { return };
    if !dir.is_dir() {
        return;
    }
    let mut found_toml = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("toml") {
                found_toml = true;
                break;
            }
        }
    }
    if found_toml {
        tracing::warn!(
            "Old-style household TOML files found in {} — M3.5 uses sqlite-backed households, \
             not TOML. Re-register each household via `reeve household add <name> <path>` \
             pointing at its meta-repo. The TOML files are otherwise ignored.",
            dir.display(),
        );
    }
}

fn spawn_signal_handler(tx: oneshot::Sender<()>) {
    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM");
        let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT");
        tokio::select! {
            _ = sigterm.recv() => {},
            _ = sigint.recv() => {},
        }
        let _ = tx.send(());
    });
}

struct MainDispatcher {
    state: std::sync::OnceLock<app::state::DaemonState>,
}

impl listener::Dispatcher for MainDispatcher {
    fn dispatch(
        &self,
        req: reeve_protocol::RpcRequest,
        _db: transport::server::DbHandle,
        caller: transport::server::CallerContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<serde_json::Value, reeve_protocol::RpcError>,
                > + Send,
        >,
    > {
        let state = self.state.get().cloned();
        Box::pin(async move {
            match state {
                Some(state) => dispatch::dispatch(req, state, caller).await,
                None => Err(reeve_protocol::RpcError {
                    code: reeve_protocol::RpcError::INTERNAL,
                    message: "dispatcher not initialised".into(),
                }),
            }
        })
    }
}
