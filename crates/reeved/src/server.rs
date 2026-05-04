//! Unix-socket JSON-line server.

use anyhow::{Context, Result};
use reeve_protocol::{RpcError, RpcRequest, RpcResponse};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};
use tracing::{error, warn};

/// A request handler shared across connections.
pub type DbHandle = Arc<Mutex<Connection>>;

/// Run the server on `socket_path`. Removes the socket on shutdown.
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

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        let db = db.clone();
                        let dispatch = dispatch.clone();
                        tokio::spawn(async move {
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

    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

async fn handle_connection<F, Fut>(stream: UnixStream, db: DbHandle, dispatch: F) -> Result<()>
where
    F: Fn(RpcRequest, DbHandle) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = std::result::Result<serde_json::Value, RpcError>> + Send,
{
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => match dispatch(req.clone(), db.clone()).await {
                Ok(value) => RpcResponse {
                    id: req.id,
                    result: Some(value),
                    error: None,
                },
                Err(err) => RpcResponse {
                    id: req.id,
                    result: None,
                    error: Some(err),
                },
            },
            Err(e) => RpcResponse {
                id: 0,
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use tempfile::tempdir;

    fn echo_db() -> DbHandle {
        Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()))
    }

    #[tokio::test]
    async fn server_round_trips_request_and_response() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("test.sock");
        let db = echo_db();
        let (tx, rx) = oneshot::channel();

        // Echo dispatcher: returns the request's params verbatim.
        let dispatch = |req: RpcRequest, _db: DbHandle| async move { Ok(req.params) };

        let socket_clone = socket.clone();
        let server_task = tokio::spawn(async move {
            run(&socket_clone, db, rx, dispatch).await.unwrap();
        });

        // Wait until the socket exists.
        for _ in 0..50 {
            if socket.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let mut stream = UnixStream::connect(&socket).await.unwrap();
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
        assert_eq!(resp.id, 42);
        assert_eq!(resp.result, Some(serde_json::json!({"hi": "there"})));
        assert!(resp.error.is_none());

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_request_yields_invalid_params_error() {
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

        for _ in 0..50 {
            if socket.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let mut stream = UnixStream::connect(&socket).await.unwrap();
        stream.write_all(b"not json\n").await.unwrap();
        stream.shutdown().await.unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: RpcResponse = serde_json::from_str(line.trim()).unwrap();
        let err = resp.error.unwrap();
        assert_eq!(err.code, RpcError::INVALID_PARAMS);

        let _ = tx.send(());
        server_task.await.unwrap();
    }

    // Suppress unused warning until test count is meaningful.
    #[allow(dead_code)]
    static _N: AtomicU32 = AtomicU32::new(0);
}
