use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use huma_protocol::PromptInput;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::acp_wire::{
    IncomingRpc, InitCapabilities, ParsedSessionUpdate, build_prompt_blocks, make_error_response,
    make_notification, make_permission_outcome, make_request, make_response, parse_incoming_rpc,
    parse_init_result, parse_new_session_result, parse_permission_request, parse_prompt_result,
    parse_session_update,
};
use crate::{HarnessError, HarnessEvent, SessionInitInfo};

const INIT_TIMEOUT: Duration = Duration::from_secs(20);
const NEW_SESSION_TIMEOUT: Duration = Duration::from_secs(25);

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, HarnessError>>>>>;
type PermissionRpcMap = Arc<Mutex<HashMap<String, Value>>>;

/// Live bidirectional ACP harness owning one child agent process.
pub struct AcpHarness {
    pub init_info: SessionInitInfo,
    write_tx: mpsc::UnboundedSender<String>,
    pending_rpc: PendingMap,
    pending_permissions: PermissionRpcMap,
    next_id: AtomicU64,
    event_tx: mpsc::UnboundedSender<HarnessEvent>,
    child: Arc<Mutex<Child>>,
}

impl AcpHarness {
    pub async fn spawn(
        command: &[String],
        cwd: &str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<HarnessEvent>), HarnessError> {
        let Some((program, args)) = command.split_first() else {
            return Err(HarnessError::InvalidCommand(
                "driver command cannot be empty".into(),
            ));
        };

        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|err| HarnessError::SpawnFailed {
            program: program.clone(),
            source: err,
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HarnessError::Protocol("child stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HarnessError::Protocol("child stdout unavailable".into()))?;
        let stderr = child.stderr.take();

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<HarnessEvent>();
        let pending_rpc: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let pending_permissions: PermissionRpcMap = Arc::new(Mutex::new(HashMap::new()));

        // Writer task: serializes all outgoing JSON-RPC lines cleanly.
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = write_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        // Drain stderr so child never blocks on a full pipe.
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(target: "huma_acp::stderr", "{}", line);
                }
            });
        }

        // Reader task: demuxes responses, notifications, and agent->client requests.
        let reader_pending = Arc::clone(&pending_rpc);
        let reader_perms = Arc::clone(&pending_permissions);
        let reader_events = event_tx.clone();
        let reader_write = write_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match parse_incoming_rpc(trimmed) {
                            Ok(IncomingRpc::Response { id, result, error }) => {
                                if let Some(num_id) = id.as_u64() {
                                    let tx = reader_pending.lock().await.remove(&num_id);
                                    if let Some(tx) = tx {
                                        let outcome = match (result, error) {
                                            (_, Some(err)) => Err(HarnessError::RpcError {
                                                code: err.code,
                                                message: err.message,
                                            }),
                                            (Some(val), None) => Ok(val),
                                            (None, None) => Ok(Value::Null),
                                        };
                                        let _ = tx.send(outcome);
                                    }
                                }
                            }
                            Ok(IncomingRpc::Notification { method, params }) => {
                                if method == "session/update" {
                                    forward_session_update(&params, &reader_events);
                                }
                            }
                            Ok(IncomingRpc::Request { id, method, params }) => {
                                if method == "session/request_permission" {
                                    let parsed = parse_permission_request(&params);
                                    let req_id = Uuid::new_v4().to_string();
                                    reader_perms.lock().await.insert(req_id.clone(), id);
                                    let _ = reader_events.send(HarnessEvent::PermissionRequested {
                                        request_id: req_id,
                                        title: parsed.title,
                                        detail: parsed.detail,
                                        options: parsed.options,
                                    });
                                } else {
                                    let err = make_error_response(
                                        id,
                                        -32601,
                                        &format!("method not supported by huma client: {method}"),
                                    );
                                    if let Ok(encoded) = serde_json::to_string(&err) {
                                        let _ = reader_write.send(encoded);
                                    }
                                }
                            }
                            Err(err) => {
                                warn!("ignoring non-JSON-RPC line from ACP server: {err}");
                            }
                        }
                    }
                    Ok(None) => {
                        let _ = reader_events.send(HarnessEvent::ProcessExited {
                            reason: "ACP agent stdout closed".into(),
                        });
                        break;
                    }
                    Err(err) => {
                        let _ = reader_events.send(HarnessEvent::ProcessExited {
                            reason: format!("ACP stdout read error: {err}"),
                        });
                        break;
                    }
                }
            }

            // Fail any in-flight RPC callers cleanly.
            let mut pending = reader_pending.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(HarnessError::Closed));
            }
        });

        let harness = Self {
            init_info: SessionInitInfo {
                session_id: String::new(),
                agent_name: String::new(),
                agent_version: None,
                supports_images: false,
                available_models: Vec::new(),
            },
            write_tx,
            pending_rpc,
            pending_permissions,
            next_id: AtomicU64::new(1),
            event_tx,
            child: Arc::new(Mutex::new(child)),
        };

        let init_caps = harness.initialize().await?;
        let new_session = harness.create_session(cwd).await?;

        let mut harness = harness;
        harness.init_info = SessionInitInfo {
            session_id: new_session.session_id,
            agent_name: init_caps.agent_name,
            agent_version: init_caps.agent_version,
            supports_images: init_caps.supports_images,
            available_models: new_session.available_models,
        };

        Ok((harness, event_rx))
    }

    async fn initialize(&self) -> Result<InitCapabilities, HarnessError> {
        let params = json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": {
                    "readTextFile": false,
                    "writeTextFile": false,
                },
                "terminal": false,
            },
            "clientInfo": {
                "name": "huma",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });
        let val = self
            .call_with_timeout("initialize", params, INIT_TIMEOUT)
            .await?;
        Ok(parse_init_result(&val))
    }

    async fn create_session(
        &self,
        cwd: &str,
    ) -> Result<crate::acp_wire::NewSessionInfo, HarnessError> {
        let params = json!({
            "cwd": cwd,
            "mcpServers": [],
        });
        let val = self
            .call_with_timeout("session/new", params, NEW_SESSION_TIMEOUT)
            .await?;
        parse_new_session_result(&val)
            .ok_or_else(|| HarnessError::Protocol("session/new response missing sessionId".into()))
    }

    /// Starts a `session/prompt` turn asynchronously. Streaming deltas arrive on the event channel,
    /// followed by `HarnessEvent::TurnFinished` when the turn resolves.
    pub fn start_prompt(&self, turn_id: String, input: &PromptInput) -> Result<(), HarnessError> {
        let blocks = build_prompt_blocks(input).map_err(HarnessError::AttachmentIo)?;
        if blocks.is_empty() {
            return Err(HarnessError::InvalidCommand(
                "prompt text and attachments cannot both be empty".into(),
            ));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = make_request(
            id,
            "session/prompt",
            json!({
                "sessionId": self.init_info.session_id,
                "prompt": blocks,
            }),
        );
        let encoded = serde_json::to_string(&payload)?;
        let (tx, rx) = oneshot::channel();

        let pending_rpc = Arc::clone(&self.pending_rpc);
        let write_tx = self.write_tx.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            pending_rpc.lock().await.insert(id, tx);
            if write_tx.send(encoded).is_err() {
                pending_rpc.lock().await.remove(&id);
                let _ = event_tx.send(HarnessEvent::TurnFailed {
                    turn_id,
                    error: "ACP write pipe closed".into(),
                });
                return;
            }
            match rx.await {
                Ok(Ok(val)) => {
                    let (stop_reason, usage) = parse_prompt_result(&val);
                    let _ = event_tx.send(HarnessEvent::TurnFinished {
                        turn_id,
                        stop_reason,
                        usage,
                    });
                }
                Ok(Err(err)) => {
                    let _ = event_tx.send(HarnessEvent::TurnFailed {
                        turn_id,
                        error: err.to_string(),
                    });
                }
                Err(_) => {
                    let _ = event_tx.send(HarnessEvent::TurnFailed {
                        turn_id,
                        error: "ACP turn channel dropped".into(),
                    });
                }
            }
        });

        Ok(())
    }

    pub fn cancel_turn(&self) -> Result<(), HarnessError> {
        let notif = make_notification(
            "session/cancel",
            json!({
                "sessionId": self.init_info.session_id,
            }),
        );
        let encoded = serde_json::to_string(&notif)?;
        self.write_tx
            .send(encoded)
            .map_err(|_| HarnessError::Closed)
    }

    pub async fn respond_permission(
        &self,
        request_id: &str,
        option_id: Option<&str>,
    ) -> Result<(), HarnessError> {
        let rpc_id = self
            .pending_permissions
            .lock()
            .await
            .remove(request_id)
            .ok_or_else(|| {
                HarnessError::Protocol(format!("unknown pending permission request: {request_id}"))
            })?;
        let resp = make_response(rpc_id, make_permission_outcome(option_id));
        let encoded = serde_json::to_string(&resp)?;
        self.write_tx
            .send(encoded)
            .map_err(|_| HarnessError::Closed)
    }

    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }

    async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, HarnessError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = make_request(id, method, params);
        let encoded = serde_json::to_string(&payload)?;

        let (tx, rx) = oneshot::channel();
        self.pending_rpc.lock().await.insert(id, tx);

        if self.write_tx.send(encoded).is_err() {
            self.pending_rpc.lock().await.remove(&id);
            return Err(HarnessError::Closed);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(HarnessError::Closed),
            Err(_) => {
                self.pending_rpc.lock().await.remove(&id);
                Err(HarnessError::Timeout {
                    method: method.to_string(),
                })
            }
        }
    }
}

