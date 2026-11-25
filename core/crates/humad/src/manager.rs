use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use huma_acp::{AcpHarness, HarnessEvent, resolve_driver_command};
use huma_protocol::rpc::{ClientCommand, ResponsePayload};
use huma_protocol::{DriverKind, ProjectId, ProjectSnapshot, PromptInput, ServerEvent};
use serde_json::json;
use tokio::sync::{Mutex, broadcast, mpsc};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::audit::{AuditLog, now_ms, redact_argv};
use crate::state::DaemonState;

struct LiveDriver {
    driver: DriverKind,
    harness: Arc<AcpHarness>,
    turn_started: Option<(String, Instant)>,
}

/// Owns live agent harness processes, pure state, audit trail, and UI event fanout.
pub struct SessionManager {
    state: Arc<Mutex<DaemonState>>,
    drivers: Arc<Mutex<HashMap<ProjectId, LiveDriver>>>,
    events_tx: broadcast::Sender<ServerEvent>,
    audit: Arc<AuditLog>,
}

impl SessionManager {
    pub fn new(audit: Arc<AuditLog>) -> Self {
        let (events_tx, _) = broadcast::channel(512);
        Self {
            state: Arc::new(Mutex::new(DaemonState::default())),
            drivers: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            audit,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.events_tx.subscribe()
    }

    pub async fn handle_command(&self, cmd: ClientCommand) -> Result<ResponsePayload, String> {
        match cmd {
            ClientCommand::Ping => Ok(ResponsePayload::Pong {
                protocol_version: huma_protocol::PROTOCOL_VERSION,
            }),
            ClientCommand::ListProjects => {
                let projects = self.state.lock().await.list_snapshots();
                Ok(ResponsePayload::Projects { projects })
            }
            ClientCommand::OpenProject {
                cwd,
                driver,
                custom_command,
            } => {
                let project = self.open_project(cwd, driver, custom_command).await?;
                Ok(ResponsePayload::Project {
                    project: Box::new(project),
                })
            }
            ClientCommand::SendPrompt { project_id, input } => {
                self.send_prompt(&project_id, input).await?;
                Ok(ResponsePayload::Ack)
            }
            ClientCommand::CancelTurn { project_id } => {
                self.cancel_turn(&project_id).await?;
                Ok(ResponsePayload::Ack)
            }
            ClientCommand::RespondPermission {
                project_id,
                request_id,
                option_id,
            } => {
                self.respond_permission(&project_id, &request_id, option_id.as_deref())
                    .await?;
                Ok(ResponsePayload::Ack)
            }
            ClientCommand::CloseProject { project_id } => {
                self.close_project(&project_id).await?;
                Ok(ResponsePayload::Ack)
            }
            ClientCommand::Subscribe => Ok(ResponsePayload::Ack),
        }
    }

    async fn open_project(
        &self,
        cwd: String,
        driver: DriverKind,
        custom_command: Option<Vec<String>>,
    ) -> Result<ProjectSnapshot, String> {
        let canonical = std::fs::canonicalize(&cwd)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|err| format!("invalid project directory `{cwd}`: {err}"))?;

        let argv = resolve_driver_command(&driver, &canonical, custom_command)
            .map_err(|err| err.to_string())?;
        let project_id = format!("prj_{}", Uuid::new_v4().simple());

        info!(
            project_id = %project_id,
            driver = ?driver,
            cwd = %canonical,
            "spawning agent harness"
        );

        let (harness, event_rx) = AcpHarness::spawn(&argv, &canonical)
            .await
            .map_err(|err| err.to_string())?;
        let init_info = harness.init_info.clone();
        let session_id = init_info.session_id.clone();

        self.audit.record(
            "project.opened",
            Some(&project_id),
            json!({
                "cwd": canonical,
                "driver": driver,
                "session_id": session_id,
            }),
        );
        self.audit.record(
            "driver.spawned",
            Some(&project_id),
            json!({
                "driver": driver,
                "argv": redact_argv(&argv),
            }),
        );

        let (snapshot, opened_ev) = self.state.lock().await.register_project(
            project_id.clone(),
            canonical,
            driver.clone(),
            init_info,
        );

        let harness = Arc::new(harness);
        self.drivers.lock().await.insert(
            project_id.clone(),
            LiveDriver {
                driver: driver.clone(),
                harness,
                turn_started: None,
            },
        );
        let _ = self.events_tx.send(opened_ev);

        self.spawn_event_pump(project_id, driver, event_rx);
        Ok(snapshot)
    }

