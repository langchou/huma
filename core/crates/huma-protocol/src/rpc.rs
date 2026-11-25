use serde::{Deserialize, Serialize};

use crate::types::{
    ActivityState, DriverKind, PermissionRequest, ProjectId, ProjectSnapshot, ProjectStatus,
    PromptInput, SessionSummary, SlashCommand, TokenUsage,
};

pub const PROTOCOL_VERSION: u32 = 1;

/// Requests sent from any UI client (huma-macos, CLI, future clients) to `humad`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRequest {
    pub id: String,
    #[serde(flatten)]
    pub command: ClientCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ClientCommand {
    Ping,
    /// Fetch all live projects and snapshots (0ms warm boot for native UI).
    ListProjects,
    /// Open or attach a repository directory with an agent driver.
    OpenProject {
        cwd: String,
        #[serde(default = "default_driver")]
        driver: DriverKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_command: Option<Vec<String>>,
    },
    /// Send a prompt (text + optional images/files) to a project's live agent session.
    SendPrompt {
        project_id: ProjectId,
        input: PromptInput,
    },
    /// Cancel an active streaming turn cleanly.
    CancelTurn {
        project_id: ProjectId,
    },
    /// Respond to a pending agent permission / approval request.
    RespondPermission {
        project_id: ProjectId,
        request_id: String,
        option_id: Option<String>,
    },
    /// Close a project and terminate its underlying agent harness process.
    CloseProject {
        project_id: ProjectId,
    },
    /// Subscribe this socket connection to live ServerEvent push frames.
    Subscribe,
}

fn default_driver() -> DriverKind {
    DriverKind::Agy
}

/// Top-level NDJSON line emitted by `humad` over the Unix Domain Socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEnvelope {
    Response(ServerResponse),
    Event(ServerEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResponse {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ResponsePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsePayload {
    Pong { protocol_version: u32 },
    Projects { projects: Vec<ProjectSnapshot> },
    Project { project: Box<ProjectSnapshot> },
    Ack,
}

/// Real-time structured events pushed to subscribed UI clients.
/// Designed so native clients (e.g. huma-macos Smooth Ink Pacer) receive clean,
/// low-noise deltas without raw terminal escape codes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ServerEvent {
    ProjectOpened {
        project: Box<ProjectSnapshot>,
    },
    ProjectClosed {
        project_id: ProjectId,
    },
    ProjectStatusChanged {
        project_id: ProjectId,
        status: ProjectStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_error: Option<String>,
    },
    SemanticTitleUpdated {
        project_id: ProjectId,
        semantic_title: String,
    },
    SlashCommandsUpdated {
        project_id: ProjectId,
        commands: Vec<SlashCommand>,
    },
    ActivityUpdated {
        project_id: ProjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<ActivityState>,
    },
    /// Incremental assistant text chunk fed directly into the UI's Smooth Ink Pacer.
    MessageDelta {
        project_id: ProjectId,
        turn_id: String,
        delta: String,
    },
    TurnCompleted {
        project_id: ProjectId,
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
        summary: SessionSummary,
        usage: TokenUsage,
    },
    UsageUpdated {
        project_id: ProjectId,
        usage: TokenUsage,
    },
    PermissionRequested {
        request: PermissionRequest,
    },
    PermissionResolved {
        project_id: ProjectId,
        request_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_client_open_project_request() {
        let req = ClientRequest {
            id: "req-1".into(),
            command: ClientCommand::OpenProject {
                cwd: "/tmp/demo".into(),
                driver: DriverKind::Agy,
                custom_command: None,
            },
        };
        let encoded = serde_json::to_string(&req).expect("serialize request");
        let decoded: ClientRequest = serde_json::from_str(&encoded).expect("deserialize request");
        assert_eq!(decoded.id, "req-1");
        match decoded.command {
            ClientCommand::OpenProject { cwd, driver, .. } => {
                assert_eq!(cwd, "/tmp/demo");
                assert_eq!(driver, DriverKind::Agy);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn roundtrip_message_delta_envelope() {
        let env = ServerEnvelope::Event(ServerEvent::MessageDelta {
            project_id: "p1".into(),
            turn_id: "t1".into(),
            delta: "hello".into(),
        });
        let encoded = serde_json::to_string(&env).expect("serialize envelope");
        let decoded: ServerEnvelope = serde_json::from_str(&encoded).expect("deserialize envelope");
        match decoded {
            ServerEnvelope::Event(ServerEvent::MessageDelta {
                project_id,
                turn_id,
                delta,
            }) => {
                assert_eq!(project_id, "p1");
                assert_eq!(turn_id, "t1");
                assert_eq!(delta, "hello");
            }
            other => panic!("unexpected envelope: {other:?}"),
        }
    }
}
