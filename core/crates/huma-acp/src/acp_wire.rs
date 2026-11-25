use std::path::Path;

use base64::Engine as _;
use huma_protocol::{
    Attachment, AvailableModel, PermissionOption, PromptInput, SlashCommand, TokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum IncomingRpc {
    /// Response to an outgoing client -> agent request.
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<JsonRpcError>,
    },
    /// Incoming agent -> client request that expects a response (e.g. `session/request_permission`).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// One-way agent -> client notification (e.g. `session/update`).
    Notification { method: String, params: Value },
}

pub fn parse_incoming_rpc(line: &str) -> Result<IncomingRpc, serde_json::Error> {
    let val: Value = serde_json::from_str(line)?;
    let has_method = val.get("method").and_then(Value::as_str).is_some();
    let id = val.get("id").cloned();

    if has_method {
        let method = val
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = val.get("params").cloned().unwrap_or(Value::Null);
        if let Some(req_id) = id {
            Ok(IncomingRpc::Request {
                id: req_id,
                method,
                params,
            })
        } else {
            Ok(IncomingRpc::Notification { method, params })
        }
    } else {
        let id = id.unwrap_or(Value::Null);
        let result = val.get("result").cloned();
        let error = val
            .get("error")
            .cloned()
            .and_then(|e| serde_json::from_value::<JsonRpcError>(e).ok());
        Ok(IncomingRpc::Response { id, result, error })
    }
}

pub fn make_request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

