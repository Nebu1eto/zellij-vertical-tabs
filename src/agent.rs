use std::collections::HashMap;

use serde_json::Value;

/// An agent falls silent between hooks while the model streams, so an active
/// state must not decay on a tool-call timescale: it ends when the agent reports
/// back. This timeout only catches an agent that died without a closing hook.
pub(crate) const AGENT_STALL_SECONDS: u64 = 900;

/// Every plugin instance mounts Zellij's shared temp directory at `/tmp`, which
/// lets a sidebar in a new tab pick up statuses collected by its siblings.
const AGENT_SYNC_DIR: &str = "/tmp";

/// Lifecycle state shown in both views, following herdr's vocabulary so the
/// glyph, the label and the colour always agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentState {
    Idle,
    Thinking,
    Working,
    Compacting,
    Blocked,
    Done,
}

impl AgentState {
    pub(crate) fn glyph(self) -> char {
        match self {
            AgentState::Idle => '○',
            AgentState::Thinking | AgentState::Working => '●',
            AgentState::Compacting => '◍',
            AgentState::Blocked => '◉',
            AgentState::Done => '✓',
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Thinking => "thinking",
            AgentState::Working => "working",
            AgentState::Compacting => "compacting",
            AgentState::Blocked => "blocked",
            AgentState::Done => "done",
        }
    }

    pub(crate) fn urgent(self) -> bool {
        matches!(self, AgentState::Blocked)
    }

    pub(crate) fn from_label(label: &str) -> Option<Self> {
        match label {
            "idle" => Some(AgentState::Idle),
            "thinking" => Some(AgentState::Thinking),
            "working" => Some(AgentState::Working),
            "compacting" => Some(AgentState::Compacting),
            "blocked" => Some(AgentState::Blocked),
            "done" => Some(AgentState::Done),
            _ => None,
        }
    }
}

/// A coding agent raises a notification for two opposite reasons: it needs
/// approval to continue, or it has finished its turn and is waiting for the
/// user. Only the first is a block; the second is simply done.
pub(crate) fn notification_state(
    message: Option<&str>,
) -> (AgentState, Option<String>, Option<u64>) {
    let text = message.unwrap_or_default().to_ascii_lowercase();
    let needs_user = ["permission", "approve", "approval", "authoriz", "confirm"]
        .iter()
        .any(|needle| text.contains(needle));
    if needs_user {
        let detail = message
            .filter(|message| !message.trim().is_empty())
            .map(str::to_string)
            .or_else(|| Some("notification".to_string()));
        (AgentState::Blocked, detail, None)
    } else {
        (AgentState::Done, None, None)
    }
}

/// choco-pi names its tools for the API, not for a status line. These are the
/// ones worth naming by hand; anything else is humanised from its identifier,
/// so a new tool still reads as words rather than as code.
pub(crate) fn choco_pi_tool_label(tool: &str) -> String {
    let named = match tool {
        "exec" => "code mode",
        "mcpScript" => "mcp code",
        "apply_patch" => "editing",
        "read_text" => "reading",
        "read_symbol" | "read_enclosing" => "reading code",
        "symbol_search" | "ast_grep_search" => "searching code",
        "module_report" | "project_report" => "mapping code",
        "lsp_diagnostics" | "diagnostics_report" => "diagnostics",
        "lsp_navigation" => "navigating code",
        "web_search" | "synthetic_web_search" => "web search",
        "fetch_content" => "fetching",
        "source_check" => "fact check",
        "shell_start" | "shell_read" | "shell_stop" => "shell",
        "find" | "ls" => "browsing files",
        "Agent" | "get_subagent_result" => "subagent",
        "steer_subagent" | "stop_subagent" => "steering agent",
        "workflow_run" | "workflow_update" => "workflow",
        "TaskCreate" | "TaskUpdate" => "task list",
        "imagegen" => "image",
        other => return humanised_tool_name(other),
    };
    named.to_string()
}

/// `read_text` reads as "read text" and `agentBrowser` as "agent browser".
pub(crate) fn humanised_tool_name(tool: &str) -> String {
    let mut words = String::new();
    for (index, character) in tool.chars().enumerate() {
        if character == '_' || character == '-' {
            words.push(' ');
        } else if character.is_uppercase() && index > 0 {
            words.push(' ');
            words.extend(character.to_lowercase());
        } else {
            words.extend(character.to_lowercase());
        }
    }
    let words = words.trim();
    if words.is_empty() {
        return tool.to_string();
    }
    words.to_string()
}

/// choco-pi runs either a batch of calls inside one code-mode cell or a single
/// tool directly, and the card should say which. Other agents keep their own
/// tool names untouched.
pub(crate) fn tool_detail(source: &str, tool: Option<String>) -> Option<String> {
    let tool = tool?;
    if !source.eq_ignore_ascii_case("choco-pi") {
        return Some(tool);
    }
    Some(choco_pi_tool_label(&tool))
}

