mod clones;
mod container;
mod db;
mod effects;
mod handlers;
mod household;
mod lifecycle;
mod listeners;
mod manifest;
mod server;
mod transitions;

use anyhow::{Context, Result};
use nix::sys::stat::Mode;
use reeve_protocol::RpcRequest;
use server::{CallerContext, DbHandle};
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

    let socket_path = lifecycle::socket_path();
    let pid_path = lifecycle::pid_path();
    let db_path = lifecycle::db_path()?;

    lifecycle::ensure_parent_dir(&db_path)?;
    lifecycle::ensure_parent_dir(&socket_path)?;

    if let Some(existing_pid) = lifecycle::read_pid_file(&pid_path)? {
        if lifecycle::process_running(existing_pid) {
            anyhow::bail!(
                "reeved already running as PID {existing_pid} ({}). \
                 Stop it first with `reeve daemon stop`.",
                pid_path.display()
            );
        } else {
            // Stale PID file — overwrite.
            lifecycle::remove_pid_file(&pid_path)?;
        }
    }

    let conn = db::open(db_path.to_str().context("db path not utf-8")?)?;
    let db = Arc::new(Mutex::new(conn));

    lifecycle::write_pid_file(&pid_path)?;
    info!(socket = %socket_path.display(), "reeved started");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    spawn_signal_handler(shutdown_tx);

    let dispatcher_holder: Arc<MainDispatcher> = Arc::new(MainDispatcher {
        state: std::sync::OnceLock::new(),
    });

    let households = {
        let conn = db.lock().await;
        household::load_households(&conn)?
    };
    let containers: container::ContainerOpsHandle = Arc::new(container::DockerOps);
    let listeners = listeners::ListenerRegistry::new(db.clone(), dispatcher_holder.clone());
    let claude_dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".claude"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent"));
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();

    let git: clones::GitOpsHandle = Arc::new(clones::RealGitOps);
    let state = effects::DaemonState {
        db: db.clone(),
        containers,
        git,
        listeners: listeners.clone(),
        households,
        claude_dir,
        api_key,
    };
    dispatcher_holder
        .state
        .set(state.clone())
        .map_err(|_| anyhow::anyhow!("dispatcher state already initialised"))?;

    container::reconcile_on_startup(&state).await?;

    let state_for_dispatch = state.clone();
    let dispatch = move |req: RpcRequest, _db: DbHandle, caller: CallerContext| {
        let state = state_for_dispatch.clone();
        async move { handlers::dispatch(req, state, caller).await }
    };

    let result = server::run(
        &socket_path,
        db,
        server::CallerContext::host(),
        shutdown_rx,
        dispatch,
    )
    .await;

    listeners.stop_all().await;

    if let Err(e) = lifecycle::remove_pid_file(&pid_path) {
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
    state: std::sync::OnceLock<effects::DaemonState>,
}

impl listeners::Dispatcher for MainDispatcher {
    fn dispatch(
        &self,
        req: reeve_protocol::RpcRequest,
        _db: server::DbHandle,
        caller: server::CallerContext,
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
                Some(state) => handlers::dispatch(req, state, caller).await,
                None => Err(reeve_protocol::RpcError {
                    code: reeve_protocol::RpcError::INTERNAL,
                    message: "dispatcher not initialised".into(),
                }),
            }
        })
    }
}
