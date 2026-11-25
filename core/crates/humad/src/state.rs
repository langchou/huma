use std::collections::HashMap;
use std::path::Path;

use huma_acp::SessionInitInfo;
use huma_protocol::{
    ActivityState, DriverKind, PermissionOption, PermissionRequest, ProjectId, ProjectSnapshot,
    ProjectStatus, ServerEvent, SessionSummary, SlashCommand, TokenUsage, TurnSnapshot,
};

const MAX_RECENT_TURNS: usize = 20;
const MAX_AUTO_TITLE_CHARS: usize = 36;
const MAX_HEADLINE_CHARS: usize = 120;

/// Pure in-memory state reducer for all managed projects.
/// Unit-testable without spawning child processes or sockets.
#[derive(Debug, Default)]
pub struct DaemonState {
    projects: HashMap<ProjectId, ProjectRecord>,
    order: Vec<ProjectId>,
}

#[derive(Debug, Clone)]
struct ProjectRecord {
    snapshot: ProjectSnapshot,
    title_from_agent: bool,
    tool_count_in_turn: u32,
}

impl DaemonState {
    pub fn list_snapshots(&self) -> Vec<ProjectSnapshot> {
        self.order
            .iter()
            .filter_map(|id| self.projects.get(id).map(|r| r.snapshot.clone()))
            .collect()
    }

    #[cfg(test)]
    pub fn snapshot(&self, project_id: &str) -> Option<ProjectSnapshot> {
        self.projects.get(project_id).map(|r| r.snapshot.clone())
    }

