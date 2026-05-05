mod db;
mod handlers;
mod lifecycle;
mod server;

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, oneshot};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reeved=info".into()),
        )
        .init();

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

    let result = server::run(&socket_path, db, shutdown_rx, handlers::dispatch).await;

    lifecycle::remove_pid_file(&pid_path)?;
    info!("reeved stopped");
    result
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