    async fn send_prompt(&self, project_id: &str, input: PromptInput) -> Result<(), String> {
        let turn_id = format!("trn_{}", Uuid::new_v4().simple());
        let harness = {
            let mut drivers = self.drivers.lock().await;
            let entry = drivers
                .get_mut(project_id)
                .ok_or_else(|| format!("unknown project: {project_id}"))?;
            entry.turn_started = Some((turn_id.clone(), Instant::now()));
            Arc::clone(&entry.harness)
        };

        let begin_events = self.state.lock().await.begin_turn(
            project_id,
            turn_id.clone(),
            input.text.clone(),
            now_ms(),
        );
        for ev in begin_events {
            let _ = self.events_tx.send(ev);
        }

        self.audit.record(
            "prompt.submitted",
            Some(project_id),
            json!({
                "turn_id": turn_id,
                "text_len": input.text.len(),
                "attachment_count": input.attachments.len(),
            }),
        );

        info!(
            project_id = %project_id,
            turn_id = %turn_id,
            attachment_count = input.attachments.len(),
            "submitted prompt turn"
        );

        harness
            .start_prompt(turn_id, &input)
            .map_err(|err| err.to_string())
    }

    async fn cancel_turn(&self, project_id: &str) -> Result<(), String> {
        let harness = {
            let drivers = self.drivers.lock().await;
            let entry = drivers
                .get(project_id)
                .ok_or_else(|| format!("unknown project: {project_id}"))?;
            Arc::clone(&entry.harness)
        };
        harness.cancel_turn().map_err(|err| err.to_string())?;
        self.audit
            .record("turn.cancelled", Some(project_id), json!({}));
        info!(project_id = %project_id, "cancelled active turn");
        Ok(())
    }

    async fn respond_permission(
        &self,
        project_id: &str,
        request_id: &str,
        option_id: Option<&str>,
    ) -> Result<(), String> {
        let harness = {
            let drivers = self.drivers.lock().await;
            let entry = drivers
                .get(project_id)
                .ok_or_else(|| format!("unknown project: {project_id}"))?;
            Arc::clone(&entry.harness)
        };
        harness
            .respond_permission(request_id, option_id)
            .await
            .map_err(|err| err.to_string())?;

        let events = self
            .state
            .lock()
            .await
            .on_permission_resolved(project_id, request_id);
        for ev in events {
            let _ = self.events_tx.send(ev);
        }

        self.audit.record(
            "permission.resolved",
            Some(project_id),
            json!({
                "request_id": request_id,
                "decision": if option_id.is_some() { "selected" } else { "cancelled" },
                "option_id": option_id,
            }),
        );
        Ok(())
    }

    async fn close_project(&self, project_id: &str) -> Result<(), String> {
        let live = self.drivers.lock().await.remove(project_id);
        if let Some(entry) = live {
            entry.harness.shutdown().await;
            self.audit.record(
                "driver.exited",
                Some(project_id),
                json!({ "driver": entry.driver, "reason": "closed" }),
            );
        }
        if let Some(ev) = self.state.lock().await.remove_project(project_id) {
            let _ = self.events_tx.send(ev);
            self.audit
                .record("project.closed", Some(project_id), json!({}));
            info!(project_id = %project_id, "closed project");
            Ok(())
        } else {
            Err(format!("unknown project: {project_id}"))
        }
    }

    pub async fn shutdown_all(&self) {
        let entries: Vec<(ProjectId, LiveDriver)> = self.drivers.lock().await.drain().collect();
        for (project_id, entry) in entries {
            entry.harness.shutdown().await;
            info!(project_id = %project_id, "terminated agent harness during daemon shutdown");
        }
    }

