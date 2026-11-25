use serde::{Deserialize, Serialize};

pub type ProjectId = String;

/// Supported agent harness drivers (BYOS — drives locally installed CLIs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverKind {
    Agy,
    Opencode,
    ClaudeCode,
    CustomAcp,
    #[serde(other)]
    Unknown,
}

impl DriverKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Agy => "Antigravity (agy)",
            Self::Opencode => "OpenCode",
            Self::ClaudeCode => "Claude Code",
            Self::CustomAcp => "Custom ACP",
            Self::Unknown => "Unknown",
        }
    }
}

/// High-level calm status for a project (drives quiet ambient indicators in UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Idle,
    Working,
    NeedsAttention,
    Ready,
    Error,
}

/// Single-line noise-canceled activity state folded above the calm summary view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityState {
    /// Human-readable single-line phase label (e.g. "正在读取 3 个文件" / "运行测试中").
    pub phase: String,
    /// Number of active/completed tool steps in the current turn.
    pub tool_count: u32,
    /// Optional concise detail about the latest tool call (kept hidden unless expanded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Slash command advertised dynamically by the active agent via ACP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub option_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub project_id: ProjectId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub thought_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
}

/// Multimodal attachment submitted alongside a user prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
    /// Local image file path (encoded to base64 by huma-core when sent over ACP).
    ImagePath {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    /// Inline base64 image payload (e.g. direct macOS Cmd+V clipboard screenshot).
    ImageBase64 { mime_type: String, data: String },
    /// Local file reference embedded as context.
    FilePath { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptInput {
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub turn_id: String,
    pub user_prompt: String,
    pub assistant_markdown: String,
    #[serde(default)]
    pub tool_events: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Concise 1-line semantic summary for zero-cold-start overview cards.
    pub headline: String,
    /// Full clean markdown from the latest assistant turn.
    pub latest_markdown: String,
    pub updated_at_ms: u64,
}

/// Full client-consumable snapshot of a managed project workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub project_id: ProjectId,
    pub repo_name: String,
    pub cwd: String,
    pub driver: DriverKind,
    /// Automatically maintained semantic title (never requires manual naming).
    pub semantic_title: String,
    pub status: ProjectStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<ActivityState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_permission: Option<PermissionRequest>,
    #[serde(default)]
    pub slash_commands: Vec<SlashCommand>,
    #[serde(default)]
    pub available_models: Vec<AvailableModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<TurnSnapshot>,
    #[serde(default)]
    pub recent_turns: Vec<TurnSnapshot>,
    #[serde(default)]
    pub usage: TokenUsage,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}
