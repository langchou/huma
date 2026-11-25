pub mod acp_client;
pub mod acp_wire;

use huma_protocol::{AvailableModel, DriverKind, PermissionOption, SlashCommand, TokenUsage};
use thiserror::Error;

pub use acp_client::AcpHarness;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("invalid driver command: {0}")]
    InvalidCommand(String),
    #[error("failed to spawn agent `{program}`: {source}")]
    SpawnFailed {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read attachment: {0}")]
    AttachmentIo(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ACP RPC error ({code}): {message}")]
    RpcError { code: i64, message: String },
    #[error("ACP request `{method}` timed out")]
    Timeout { method: String },
    #[error("ACP protocol error: {0}")]
    Protocol(String),
    #[error("ACP agent connection closed")]
    Closed,
}

#[derive(Debug, Clone)]
pub struct SessionInitInfo {
    pub session_id: String,
    pub agent_name: String,
    pub agent_version: Option<String>,
    pub supports_images: bool,
    pub available_models: Vec<AvailableModel>,
}

/// Normalized events emitted by any driver in `huma-acp`.
#[derive(Debug, Clone)]
pub enum HarnessEvent {
    MessageChunk(String),
    ThoughtChunk(String),
    ToolActivity {
        title: String,
        status: Option<String>,
        detail: Option<String>,
    },
    SlashCommands(Vec<SlashCommand>),
    TitleUpdated(String),
    UsageUpdated {
        context_used: Option<u64>,
        context_limit: Option<u64>,
    },
    PermissionRequested {
        request_id: String,
        title: String,
        detail: Option<String>,
        options: Vec<PermissionOption>,
    },
    TurnFinished {
        turn_id: String,
        stop_reason: Option<String>,
        usage: TokenUsage,
    },
    TurnFailed {
        turn_id: String,
        error: String,
    },
    ProcessExited {
        reason: String,
    },
}

/// Resolves a `DriverKind` into its default local CLI invocation (BYOS).
pub fn resolve_driver_command(
    driver: &DriverKind,
    cwd: &str,
    custom_command: Option<Vec<String>>,
) -> Result<Vec<String>, HarnessError> {
    if let Some(cmd) = custom_command {
        if cmd.is_empty() {
            return Err(HarnessError::InvalidCommand(
                "custom_command cannot be empty".into(),
            ));
        }
        return Ok(cmd);
    }

    match driver {
        DriverKind::Agy => Ok(vec!["agy".into(), "acp".into(), "--stdio".into()]),
        DriverKind::Opencode => Ok(vec![
            "opencode".into(),
            "acp".into(),
            "--cwd".into(),
            cwd.to_string(),
        ]),
        DriverKind::ClaudeCode => Err(HarnessError::InvalidCommand(
            "claude_code native stream-json driver is scheduled next; pass custom_command for an ACP bridge or use agy/opencode".into(),
        )),
        DriverKind::CustomAcp => Err(HarnessError::InvalidCommand(
            "custom_acp requires custom_command".into(),
        )),
        DriverKind::Unknown => Err(HarnessError::InvalidCommand(
            "unknown driver kind".into(),
        )),
    }
}