    pub fn register_project(
        &mut self,
        project_id: ProjectId,
        cwd: String,
        driver: DriverKind,
        init: SessionInitInfo,
    ) -> (ProjectSnapshot, ServerEvent) {
        let repo_name = Path::new(&cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("workspace")
            .to_string();

        let snapshot = ProjectSnapshot {
            project_id: project_id.clone(),
            repo_name: repo_name.clone(),
            cwd,
            driver,
            semantic_title: repo_name,
            status: ProjectStatus::Idle,
            session_id: Some(init.session_id),
            activity: None,
            summary: None,
            pending_permission: None,
            slash_commands: Vec::new(),
            available_models: init.available_models,
            active_turn: None,
            recent_turns: Vec::new(),
            usage: TokenUsage::default(),
            supports_images: init.supports_images,
            last_error: None,
        };

        self.order.push(project_id.clone());
        self.projects.insert(
            project_id,
            ProjectRecord {
                snapshot: snapshot.clone(),
                title_from_agent: false,
                tool_count_in_turn: 0,
            },
        );

        let event = ServerEvent::ProjectOpened {
            project: Box::new(snapshot.clone()),
        };
        (snapshot, event)
    }

    pub fn remove_project(&mut self, project_id: &str) -> Option<ServerEvent> {
        self.projects.remove(project_id)?;
        self.order.retain(|id| id != project_id);
        Some(ServerEvent::ProjectClosed {
            project_id: project_id.to_string(),
        })
    }

    pub fn begin_turn(
        &mut self,
        project_id: &str,
        turn_id: String,
        user_prompt: String,
        now_ms: u64,
    ) -> Vec<ServerEvent> {
        let Some(rec) = self.projects.get_mut(project_id) else {
            return Vec::new();
        };
        let mut events = Vec::new();

        rec.tool_count_in_turn = 0;
        rec.snapshot.status = ProjectStatus::Working;
        rec.snapshot.last_error = None;
        rec.snapshot.pending_permission = None;
        let initial_activity = ActivityState {
            phase: "思考中".to_string(),
            tool_count: 0,
            detail: None,
        };
        rec.snapshot.activity = Some(initial_activity.clone());
        rec.snapshot.active_turn = Some(TurnSnapshot {
            turn_id,
            user_prompt: user_prompt.clone(),
            assistant_markdown: String::new(),
            tool_events: Vec::new(),
            stop_reason: None,
            started_at_ms: now_ms,
            finished_at_ms: None,
        });

        events.push(ServerEvent::ProjectStatusChanged {
            project_id: project_id.to_string(),
            status: ProjectStatus::Working,
            last_error: None,
        });
        events.push(ServerEvent::ActivityUpdated {
            project_id: project_id.to_string(),
            activity: Some(initial_activity),
        });

        // Auto-derive a semantic title from the first prompt until the agent provides one.
        if !rec.title_from_agent && rec.snapshot.recent_turns.is_empty() {
            let derived = derive_semantic_title(&user_prompt, &rec.snapshot.repo_name);
            if derived != rec.snapshot.semantic_title {
                rec.snapshot.semantic_title = derived.clone();
                events.push(ServerEvent::SemanticTitleUpdated {
                    project_id: project_id.to_string(),
                    semantic_title: derived,
                });
            }
        }

        events
    }

    pub fn on_message_chunk(&mut self, project_id: &str, delta: String) -> Option<ServerEvent> {
        let rec = self.projects.get_mut(project_id)?;
        let turn_id = if let Some(turn) = rec.snapshot.active_turn.as_mut() {
            turn.assistant_markdown.push_str(&delta);
            turn.turn_id.clone()
        } else {
            "untracked".to_string()
        };
        Some(ServerEvent::MessageDelta {
            project_id: project_id.to_string(),
            turn_id,
            delta,
        })
    }

    pub fn on_tool_activity(
        &mut self,
        project_id: &str,
        title: String,
        detail: Option<String>,
    ) -> Option<ServerEvent> {
        let rec = self.projects.get_mut(project_id)?;
        rec.tool_count_in_turn = rec.tool_count_in_turn.saturating_add(1);
        if let Some(turn) = rec.snapshot.active_turn.as_mut() {
            turn.tool_events.push(title.clone());
        }
        let activity = ActivityState {
            phase: title,
            tool_count: rec.tool_count_in_turn,
            detail,
        };
        rec.snapshot.activity = Some(activity.clone());
        Some(ServerEvent::ActivityUpdated {
            project_id: project_id.to_string(),
            activity: Some(activity),
        })
    }

    pub fn on_slash_commands(
        &mut self,
        project_id: &str,
        commands: Vec<SlashCommand>,
    ) -> Option<ServerEvent> {
        let rec = self.projects.get_mut(project_id)?;
        rec.snapshot.slash_commands = commands.clone();
        Some(ServerEvent::SlashCommandsUpdated {
            project_id: project_id.to_string(),
            commands,
        })
    }

    pub fn on_title_updated(&mut self, project_id: &str, title: String) -> Option<ServerEvent> {
        let rec = self.projects.get_mut(project_id)?;
        rec.title_from_agent = true;
        if rec.snapshot.semantic_title == title {
            return None;
        }
        rec.snapshot.semantic_title = title.clone();
        Some(ServerEvent::SemanticTitleUpdated {
            project_id: project_id.to_string(),
            semantic_title: title,
        })
    }

    pub fn on_usage_updated(
        &mut self,
        project_id: &str,
        context_used: Option<u64>,
        context_limit: Option<u64>,
    ) -> Option<ServerEvent> {
        let rec = self.projects.get_mut(project_id)?;
        if context_used.is_some() {
            rec.snapshot.usage.context_used = context_used;
        }
        if context_limit.is_some() {
            rec.snapshot.usage.context_limit = context_limit;
        }
        Some(ServerEvent::UsageUpdated {
            project_id: project_id.to_string(),
            usage: rec.snapshot.usage.clone(),
        })
    }

    pub fn on_permission_requested(
        &mut self,
        project_id: &str,
        request_id: String,
        title: String,
        detail: Option<String>,
        options: Vec<PermissionOption>,
    ) -> Vec<ServerEvent> {
        let Some(rec) = self.projects.get_mut(project_id) else {
            return Vec::new();
        };
        let req = PermissionRequest {
            request_id,
            project_id: project_id.to_string(),
            title,
            detail,
            options,
        };
        rec.snapshot.status = ProjectStatus::NeedsAttention;
        rec.snapshot.pending_permission = Some(req.clone());
        vec![
            ServerEvent::ProjectStatusChanged {
                project_id: project_id.to_string(),
                status: ProjectStatus::NeedsAttention,
                last_error: None,
            },
            ServerEvent::PermissionRequested { request: req },
        ]
    }

    pub fn on_permission_resolved(
        &mut self,
        project_id: &str,
        request_id: &str,
    ) -> Vec<ServerEvent> {
        let Some(rec) = self.projects.get_mut(project_id) else {
            return Vec::new();
        };
        rec.snapshot.pending_permission = None;
        rec.snapshot.status = ProjectStatus::Working;
        vec![
            ServerEvent::PermissionResolved {
                project_id: project_id.to_string(),
                request_id: request_id.to_string(),
            },
            ServerEvent::ProjectStatusChanged {
                project_id: project_id.to_string(),
                status: ProjectStatus::Working,
                last_error: None,
            },
        ]
    }

    pub fn finish_turn(
        &mut self,
        project_id: &str,
        turn_id: String,
        stop_reason: Option<String>,
        turn_usage: TokenUsage,
        now_ms: u64,
    ) -> Vec<ServerEvent> {
        let Some(rec) = self.projects.get_mut(project_id) else {
            return Vec::new();
        };

        rec.snapshot.usage.input_tokens = rec
            .snapshot
            .usage
            .input_tokens
            .saturating_add(turn_usage.input_tokens);
        rec.snapshot.usage.output_tokens = rec
            .snapshot
            .usage
            .output_tokens
            .saturating_add(turn_usage.output_tokens);
        rec.snapshot.usage.thought_tokens = rec
            .snapshot
            .usage
            .thought_tokens
            .saturating_add(turn_usage.thought_tokens);
        rec.snapshot.usage.total_tokens = rec
            .snapshot
            .usage
            .total_tokens
            .saturating_add(turn_usage.total_tokens);

        let latest_markdown = if let Some(mut turn) = rec.snapshot.active_turn.take() {
            turn.stop_reason = stop_reason.clone();
            turn.finished_at_ms = Some(now_ms);
            let md = turn.assistant_markdown.clone();
            rec.snapshot.recent_turns.push(turn);
            if rec.snapshot.recent_turns.len() > MAX_RECENT_TURNS {
                let drop_n = rec.snapshot.recent_turns.len() - MAX_RECENT_TURNS;
                rec.snapshot.recent_turns.drain(0..drop_n);
            }
            md
        } else {
            String::new()
        };

        let summary = SessionSummary {
            headline: extract_headline(&latest_markdown),
            latest_markdown,
            updated_at_ms: now_ms,
        };
        rec.snapshot.summary = Some(summary.clone());
        rec.snapshot.activity = None;
        rec.snapshot.pending_permission = None;
        rec.snapshot.status = ProjectStatus::Ready;

        vec![
            ServerEvent::ActivityUpdated {
                project_id: project_id.to_string(),
                activity: None,
            },
            ServerEvent::TurnCompleted {
                project_id: project_id.to_string(),
                turn_id,
                stop_reason,
                summary,
                usage: rec.snapshot.usage.clone(),
            },
            ServerEvent::ProjectStatusChanged {
                project_id: project_id.to_string(),
                status: ProjectStatus::Ready,
                last_error: None,
            },
        ]
    }

    pub fn fail_project(&mut self, project_id: &str, error: String) -> Vec<ServerEvent> {
        let Some(rec) = self.projects.get_mut(project_id) else {
            return Vec::new();
        };
        rec.snapshot.status = ProjectStatus::Error;
        rec.snapshot.activity = None;
        rec.snapshot.active_turn = None;
        rec.snapshot.last_error = Some(error.clone());
        vec![
            ServerEvent::ActivityUpdated {
                project_id: project_id.to_string(),
                activity: None,
            },
            ServerEvent::ProjectStatusChanged {
                project_id: project_id.to_string(),
                status: ProjectStatus::Error,
                last_error: Some(error),
            },
        ]
    }
}

fn derive_semantic_title(prompt: &str, fallback: &str) -> String {
    let first_line = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback);
    truncate_chars(first_line, MAX_AUTO_TITLE_CHARS)
}