fn forward_session_update(params: &Value, event_tx: &mpsc::UnboundedSender<HarnessEvent>) {
    match parse_session_update(params) {
        ParsedSessionUpdate::MessageChunk(text) => {
            let _ = event_tx.send(HarnessEvent::MessageChunk(text));
        }
        ParsedSessionUpdate::ThoughtChunk(text) => {
            let _ = event_tx.send(HarnessEvent::ThoughtChunk(text));
        }
        ParsedSessionUpdate::ToolCall {
            title,
            status,
            detail,
        } => {
            let _ = event_tx.send(HarnessEvent::ToolActivity {
                title,
                status,
                detail,
            });
        }
        ParsedSessionUpdate::AvailableCommands(commands) => {
            let _ = event_tx.send(HarnessEvent::SlashCommands(commands));
        }
        ParsedSessionUpdate::TitleUpdated(title) => {
            let _ = event_tx.send(HarnessEvent::TitleUpdated(title));
        }
        ParsedSessionUpdate::UsageUpdated {
            context_used,
            context_limit,
        } => {
            let _ = event_tx.send(HarnessEvent::UsageUpdated {
                context_used,
                context_limit,
            });
        }
        ParsedSessionUpdate::PlanUpdated { summary } => {
            let _ = event_tx.send(HarnessEvent::ToolActivity {
                title: summary,
                status: Some("in_progress".into()),
                detail: None,
            });
        }
        ParsedSessionUpdate::Ignored => {}
    }
}