    fn spawn_event_pump(
        &self,
        project_id: ProjectId,
        driver: DriverKind,
        mut event_rx: mpsc::UnboundedReceiver<HarnessEvent>,
    ) {
        let state = Arc::clone(&self.state);
        let drivers = Arc::clone(&self.drivers);
        let events_tx = self.events_tx.clone();
        let audit = Arc::clone(&self.audit);

        tokio::spawn(async move {
            while let Some(ev) = event_rx.recv().await {
                match ev {
                    HarnessEvent::MessageChunk(delta) => {
                        if let Some(out) = state.lock().await.on_message_chunk(&project_id, delta) {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::ThoughtChunk(_) => {
                        // Thought chunks remain hidden from the calm primary canvas by design.
                    }
                    HarnessEvent::ToolActivity { title, detail, .. } => {
                        if let Some(out) =
                            state
                                .lock()
                                .await
                                .on_tool_activity(&project_id, title, detail)
                        {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::SlashCommands(cmds) => {
                        if let Some(out) = state.lock().await.on_slash_commands(&project_id, cmds) {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::TitleUpdated(title) => {
                        if let Some(out) = state.lock().await.on_title_updated(&project_id, title) {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::UsageUpdated {
                        context_used,
                        context_limit,
                    } => {
                        if let Some(out) = state.lock().await.on_usage_updated(
                            &project_id,
                            context_used,
                            context_limit,
                        ) {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::PermissionRequested {
                        request_id,
                        title,
                        detail,
                        options,
                    } => {
                        audit.record(
                            "permission.requested",
                            Some(&project_id),
                            json!({
                                "request_id": request_id,
                                "tool_title": title,
                            }),
                        );
                        let outs = state.lock().await.on_permission_requested(
                            &project_id,
                            request_id,
                            title,
                            detail,
                            options,
                        );
                        for out in outs {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::TurnFinished {
                        turn_id,
                        stop_reason,
                        usage,
                    } => {
                        let elapsed_ms = drivers
                            .lock()
                            .await
                            .get_mut(&project_id)
                            .and_then(|d| d.turn_started.take())
                            .map(|(_, started)| started.elapsed().as_millis() as u64)
                            .unwrap_or(0);

                        audit.record(
                            "turn.completed",
                            Some(&project_id),
                            json!({
                                "turn_id": turn_id,
                                "stop_reason": stop_reason,
                                "usage": usage,
                                "elapsed_ms": elapsed_ms,
                            }),
                        );
                        info!(
                            project_id = %project_id,
                            driver = ?driver,
                            turn_id = %turn_id,
                            elapsed_ms,
                            "turn completed"
                        );
                        let outs = state.lock().await.finish_turn(
                            &project_id,
                            turn_id,
                            stop_reason,
                            usage,
                            now_ms(),
                        );
                        for out in outs {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::TurnFailed {
                        turn_id,
                        error: err_msg,
                    } => {
                        error!(project_id = %project_id, turn_id = %turn_id, error = %err_msg, "turn failed");
                        audit.record(
                            "turn.failed",
                            Some(&project_id),
                            json!({ "turn_id": turn_id, "error": err_msg }),
                        );
                        let outs = state.lock().await.fail_project(&project_id, err_msg);
                        for out in outs {
                            let _ = events_tx.send(out);
                        }
                    }
                    HarnessEvent::ProcessExited { reason } => {
                        // Ignore normal exits after explicit close.
                        if drivers.lock().await.remove(&project_id).is_some() {
                            warn!(project_id = %project_id, driver = ?driver, reason = %reason, "agent process exited unexpectedly");
                            audit.record(
                                "driver.exited",
                                Some(&project_id),
                                json!({ "driver": driver, "reason": reason }),
                            );
                            let outs = state.lock().await.fail_project(&project_id, reason);
                            for out in outs {
                                let _ = events_tx.send(out);
                            }
                        }
                        break;
                    }
                }
            }
        });
    }
}
