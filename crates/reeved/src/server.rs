//! Unix-socket JSON-line server.

use anyhow::{Context, Result};
use reeve_protocol::{RpcError, RpcRequest, RpcResponse};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinSet;
use tracing::{error, warn};

/// A request handler shared across connections.
pub type DbHandle = Arc<Mutex<Connection>>;

/// Time we'll wait for in-flight connections to finish after `shutdown` fires
/// before forcibly returning. Prevents a slow client from holding the daemon
/// open forever.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Run the server on `socket_path`. Removes the socket on shutdown and waits
/// (with a bounded grace period) for in-flight connections to drain before
/// returning, so the caller can assume exclusive access to `db` once
/// `run()` resolves.
///
/// `dispatch` receives the parsed request and produces a result or RpcError.
pub async fn run<F, Fut>(
    socket_path: &Path,
    db: DbHandle,
    mut shutdown: oneshot::Receiver<()>,
    dispatch: F,
) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("remove stale socket {}", socket_path.display()))?;
    }
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;

    // Restrict the socket to user-only access regardless of inherited umask
    // or the host's $XDG_RUNTIME_DIR mode. Combined with the umask(0o077)
    // call in main.rs, this makes the daemon socket the trust boundary for
    // in-container `reeve` calls (per the design spec).
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 {}", socket_path.display()))?;

    let mut connections: JoinSet<()> = JoinSet::new();

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        let db = db.clone();
                        let dispatch = dispatch.clone();
                        connections.spawn(async move {
                            if let Err(e) = handle_connection(stream, db, dispatch).await {
                                warn!("connection error: {e:#}");
                            }
                        });
                    }
                    Err(e) => {
                        error!("accept failed: {e:#}");
                    }
                }
            }
        }
    }

    drop(listener);

    // Drain in-flight connections within the grace period; then move on.
    let drain = async {
        while connections.join_next().await.is_some() {}
    };
    if tokio::time::timeout(SHUTDOWN_GRACE, drain).await.is_err() {
        warn!(
            "{} connection(s) still in flight after {:?}; aborting",
            connections.len(),
            SHUTDOWN_GRACE
        );
        connections.abort_all();
        // Best-effort: let aborted tasks resolve.
        while connections.join_next().await.is_some() {}
    }

    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

async fn handle_connection<F, Fut>(stream: UnixStream, db: DbHandle, dispatch: F) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    let (read, mut write) = stream.into_split();
    // SECURITY: BufReader::lines() has no per-line size cap. v1 accepts this
    // because the socket is chmod'd to 0600 by `run()` immediately after bind
    // (and main.rs sets umask 0077), so only the daemon's user can connect.
    // Worst case is the user DoS'ing themselves. If socket permissions ever
    // widen, switch to a length-prefixed framing first.
    let mut lines = BufReader::new(read).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => {
                let id = req.id;
                match dispatch(req, db.clone()).await {
                    Ok(value) => RpcResponse {
                        id: Some(id),
                        result: Some(value),
                        error: None,
                    },
                    Err(err) => RpcResponse {
                        id: Some(id),
                        result: None,
                        error: Some(err),
                    },
                }
            }
            Err(e) => RpcResponse {
                id: None,
                result: None,
                error: Some(RpcError {
                    code: RpcError::INVALID_PARAMS,
                    message: format!("malformed request: {e}"),
                }),
            },
        };
        let mut bytes = serde_json::to_vec(&resp)?;
        bytes.push(b'\n');
        write.write_all(&bytes).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn echo_db() -> DbHandle {
        Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()))
    }

    /// Connect to `socket` once it's accepting, retrying briefly to absorb
    /// the gap between bind and the first poll of accept().
    async fn connect_with_retry(socket: &Path) -> UnixStream {
        for _ in 0..50 {
            if let Ok(stream) = UnixStream::connect(socket).await {
                return stream;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("could not connect to {} within 1s", socket.display());
    }

    #[tokio::test]
    async fn server_round_trips_request_and_response() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("test.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();

        let dispatch = |req: RpcRequest, _db: DbHandle| async move { Ok(req.params) };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        let mut stream = connect_with_retry(&socket).await;
        let req = RpcRequest {
            id: 42,
            method: "echo".into(),
            params: serde_json::json!({"hi": "there"}),
        };
        let mut bytes = serde_json::to_vec(&req).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: RpcResponse = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(resp.id, Some(42));
        assert_eq!(resp.result, Some(serde_json::json!({"hi": "there"})));
        assert!(resp.error.is_none());

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_request_yields_invalid_params_error_with_no_id() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("bad.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();
        let dispatch = |_req: RpcRequest, _db: DbHandle| async {
            Ok(serde_json::Value::Null)
        };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        let mut stream = connect_with_retry(&socket).await;
        stream.write_all(b"not json\n").await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: RpcResponse = serde_json::from_str(line.trim()).unwrap();
        assert!(resp.id.is_none());
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, RpcError::INVALID_PARAMS);
        assert!(!err.message.is_empty());

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn server_handles_multiple_requests_on_one_connection() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("multi.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();
        let dispatch = |req: RpcRequest, _db: DbHandle| async move { Ok(req.params) };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        let mut stream = connect_with_retry(&socket).await;
        for id in [1u64, 2, 3] {
            let req = RpcRequest {
                id,
                method: "echo".into(),
                params: serde_json::json!({"n": id}),
            };
            let mut bytes = serde_json::to_vec(&req).unwrap();
            bytes.push(b'\n');
            stream.write_all(&bytes).await.unwrap();
        }
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        for id in [1u64, 2, 3] {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let resp: RpcResponse = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(resp.id, Some(id));
            assert_eq!(resp.result, Some(serde_json::json!({"n": id})));
        }

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_returns_even_with_idle_connection_open() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("idle.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();
        let dispatch = |req: RpcRequest, _db: DbHandle| async move { Ok(req.params) };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        // Hold a connection open with no requests. Shutdown should still drain.
        let stream = connect_with_retry(&socket).await;

        let _ = tx.send(());
        // Server returns once we drop the idle connection (causing EOF) OR when
        // the grace period expires. Drop the stream to let the connection
        // close immediately; server's drain loop sees the task complete.
        drop(stream);
        let result = tokio::time::timeout(Duration::from_secs(2), server_task).await;
        assert!(result.is_ok(), "server did not shut down within 2s");
    }
}