pub(crate) fn agent_sync_path(session_name: &str) -> Option<String> {
    let name: String = session_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    if name.is_empty() {
        return None;
    }
    Some(format!(
        "{}/zellij-agent-status-{}.json",
        AGENT_SYNC_DIR, name
    ))
}

/// Statuses detected from the pane manifest stay out of the shared file: every
/// instance reads the same manifest, so sharing them would only fight the local
/// detection.
pub(crate) fn encode_agent_statuses(statuses: &HashMap<u32, AgentStatus>) -> String {
    let mut shared: Vec<&AgentStatus> = statuses.values().filter(|s| !s.detected).collect();
    shared.sort_by_key(|status| status.pane_id);
    let entries: Vec<Value> = shared
        .iter()
        .map(|status| {
            serde_json::json!({
                "pane_id": status.pane_id,
                "source": status.source,
                "state": status.state.label(),
                "detail": status.detail,
                "summary": status.summary,
                "since": status.since,
                "expires_at": status.expires_at,
                "updated_at": status.updated_at,
                "clear_on_focus": status.clear_on_focus,
            })
        })
        .collect();
    serde_json::json!({ "version": 1, "statuses": entries }).to_string()
}

pub(crate) fn decode_agent_statuses(payload: &str) -> Vec<AgentStatus> {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Vec::new();
    };
    value
        .get("statuses")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let pane_id = entry.get("pane_id").and_then(Value::as_u64)? as u32;
                    let state = entry
                        .get("state")
                        .and_then(Value::as_str)
                        .and_then(AgentState::from_label)?;
                    Some(AgentStatus {
                        pane_id,
                        source: entry
                            .get("source")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        state,
                        detail: entry
                            .get("detail")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        summary: entry
                            .get("summary")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        since: entry.get("since").and_then(Value::as_u64).unwrap_or(0),
                        sequence: 0,
                        expires_at: entry.get("expires_at").and_then(Value::as_u64),
                        updated_at: entry.get("updated_at").and_then(Value::as_u64).unwrap_or(0),
                        detected: false,
                        clear_on_focus: entry
                            .get("clear_on_focus")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
pub(crate) struct AgentStatus {
    pub(crate) pane_id: u32,
    pub(crate) source: String,
    pub(crate) state: AgentState,
    pub(crate) detail: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) since: u64,
    pub(crate) sequence: u64,
    pub(crate) expires_at: Option<u64>,
    /// When this status last changed, in unix seconds. Sidebars in other tabs
    /// compare it to decide whose copy of a pane's status is the current one.
    pub(crate) updated_at: u64,
    pub(crate) detected: bool,
    pub(crate) clear_on_focus: bool,
}

impl AgentStatus {
    pub(crate) fn urgent(&self) -> bool {
        self.state.urgent()
    }

    /// Single-line form used by the horizontal bar.
    pub(crate) fn message(&self) -> String {
        let detail = self
            .detail
            .as_deref()
            .map(|detail| format!(" · {detail}"))
            .unwrap_or_default();
        format!(
            "{} {} {}{detail}",
            self.state.glyph(),
            self.source,
            self.state.label()
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AgentEntry {
    pub(crate) pane_id: u32,
    pub(crate) state: AgentState,
    pub(crate) name: String,
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentEvent {
    pub(crate) source: String,
    pub(crate) event: String,
    pub(crate) tool: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) pane_id: u32,
    pub(crate) timestamp: Option<u64>,
}

pub(crate) fn parse_agent_event(payload: &str) -> Option<AgentEvent> {
    let payload = serde_json::from_str::<Value>(payload).ok()?;
    Some(AgentEvent {
        source: agent_label(
            payload
                .get("source_agent")
                .and_then(Value::as_str)
                .unwrap_or("coding agent"),
        ),
        event: payload.get("hook_event")?.as_str()?.to_string(),
        tool: payload
            .get("tool_name")
            .and_then(Value::as_str)
            .filter(|tool| !tool.is_empty())
            .map(str::to_string),
        summary: payload
            .get("summary")
            .and_then(Value::as_str)
            .map(|summary| summary.lines().next().unwrap_or("").trim().to_string())
            .filter(|summary| !summary.is_empty()),
        pane_id: payload.get("pane_id").and_then(Value::as_u64).unwrap_or(0) as u32,
        timestamp: payload.get("ts_ms").and_then(Value::as_u64),
    })
}

pub(crate) fn agent_label(source: &str) -> String {
    match source {
        "choco-pi" => "choco-pi",
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        other => other,
    }
    .to_string()
}

pub(crate) fn detected_agent_label(command: &str) -> Option<&'static str> {
    match command {
        "claude" | "claude-code" => Some("Claude Code"),
        "codex" => Some("Codex"),
        "pi" | "choco-pi" => Some("choco-pi"),
        "opencode" => Some("OpenCode"),
        "gemini" => Some("Gemini"),
        "cursor-agent" | "cursor" => Some("Cursor"),
        "aider" => Some("Aider"),
        "amp" => Some("Amp"),
        _ => None,
    }
}