fn extract_headline(markdown: &str) -> String {
    let line = markdown
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty() && !l.starts_with("```"))
        .unwrap_or("已完成当前轮次");
    truncate_chars(line, MAX_HEADLINE_CHARS)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_init() -> SessionInitInfo {
        SessionInitInfo {
            session_id: "sess-1".into(),
            agent_name: "Test Agent".into(),
            agent_version: Some("1.0".into()),
            supports_images: true,
            available_models: Vec::new(),
        }
    }

    #[test]
    fn auto_title_derives_from_prompt_and_yields_to_agent_title() {
        let mut state = DaemonState::default();
        let (snap, _) = state.register_project(
            "p1".into(),
            "/work/auth-service".into(),
            DriverKind::Agy,
            sample_init(),
        );
        assert_eq!(snap.semantic_title, "auth-service");

        let events = state.begin_turn(
            "p1",
            "t1".into(),
            "Fix token refresh race condition".into(),
            100,
        );
        assert!(events.iter().any(|e| matches!(
            e,
            ServerEvent::SemanticTitleUpdated { semantic_title, .. }
            if semantic_title == "Fix token refresh race condition"
        )));

        let override_ev = state.on_title_updated("p1", "Token Refresh Lock".into());
        assert!(matches!(
            override_ev,
            Some(ServerEvent::SemanticTitleUpdated { semantic_title, .. })
            if semantic_title == "Token Refresh Lock"
        ));
    }

    #[test]
    fn folds_tool_noise_and_produces_clean_summary_on_finish() {
        let mut state = DaemonState::default();
        state.register_project(
            "p1".into(),
            "/work/huma".into(),
            DriverKind::Agy,
            sample_init(),
        );
        state.begin_turn("p1", "t1".into(), "audit logs".into(), 10);

        state.on_tool_activity("p1", "Reading audit.rs".into(), Some("src/audit.rs".into()));
        state.on_tool_activity("p1", "Running cargo test".into(), None);
        state.on_message_chunk("p1", "## Summary\nAll 12 tests passed.".into());

        let snap_mid = state.snapshot("p1").expect("project exists");
        assert_eq!(snap_mid.activity.as_ref().map(|a| a.tool_count), Some(2));

        state.finish_turn(
            "p1",
            "t1".into(),
            Some("end_turn".into()),
            TokenUsage::default(),
            50,
        );
        let snap_done = state.snapshot("p1").expect("project exists");
        assert_eq!(snap_done.status, ProjectStatus::Ready);
        assert!(snap_done.activity.is_none());
        assert_eq!(
            snap_done.summary.as_ref().map(|s| s.headline.as_str()),
            Some("Summary")
        );
    }
}
