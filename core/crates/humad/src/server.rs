use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use huma_protocol::rpc::ClientCommand;
use huma_protocol::{ClientRequest, ServerEnvelope, ServerResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

use crate::manager::SessionManager;

/// Resolves the local state directory (`$HUMA_STATE_DIR` or `~/.local/state/huma`).
pub fn resolve_state_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("HUMA_STATE_DIR")
        && !custom.trim().is_empty()
    {
        return PathBuf::from(custom);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/state/huma");
    }
    std::env::temp_dir().join("huma-state")
}

/// Resolves the Unix Domain Socket path (`$HUMA_SOCKET_PATH` or `<state_dir>/humad.sock`).
pub fn resolve_socket_path() -> PathBuf {
    if let Ok(custom) = std::env::var("HUMA_SOCKET_PATH")
        && !custom.trim().is_empty()
    {
        return PathBuf::from(custom);
    }
    resolve_state_dir().join("humad.sock")
}

pub async fn run_unix_server(
    socket_path: &Path,
    manager: Arc<SessionManager>,
) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Detect and remove stale socket files cleanly.
    if socket_path.exists() {
        match UnixStream::connect(socket_path).await {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("humad is already listening on {}", socket_path.display()),
                ));
            }
            Err(_) => {
                warn!(path = %socket_path.display(), "removing stale humad socket");
                let _ = fs::remove_file(socket_path);
            }
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600));
    }

    info!(socket = %socket_path.display(), "humad listening");

    let shutdown_signal = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    tokio::pin!(shutdown_signal);

    loop {
        tokio::select! {
            _ = &mut shutdown_signal => {
                info!("humad received shutdown signal");
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let mgr = Arc::clone(&manager);
                        tokio::spawn(async move {
                            handle_connection(stream, mgr).await;
                        });
                    }
                    Err(err) => {
                        warn!(error = %err, "socket accept failed");
                    }
                }
            }
        }
    }

    manager.shutdown_all().await;
    let _ = fs::remove_file(socket_path);
    Ok(())
}

async fn handle_connection(stream: UnixStream, manager: Arc<SessionManager>) {
    let (read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    let writer_task = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
            if write_half.flush().await.is_err() {
                break;
            }
        }
    });

    let mut sub_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut lines = BufReader::new(read_half).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req = match serde_json::from_str::<ClientRequest>(trimmed) {
            Ok(req) => req,
            Err(err) => {
                let resp = ServerEnvelope::Response(ServerResponse {
                    id: String::new(),
                    ok: false,
                    result: None,
                    error: Some(format!("invalid request json: {err}")),
                });
                if let Ok(encoded) = serde_json::to_string(&resp) {
                    let _ = out_tx.send(encoded);
                }
                continue;
            }
        };

        let is_subscribe = matches!(req.command, ClientCommand::Subscribe);
        debug!(request_id = %req.id, "handling client request");

        let resp = match manager.handle_command(req.command).await {
            Ok(payload) => ServerResponse {
                id: req.id,
                ok: true,
                result: Some(payload),
                error: None,
            },
            Err(err) => ServerResponse {
                id: req.id,
                ok: false,
                result: None,
                error: Some(err),
            },
        };

        if let Ok(encoded) = serde_json::to_string(&ServerEnvelope::Response(resp))
            && out_tx.send(encoded).is_err()
        {
            break;
        }

        if is_subscribe && sub_task.is_none() {
            let mut rx = manager.subscribe();
            let ev_tx = out_tx.clone();
            sub_task = Some(tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if let Ok(encoded) =
                                serde_json::to_string(&ServerEnvelope::Event(event))
                                && ev_tx.send(encoded).is_err()
                            {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(skipped, "subscribed client lagged behind event stream");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }));
        }
    }

    if let Some(handle) = sub_task {
        handle.abort();
    }
    drop(out_tx);
    let _ = writer_task.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditLog;
    use huma_protocol::rpc::ResponsePayload;

    #[tokio::test]
    async fn unix_socket_ping_and_list_projects_roundtrip() {
        let short_id = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let dir = PathBuf::from(format!("/tmp/huma-t-{short_id}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        let sock = dir.join("s.sock");
        let audit = Arc::new(AuditLog::new(dir.join("audit.jsonl")));
        let mgr = Arc::new(SessionManager::new(audit));

        let listener = UnixListener::bind(&sock).expect("bind unix socket");
        let server_mgr = Arc::clone(&mgr);
        let server = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                handle_connection(stream, server_mgr).await;
            }
        });

        let stream = UnixStream::connect(&sock)
            .await
            .expect("connect unix socket");
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        let ping = ClientRequest {
            id: "1".into(),
            command: ClientCommand::Ping,
        };
        let line = serde_json::to_string(&ping).expect("serialize ping");
        write_half
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("write ping");

        let reply = reader
            .next_line()
            .await
            .expect("read reply")
            .expect("non-eof");
        let env: ServerEnvelope = serde_json::from_str(&reply).expect("parse envelope");
        match env {
            ServerEnvelope::Response(ServerResponse {
                ok: true,
                result: Some(ResponsePayload::Pong { protocol_version }),
                ..
            }) => assert_eq!(protocol_version, huma_protocol::PROTOCOL_VERSION),
            other => panic!("unexpected ping reply: {other:?}"),
        }

        drop(write_half);
        let _ = server.await;
        let _ = fs::remove_dir_all(dir);
    }
}
