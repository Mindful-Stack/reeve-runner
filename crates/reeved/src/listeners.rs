//! Per-card Unix-socket listeners. Each running card has its own
//! socket file at $XDG_RUNTIME_DIR/reeve/cards/<id>.sock; the daemon
//! resolves caller identity from which listener received the request.

use crate::transport::server::{CallerContext, DbHandle, run};
use anyhow::Result;
use reeve_protocol::{RpcError, RpcRequest};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;
use tracing::warn;

/// Type-erased dispatcher so the registry can hold a single reference
/// without carrying generic parameters everywhere.
pub trait Dispatcher: Send + Sync {
    fn dispatch(
        &self,
        req: RpcRequest,
        db: DbHandle,
        caller: CallerContext,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>>
                + Send,
        >,
    >;
}

#[derive(Clone)]
pub struct ListenerRegistry {
    inner: Arc<Mutex<HashMap<String, ListenerEntry>>>,
    dispatcher: Arc<dyn Dispatcher>,
    db: DbHandle,
}

struct ListenerEntry {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
    socket: PathBuf,
}

impl ListenerRegistry {
    pub fn new(db: DbHandle, dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            dispatcher,
            db,
        }
    }

    /// Spawn a listener for `card_id` at `socket_path`. Idempotent: a
    /// second call with the same id is a no-op.
    pub async fn spawn(&self, card_id: &str, socket_path: PathBuf) {
        let mut map = self.inner.lock().await;
        if map.contains_key(card_id) {
            return;
        }
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
        // server::run unlinks any existing file at the socket path before bind.

        let caller = CallerContext::card(card_id);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let socket_for_task = socket_path.clone();
        let card_for_task = card_id.to_string();
        let dispatcher = self.dispatcher.clone();
        let db = self.db.clone();
        let dispatch = move |req: RpcRequest, db: DbHandle, caller: CallerContext| {
            dispatcher.dispatch(req, db, caller)
        };
        let task = tokio::spawn(async move {
            if let Err(e) = run(&socket_for_task, db, caller, shutdown_rx, dispatch).await {
                warn!("per-card listener for {card_for_task} exited: {e:#}");
            }
        });
        map.insert(
            card_id.to_string(),
            ListenerEntry {
                shutdown: shutdown_tx,
                task,
                socket: socket_path,
            },
        );
    }

    /// Stop the listener for `card_id` and remove the socket file.
    /// No-op if no listener exists for this id.
    pub async fn stop(&self, card_id: &str) -> Result<()> {
        let entry = self.inner.lock().await.remove(card_id);
        if let Some(entry) = entry {
            let _ = entry.shutdown.send(());
            let _ = entry.task.await;
            let _ = std::fs::remove_file(&entry.socket);
        }
        Ok(())
    }

    /// Drop every listener (used on daemon shutdown).
    pub async fn stop_all(&self) {
        let entries: Vec<_> = self.inner.lock().await.drain().collect();
        for (_, entry) in entries {
            let _ = entry.shutdown.send(());
            let _ = entry.task.await;
            let _ = std::fs::remove_file(&entry.socket);
        }
    }

    /// Test-only: check whether a listener is registered for `card_id`.
    #[cfg(test)]
    pub async fn contains(&self, card_id: &str) -> bool {
        self.inner.lock().await.contains_key(card_id)
    }

    /// Best-effort: unlink any socket file at `path` before binding.
    /// Per-card socket files survive daemon shutdown (containers do too).
    pub fn unlink_stale(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    /// Test dispatcher: echoes the calling card_id back to the client.
    struct EchoCaller;
    impl Dispatcher for EchoCaller {
        fn dispatch(
            &self,
            _req: RpcRequest,
            _db: DbHandle,
            caller: CallerContext,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>>
                    + Send,
            >,
        > {
            Box::pin(async move { Ok(json!({ "card_id": caller.card_id })) })
        }
    }

    fn fresh_db() -> DbHandle {
        Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))
    }

    fn registry() -> ListenerRegistry {
        ListenerRegistry::new(fresh_db(), Arc::new(EchoCaller))
    }

    async fn connect_with_retry(socket: &Path) -> UnixStream {
        for _ in 0..50 {
            if let Ok(s) = UnixStream::connect(socket).await {
                return s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("connect timeout: {}", socket.display());
    }

    #[tokio::test]
    async fn listener_passes_card_id_in_caller_context() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let reg = registry();

        reg.spawn("test-card", socket.clone()).await;

        let mut stream = connect_with_retry(&socket).await;
        let req = RpcRequest {
            id: 1,
            method: "noop".into(),
            params: serde_json::json!({}),
        };
        let mut bytes = serde_json::to_vec(&req).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains("\"card_id\":\"test-card\""), "got: {line}");

        reg.stop("test-card").await.unwrap();
    }

    #[tokio::test]
    async fn spawn_is_idempotent_for_same_card_id() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let reg = registry();

        reg.spawn("c", socket.clone()).await;
        reg.spawn("c", socket.clone()).await;
        assert!(reg.contains("c").await);

        reg.stop("c").await.unwrap();
    }

    #[tokio::test]
    async fn stop_removes_socket_file() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("c.sock");
        let reg = registry();

        reg.spawn("c", socket.clone()).await;
        for _ in 0..50 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(socket.exists());

        reg.stop("c").await.unwrap();
        assert!(!socket.exists());
    }
}
