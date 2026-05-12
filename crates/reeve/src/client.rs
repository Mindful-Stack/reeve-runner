//! IPC client: sends one RPC request over a Unix socket, returns the response.

use anyhow::{Context, Result};
use reeve_protocol::{RpcError, RpcRequest, RpcResponse};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Connect to the daemon socket and perform a single request/response cycle.
pub async fn call(
    socket_path: &Path,
    method: &str,
    params: serde_json::Value,
) -> Result<std::result::Result<serde_json::Value, RpcError>> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect to {}", socket_path.display()))?;

    let req = RpcRequest {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        method: method.to_owned(),
        params,
    };
    let mut bytes = serde_json::to_vec(&req)?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    stream.shutdown().await.ok();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        anyhow::bail!("daemon closed connection without responding");
    }
    let resp: RpcResponse = serde_json::from_str(line.trim())?;
    if let Some(err) = resp.error {
        return Ok(Err(err));
    }
    Ok(Ok(resp.result.unwrap_or(serde_json::Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::net::UnixListener;

    /// Starts a tiny echo server that replies with the same `params` field.
    async fn start_echo(socket_path: &Path) {
        let listener = UnixListener::bind(socket_path).unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let req: RpcRequest = serde_json::from_str(&line).unwrap();
                    let resp = RpcResponse {
                        id: Some(req.id),
                        result: Some(req.params),
                        error: None,
                    };
                    let mut bytes = serde_json::to_vec(&resp).unwrap();
                    bytes.push(b'\n');
                    write.write_all(&bytes).await.unwrap();
                    break;
                }
            }
        });
    }

    #[tokio::test]
    async fn client_round_trips_to_echo_server() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("echo.sock");
        start_echo(&socket).await;
        // Give the listener a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = call(&socket, "ping", serde_json::json!({"hi": 1}))
            .await
            .unwrap();
        let value = result.unwrap();
        assert_eq!(value, serde_json::json!({"hi": 1}));
    }

    #[tokio::test]
    async fn client_propagates_rpc_error_response() {
        let dir = tempdir().unwrap();
        let socket = dir.path().join("err.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                if let Ok(Some(_)) = lines.next_line().await {
                    let resp = RpcResponse {
                        id: Some(1),
                        result: None,
                        error: Some(RpcError {
                            code: 404,
                            message: "nope".into(),
                        }),
                    };
                    let mut bytes = serde_json::to_vec(&resp).unwrap();
                    bytes.push(b'\n');
                    write.write_all(&bytes).await.unwrap();
                }
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = call(&socket, "anything", serde_json::json!({}))
            .await
            .unwrap();
        let err = result.unwrap_err();
        assert_eq!(err.code, 404);
        assert_eq!(err.message, "nope");
    }
}