pub fn make_notification(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

pub fn make_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

pub fn make_error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitCapabilities {
    pub agent_name: String,
    pub agent_version: Option<String>,
    pub supports_images: bool,
    pub supports_load_session: bool,
}

pub fn parse_init_result(result: &Value) -> InitCapabilities {
    let agent_name = result
        .pointer("/agentInfo/name")
        .and_then(Value::as_str)
        .unwrap_or("ACP Agent")
        .to_string();
    let agent_version = result
        .pointer("/agentInfo/version")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let supports_images = result
        .pointer("/agentCapabilities/promptCapabilities/image")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let supports_load_session = result
        .pointer("/agentCapabilities/loadSession")
        .and_then(Value::as_bool)
        .or_else(|| {
            result
                .pointer("/agentCapabilities/load_session")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);

    InitCapabilities {
        agent_name,
        agent_version,
        supports_images,
        supports_load_session,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionInfo {
    pub session_id: String,
    pub available_models: Vec<AvailableModel>,
}

pub fn parse_new_session_result(result: &Value) -> Option<NewSessionInfo> {
    let session_id = result.get("sessionId").and_then(Value::as_str)?.to_string();

    let available_models = result
        .pointer("/models/availableModels")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("modelId").and_then(Value::as_str)?.to_string();
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    Some(AvailableModel { id, name })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(NewSessionInfo {
        session_id,
        available_models,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSessionUpdate {
    MessageChunk(String),
    ThoughtChunk(String),
    ToolCall {
        title: String,
        status: Option<String>,
        detail: Option<String>,
    },
    AvailableCommands(Vec<SlashCommand>),
    TitleUpdated(String),
    UsageUpdated {
        context_used: Option<u64>,
        context_limit: Option<u64>,
    },
    PlanUpdated {
        summary: String,
    },
    Ignored,
}

pub fn parse_session_update(params: &Value) -> ParsedSessionUpdate {
    let Some(update) = params.get("update") else {
        return ParsedSessionUpdate::Ignored;
    };
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match kind {
        "agent_message_chunk" => {
            if let Some(text) = extract_content_text(update.get("content")) {
                ParsedSessionUpdate::MessageChunk(text)
            } else {
                ParsedSessionUpdate::Ignored
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = extract_content_text(update.get("content")) {
                ParsedSessionUpdate::ThoughtChunk(text)
            } else {
                ParsedSessionUpdate::Ignored
            }
        }
        "tool_call" | "tool_call_update" => {
            let title = update
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| update.get("kind").and_then(Value::as_str))
                .or_else(|| update.get("toolCallId").and_then(Value::as_str))
                .unwrap_or("执行工具")
                .to_string();
            let status = update
                .get("status")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let detail = update
                .pointer("/rawInput/command")
                .and_then(Value::as_str)
                .or_else(|| update.pointer("/rawInput/path").and_then(Value::as_str))
                .map(ToString::to_string);
            ParsedSessionUpdate::ToolCall {
                title,
                status,
                detail,
            }
        }
        "available_commands_update" => {
            let commands = update
                .get("availableCommands")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|cmd| {
                            let name = cmd.get("name").and_then(Value::as_str)?.to_string();
                            let description = cmd
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            Some(SlashCommand { name, description })
                        })
                        .collect()
                })
                .unwrap_or_default();
            ParsedSessionUpdate::AvailableCommands(commands)
        }
        "session_info_update" => {
            if let Some(title) = update.get("title").and_then(Value::as_str) {
                let trimmed = title.trim();
                if !trimmed.is_empty() {
                    return ParsedSessionUpdate::TitleUpdated(trimmed.to_string());
                }
            }
            ParsedSessionUpdate::Ignored
        }
        "usage_update" => {
            let context_used = update.get("used").and_then(Value::as_u64);
            let context_limit = update.get("size").and_then(Value::as_u64);
            ParsedSessionUpdate::UsageUpdated {
                context_used,
                context_limit,
            }
        }
        "plan" => {
            let summary = update
                .get("entries")
                .and_then(Value::as_array)
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|e| e.get("status").and_then(Value::as_str) == Some("in_progress"))
                        .or_else(|| entries.first())
                        .and_then(|e| e.get("content").and_then(Value::as_str))
                })
                .unwrap_or("规划步骤中")
                .to_string();
            ParsedSessionUpdate::PlanUpdated { summary }
        }
        _ => ParsedSessionUpdate::Ignored,
    }
}

fn extract_content_text(content: Option<&Value>) -> Option<String> {
    let c = content?;
    if let Some(text) = c.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(arr) = c.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

pub fn parse_prompt_result(result: &Value) -> (Option<String>, TokenUsage) {
    let stop_reason = result
        .get("stopReason")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let usage = if let Some(u) = result.get("usage") {
        TokenUsage {
            input_tokens: u.get("inputTokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: u.get("outputTokens").and_then(Value::as_u64).unwrap_or(0),
            thought_tokens: u.get("thoughtTokens").and_then(Value::as_u64).unwrap_or(0),
            total_tokens: u.get("totalTokens").and_then(Value::as_u64).unwrap_or(0),
            context_used: None,
            context_limit: None,
        }
    } else {
        TokenUsage::default()
    };
    (stop_reason, usage)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPermissionRequest {
    pub title: String,
    pub detail: Option<String>,
    pub options: Vec<PermissionOption>,
}

pub fn parse_permission_request(params: &Value) -> ParsedPermissionRequest {
    let title = params
        .pointer("/toolCall/title")
        .and_then(Value::as_str)
        .or_else(|| params.get("title").and_then(Value::as_str))
        .unwrap_or("Agent 请求执行操作")
        .to_string();
    let detail = params
        .pointer("/toolCall/rawInput/command")
        .and_then(Value::as_str)
        .or_else(|| {
            params
                .pointer("/toolCall/rawInput/path")
                .and_then(Value::as_str)
        })
        .or_else(|| params.pointer("/toolCall/kind").and_then(Value::as_str))
        .map(ToString::to_string);
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|opt| {
                    let option_id = opt.get("optionId").and_then(Value::as_str)?.to_string();
                    let label = opt
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&option_id)
                        .to_string();
                    let kind = opt
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    Some(PermissionOption {
                        option_id,
                        label,
                        kind,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    ParsedPermissionRequest {
        title,
        detail,
        options,
    }
}

pub fn make_permission_outcome(option_id: Option<&str>) -> Value {
    match option_id {
        Some(selected) => json!({
            "outcome": {
                "outcome": "selected",
                "optionId": selected,
            }
        }),
        None => json!({
            "outcome": {
                "outcome": "cancelled",
            }
        }),
    }
}

/// Converts a Huma `PromptInput` (text + image/file attachments) into standard ACP `ContentBlock`s.
pub fn build_prompt_blocks(input: &PromptInput) -> Result<Vec<Value>, std::io::Error> {
    let mut blocks = Vec::new();
    if !input.text.is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": input.text,
        }));
    }

    for attachment in &input.attachments {
        match attachment {
            Attachment::ImagePath { path, mime_type } => {
                let bytes = std::fs::read(path)?;
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                let mime = mime_type
                    .clone()
                    .unwrap_or_else(|| guess_image_mime(path).to_string());
                blocks.push(json!({
                    "type": "image",
                    "mimeType": mime,
                    "data": encoded,
                }));
            }
            Attachment::ImageBase64 { mime_type, data } => {
                blocks.push(json!({
                    "type": "image",
                    "mimeType": mime_type,
                    "data": data,
                }));
            }
            Attachment::FilePath { path } => {
                let uri = format!("file://{path}");
                let name = Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path);
                blocks.push(json!({
                    "type": "resource_link",
                    "uri": uri,
                    "name": name,
                }));
            }
        }
    }

    Ok(blocks)
}

fn guess_image_mime(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "image/png"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_agy_initialize_response() {
        let raw = r#"{"jsonrpc":"2.0","result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":true,"audio":false,"embeddedContext":true}},"agentInfo":{"name":"Antigravity ACP Server","version":"1.5.16"}},"id":1}"#;
        let rpc = parse_incoming_rpc(raw).expect("parse rpc");
        match rpc {
            IncomingRpc::Response {
                result: Some(r), ..
            } => {
                let caps = parse_init_result(&r);
                assert_eq!(caps.agent_name, "Antigravity ACP Server");
                assert_eq!(caps.agent_version.as_deref(), Some("1.5.16"));
                assert!(caps.supports_images);
                assert!(caps.supports_load_session);
            }
            other => panic!("unexpected rpc: {other:?}"),
        }
    }

    #[test]
    fn parses_real_agy_session_updates() {
        let title_msg: Value = serde_json::from_str(
            r#"{"sessionId":"s1","update":{"sessionUpdate":"session_info_update","title":"One Word Reply PONG"}}"#,
        )
        .expect("valid json");
        assert_eq!(
            parse_session_update(&title_msg),
            ParsedSessionUpdate::TitleUpdated("One Word Reply PONG".into())
        );

        let chunk_msg: Value = serde_json::from_str(
            r#"{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"PONG"}}}"#,
        )
        .expect("valid json");
        assert_eq!(
            parse_session_update(&chunk_msg),
            ParsedSessionUpdate::MessageChunk("PONG".into())
        );

        let cmd_msg: Value = serde_json::from_str(
            r#"{"sessionId":"s1","update":{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"compact","description":"Compact history"}]}}"#,
        )
        .expect("valid json");
        assert_eq!(
            parse_session_update(&cmd_msg),
            ParsedSessionUpdate::AvailableCommands(vec![SlashCommand {
                name: "compact".into(),
                description: "Compact history".into(),
            }])
        );
    }

    #[test]
    fn builds_multimodal_prompt_blocks() {
        let input = PromptInput {
            text: "check this screenshot".into(),
            attachments: vec![
                Attachment::ImageBase64 {
                    mime_type: "image/png".into(),
                    data: "AAAA".into(),
                },
                Attachment::FilePath {
                    path: "/tmp/example.rs".into(),
                },
            ],
        };
        let blocks = build_prompt_blocks(&input).expect("build blocks");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["mimeType"], "image/png");
        assert_eq!(blocks[2]["type"], "resource_link");
        assert_eq!(blocks[2]["name"], "example.rs");
    }
}
