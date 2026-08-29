use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::format::{strftime::StrftimeItems, Item};
use chrono::{FixedOffset, Utc};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;

const AGENT_PIPE: &str = "coding-agent-status";
const AGENT_FOCUS_PIPE: &str = "coding-agent-status:focus";
const DEBUG_TRIGGER_PATH: &str = "/host/.zellij-vtabs-debug";
const VERTICAL_SIDEBAR_URL_SUFFIX: &str = "/vertical-sidebar.wasm";
const WIDTH_SYNC_MAX_ATTEMPTS: u8 = 64;
const DEFAULT_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M";
const GIT_COMMAND: &str =
    "root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 1; printf '%s\\n' \"$root\"; git status --porcelain=v1 --branch";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Style {
    fg: Rgb,
    bg: Rgb,
    bold: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Colors {
    background: Rgb,
    session: Style,
    mode_normal: Style,
    mode_locked: Style,
    mode_resize: Style,
    mode_pane: Style,
    mode_tab: Style,
    mode_search: Style,
    mode_rename_tab: Style,
    mode_rename_pane: Style,
    mode_move: Style,
    mode_default: Style,
    tab_normal: Style,
    tab_active: Style,
    cwd_normal: Style,
    cwd_active: Style,
    context: Style,
    clock: Style,
    border: Style,
    agent: Style,
    agent_urgent: Style,
}

impl Default for Colors {
    fn default() -> Self {
        Self::from_config(&BTreeMap::new())
    }
}

impl Colors {
    fn from_config(configuration: &BTreeMap<String, String>) -> Self {
        let style = |name: &str, default_fg: Rgb, default_bg: Rgb, bold| Style {
            fg: configured_color(configuration, &format!("{name}_fg"), default_fg),
            bg: configured_color(configuration, &format!("{name}_bg"), default_bg),
            bold,
        };
        let background = configured_color(configuration, "color_background", Rgb(46, 52, 64));
        Self {
            background,
            session: style("color_session", Rgb(216, 222, 233), Rgb(59, 66, 82), true),
            mode_normal: style(
                "color_mode_normal",
                Rgb(46, 52, 64),
                Rgb(163, 190, 140),
                true,
            ),
            mode_locked: style(
                "color_mode_locked",
                Rgb(236, 239, 244),
                Rgb(191, 97, 106),
                true,
            ),
            mode_resize: style(
                "color_mode_resize",
                Rgb(46, 52, 64),
                Rgb(235, 203, 139),
                true,
            ),
            mode_pane: style("color_mode_pane", Rgb(46, 52, 64), Rgb(136, 192, 208), true),
            mode_tab: style("color_mode_tab", Rgb(46, 52, 64), Rgb(180, 142, 173), true),
            mode_search: style(
                "color_mode_search",
                Rgb(46, 52, 64),
                Rgb(235, 203, 139),
                true,
            ),
            mode_rename_tab: style(
                "color_mode_rename_tab",
                Rgb(46, 52, 64),
                Rgb(208, 135, 112),
                true,
            ),
            mode_rename_pane: style(
                "color_mode_rename_pane",
                Rgb(46, 52, 64),
                Rgb(208, 135, 112),
                true,
            ),
            mode_move: style("color_mode_move", Rgb(46, 52, 64), Rgb(180, 142, 173), true),
            mode_default: style(
                "color_mode_default",
                Rgb(216, 222, 233),
                Rgb(76, 86, 106),
                true,
            ),
            tab_normal: style(
                "color_tab_normal",
                Rgb(216, 222, 233),
                Rgb(59, 66, 82),
                false,
            ),
            tab_active: style(
                "color_tab_active",
                Rgb(46, 52, 64),
                Rgb(136, 192, 208),
                true,
            ),
            cwd_normal: style(
                "color_cwd_normal",
                Rgb(129, 161, 193),
                Rgb(46, 52, 64),
                false,
            ),
            cwd_active: style(
                "color_cwd_active",
                Rgb(236, 239, 244),
                Rgb(94, 129, 172),
                true,
            ),
            context: style("color_context", Rgb(216, 222, 233), Rgb(59, 66, 82), false),
            clock: style("color_clock", Rgb(46, 52, 64), Rgb(136, 192, 208), true),
            border: style("color_border", Rgb(76, 86, 106), background, false),
            agent: style("color_agent", Rgb(46, 52, 64), Rgb(163, 190, 140), true),
            agent_urgent: style(
                "color_agent_urgent",
                Rgb(236, 239, 244),
                Rgb(191, 97, 106),
                true,
            ),
        }
    }

    fn mode(&self, mode: InputMode) -> Style {
        match mode {
            InputMode::Normal => self.mode_normal,
            InputMode::Locked => self.mode_locked,
            InputMode::Resize => self.mode_resize,
            InputMode::Pane => self.mode_pane,
            InputMode::Tab => self.mode_tab,
            InputMode::Scroll | InputMode::EnterSearch | InputMode::Search => self.mode_search,
            InputMode::RenameTab => self.mode_rename_tab,
            InputMode::RenamePane => self.mode_rename_pane,
            InputMode::Move => self.mode_move,
            InputMode::Session | InputMode::Prompt | InputMode::Tmux => self.mode_default,
        }
    }
}

struct AnsiFrame {
    output: String,
    rows: usize,
    cols: usize,
}

impl AnsiFrame {
    fn new(rows: usize, cols: usize, colors: &Colors) -> Self {
        let mut frame = Self {
            output: String::with_capacity(rows.saturating_mul(cols.saturating_add(32))),
            rows,
            cols,
        };
        let clear = Style {
            fg: colors.background,
            bg: colors.background,
            bold: false,
        };
        for y in 0..rows {
            frame.put(0, y, clear, &" ".repeat(cols));
        }
        frame
    }

    fn put(&mut self, x: usize, y: usize, style: Style, value: &str) {
        if x >= self.cols || y >= self.rows {
            return;
        }
        let value = sanitize_and_clip(value, self.cols - x);
        if value.is_empty() {
            return;
        }
        let _ = write!(
            self.output,
            "\x1b[{};{}H{}{}\x1b[0m",
            y + 1,
            x + 1,
            ansi_style(style),
            value
        );
    }

    fn finish(mut self) -> String {
        self.output.push_str("\x1b[0m");
        self.output
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum View {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Default)]
struct State {
    plugin_id: Option<u32>,
    view: View,
    mode: InputMode,
    session_name: String,
    tabs: Vec<TabInfo>,
    panes: PaneManifest,
    active_pane_id: Option<PaneId>,
    active_cwd: Option<PathBuf>,
    active_command: Option<String>,
    cwd_by_tab: HashMap<usize, PathBuf>,
    repo_by_tab: HashMap<usize, RepoInfo>,
    repo_cwd_by_tab: HashMap<usize, PathBuf>,
    /// The shared status file's content as this instance last saw it, so a write
    /// happens only on a real change and a read only on a foreign one.
    agent_sync_payload: Option<String>,
    cwd_error: Option<String>,
    configured_home: Option<PathBuf>,
    git_context: Option<GitContext>,
    git_refresh_pending: bool,
    permissions_granted: bool,
    agent_statuses: HashMap<u32, AgentStatus>,
    agent_sequence: u64,
    focused_terminal_pane: Option<u32>,
    agent_focus_targets: Vec<(usize, usize, u32)>,
    timer_ticks: u8,
    git_refresh_interval: u8,
    timezone_offset_hours: i32,
    datetime_format: String,
    show_tabs: bool,
    border_enabled: bool,
    border_char: String,
    vertical_separator_enabled: bool,
    vertical_separator_char: String,
    colors: Colors,
    visible_vertical_tabs: Vec<(usize, usize)>,
    visible_horizontal_tabs: Vec<TabHitbox>,
    last_hook_timestamp_by_pane: HashMap<u32, u64>,
    session_end_timestamp_by_pane: HashMap<u32, u64>,
    pending_width_sync: Option<PendingWidthSync>,
    last_observed_sidebar_width: Option<usize>,
    tabs_with_user_content: HashSet<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingWidthSync {
    target_width: usize,
    pane_ids: Vec<u32>,
    last_requested_widths: HashMap<u32, usize>,
    attempts_remaining: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitContext {
    cwd: PathBuf,
    repository: String,
    branch: String,
    dirty: bool,
}

/// Repository identity for a tab, read straight from the git directory.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RepoInfo {
    repository: String,
    branch: String,
    worktree: Option<String>,
}

/// Lifecycle state shown in both views, following herdr's vocabulary so the
/// glyph, the label and the colour always agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentState {
    Idle,
    Thinking,
    Working,
    Compacting,
    Blocked,
    Done,
}

impl AgentState {
    fn glyph(self) -> char {
        match self {
            AgentState::Idle => '○',
            AgentState::Thinking | AgentState::Working => '●',
            AgentState::Compacting => '◍',
            AgentState::Blocked => '◉',
            AgentState::Done => '✓',
        }
    }

    fn label(self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Thinking => "thinking",
            AgentState::Working => "working",
            AgentState::Compacting => "compacting",
            AgentState::Blocked => "blocked",
            AgentState::Done => "done",
        }
    }

    fn urgent(self) -> bool {
        matches!(self, AgentState::Blocked)
    }

    fn from_label(label: &str) -> Option<Self> {
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

/// An agent falls silent between hooks while the model streams, so an active
/// state must not decay on a tool-call timescale: it ends when the agent reports
/// back. This timeout only catches an agent that died without a closing hook.
const AGENT_STALL_SECONDS: u64 = 900;

/// A coding agent raises a notification for two opposite reasons: it needs
/// approval to continue, or it has finished its turn and is waiting for the
/// user. Only the first is a block; the second is simply done.
fn notification_state(message: Option<&str>) -> (AgentState, Option<String>, Option<u64>) {
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
fn choco_pi_tool_label(tool: &str) -> String {
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
fn humanised_tool_name(tool: &str) -> String {
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
fn tool_detail(source: &str, tool: Option<String>) -> Option<String> {
    let tool = tool?;
    if !source.eq_ignore_ascii_case("choco-pi") {
        return Some(tool);
    }
    Some(choco_pi_tool_label(&tool))
}

/// Every plugin instance mounts zellij's shared temp directory at `/tmp`, which
/// makes it the one place a sidebar in a new tab can pick up the agent statuses
/// its siblings already collected.
const AGENT_SYNC_DIR: &str = "/tmp";

fn agent_sync_path(session_name: &str) -> Option<String> {
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
fn encode_agent_statuses(statuses: &HashMap<u32, AgentStatus>) -> String {
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

fn decode_agent_statuses(payload: &str) -> Vec<AgentStatus> {
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
struct AgentStatus {
    pane_id: u32,
    source: String,
    state: AgentState,
    detail: Option<String>,
    summary: Option<String>,
    since: u64,
    sequence: u64,
    expires_at: Option<u64>,
    /// When this status last changed, in unix seconds. Sidebars in other tabs
    /// compare it to decide whose copy of a pane's status is the current one.
    updated_at: u64,
    detected: bool,
    clear_on_focus: bool,
}

impl AgentStatus {
    fn urgent(&self) -> bool {
        self.state.urgent()
    }

    /// Single-line form used by the horizontal bar.
    fn message(&self) -> String {
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
struct AgentEntry {
    pane_id: u32,
    state: AgentState,
    name: String,
    detail: Option<String>,
}

#[derive(Clone, Debug)]
struct AgentEvent {
    source: String,
    event: String,
    tool: Option<String>,
    summary: Option<String>,
    pane_id: u32,
    timestamp: Option<u64>,
}

#[derive(Clone, Debug)]
struct TabHitbox {
    start: usize,
    end: usize,
    position: usize,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.plugin_id = Some(get_plugin_ids().plugin_id);
        // Stay focusable until permissions are granted: zellij renders its
        // permission prompt inside this pane and withholds every event until it
        // is answered, which is impossible on a pane nobody can focus.
        set_selectable(true);
        self.view = match configuration.get("view").map(String::as_str) {
            Some("vertical") => View::Vertical,
            _ => View::Horizontal,
        };
        self.configured_home = configuration
            .get("home")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from);
        self.timezone_offset_hours = configuration
            .get("timezone_offset_hours")
            .and_then(|value| value.parse().ok())
            .unwrap_or(9)
            .clamp(-23, 23);
        self.datetime_format =
            validated_datetime_format(configuration.get("datetime_format").map(String::as_str));
        self.show_tabs = configuration
            .get("show_tabs")
            .is_none_or(|value| value != "false");
        self.border_enabled = configuration
            .get("border_enabled")
            .is_none_or(|value| value != "false");
        self.border_char = configuration
            .get("border_char")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| "─".to_string());
        self.vertical_separator_enabled = configuration
            .get("vertical_separator_enabled")
            .is_none_or(|value| value != "false");
        self.vertical_separator_char = configuration
            .get("vertical_separator_char")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| "│".to_string());
        self.git_refresh_interval = configuration
            .get("command_git_branch_interval")
            .and_then(|value| value.parse().ok())
            .unwrap_or(10)
            .clamp(1, u8::MAX as u64) as u8;
        self.colors = Colors::from_config(&configuration);

        // Subscribe before asking: an already-cached grant is answered immediately,
        // and the result event is lost when nothing is listening for it yet.
        subscribe_to_events();
        set_timeout(1.0);
        request_permission(permissions_for_view(self.view));
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::ModeUpdate(mode_info) => {
                self.mode = mode_info.mode;
                self.session_name = mode_info.session_name.unwrap_or_default();
                // A sidebar opened in a new tab starts empty; this is the first
                // moment it knows which session's statuses to adopt.
                self.hydrate_agent_statuses();
            }
            Event::TabUpdate(mut tabs) => {
                self.note_permissions_granted();
                tabs.sort_by_key(|tab| tab.position);
                self.tabs = tabs;
                self.close_empty_own_tab_if_needed();
                self.refresh_active_pane();
                self.refresh_cwds();
            }
            Event::PaneUpdate(panes) => {
                self.note_permissions_granted();
                self.panes = panes;
                self.track_focused_pane();
                self.detect_agents_from_manifest();
                self.observe_sidebar_width_change();
                self.sync_sidebar_widths();
                self.close_empty_own_tab_if_needed();
                self.refresh_active_pane();
                self.refresh_cwds();
            }
            Event::CwdChanged(pane_id, cwd, _) => {
                if Some(pane_id) == self.active_pane_id {
                    self.update_cwd(cwd.clone());
                }
                if let Some(tab_position) = self.tab_for_selected_pane(pane_id) {
                    self.cwd_by_tab.insert(tab_position, cwd);
                }
            }
            Event::CommandChanged(pane_id, command, is_foreground, _) => {
                if Some(pane_id) == self.active_pane_id {
                    self.active_command = is_foreground.then(|| command_label(&command));
                }
                if let PaneId::Terminal(id) = pane_id {
                    self.update_detected_agent(id, &command, is_foreground);
                }
            }
            Event::RunCommandResult(exit_code, stdout, _, context) => {
                if context.get("kind").map(String::as_str) == Some("repo") {
                    if let Some(position) =
                        context.get("tab").and_then(|tab| tab.parse::<usize>().ok())
                    {
                        match (exit_code, parse_repo_info(&stdout)) {
                            (Some(0), Some(info)) => {
                                self.repo_by_tab.insert(position, info);
                            }
                            _ => {
                                self.repo_by_tab.remove(&position);
                            }
                        }
                    }
                    return true;
                }
                self.git_refresh_pending = false;
                if exit_code != Some(0) {
                    self.git_context = None;
                } else if let Some(cwd) = context
                    .get("cwd")
                    .map(PathBuf::from)
                    .filter(|cwd| self.active_cwd.as_ref() == Some(cwd))
                {
                    self.git_context = parse_git_context(&stdout, cwd);
                }
            }
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                set_selectable(view_selectable(self.view));
                self.permissions_granted = true;
                // Subscriptions and timers requested before the grant are dropped,
                // so re-arm them here or the pane never receives another event.
                subscribe_to_events();
                set_timeout(1.0);
                self.active_pane_id = None;
                self.refresh_active_pane();
                self.refresh_cwds();
                self.refresh_git();
            }
            Event::Timer(_) => {
                set_timeout(1.0);
                self.hydrate_agent_statuses();
                if std::path::Path::new(DEBUG_TRIGGER_PATH).exists() {
                    let view = match self.view {
                        View::Horizontal => "horizontal",
                        View::Vertical => "vertical",
                    };
                    let path = format!(
                        "/host/.zellij-vtabs-debug-{view}-{}.json",
                        self.plugin_id.unwrap_or(0)
                    );
                    let _ = std::fs::write(path, self.debug_snapshot());
                }
                let now = unix_seconds();
                // An expired status does not mean the agent left, only that it
                // went quiet: keep it listed as idle so the section stays complete.
                for status in self.agent_statuses.values_mut() {
                    if status
                        .expires_at
                        .is_some_and(|expires_at| expires_at <= now)
                    {
                        status.state = AgentState::Idle;
                        status.detail = None;
                        status.expires_at = None;
                        status.clear_on_focus = false;
                        status.since = now;
                        status.updated_at = now;
                    }
                }
                self.timer_ticks = self.timer_ticks.wrapping_add(1);
                if self.timer_ticks >= self.git_refresh_interval {
                    self.timer_ticks = 0;
                    // Branch changes are picked up on the same cadence as the bar's
                    // git state; refreshing cwds first also recovers tabs whose
                    // directory was unknown when an event arrived.
                    self.repo_cwd_by_tab.clear();
                    self.refresh_cwds();
                    self.refresh_git();
                }
            }
            Event::Mouse(Mouse::LeftClick(line, column)) if line >= 0 => {
                match self.view {
                    View::Vertical => {
                        let line_index = line as usize;
                        if let Some((_, _, pane_id)) =
                            self.agent_focus_targets
                                .iter()
                                .find(|(target_line, start, _)| {
                                    *target_line == line_index && column >= *start
                                })
                        {
                            focus_terminal_pane(*pane_id, false, false);
                        } else if let Some((_, position)) =
                            self.visible_vertical_tabs.iter().find(|(title_line, _)| {
                                line_index == *title_line || line_index == *title_line + 1
                            })
                        {
                            switch_tab_to((*position + 1) as u32);
                        }
                    }
                    View::Horizontal => {
                        let column = column as usize;
                        if let Some(hitbox) = self
                            .visible_horizontal_tabs
                            .iter()
                            .find(|hitbox| column >= hitbox.start && column < hitbox.end)
                        {
                            switch_tab_to((hitbox.position + 1) as u32);
                        }
                    }
                }
                return false;
            }
            _ => {}
        }
        self.persist_agent_statuses();
        true
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name == AGENT_FOCUS_PIPE {
            if let Some(pane_id) = pipe_message
                .payload
                .and_then(|payload| payload.trim().parse::<u32>().ok())
            {
                focus_terminal_pane(pane_id, false, false);
            }
            return false;
        }
        if pipe_message.name != AGENT_PIPE {
            return false;
        }
        let Some(payload) = pipe_message.payload else {
            return false;
        };
        let Some(event) = parse_agent_event(&payload) else {
            return false;
        };
        let changed = self.handle_agent_event(event);
        self.persist_agent_statuses();
        changed
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if rows == 0 || cols == 0 {
            return;
        }
        let mut frame = AnsiFrame::new(rows, cols, &self.colors);
        match self.view {
            View::Horizontal => self.render_horizontal(&mut frame, rows, cols),
            View::Vertical => self.render_vertical(&mut frame, rows, cols),
        }
        print!("{}", frame.finish());
    }
}

impl State {
    fn observe_sidebar_width_change(&mut self) {
        if self.view != View::Vertical {
            return;
        }
        let Some(plugin_id) = self.plugin_id else {
            return;
        };
        let Some((width, _)) = active_sidebar_state(&self.tabs, &self.panes, plugin_id) else {
            return;
        };
        let previous_width = self.last_observed_sidebar_width.replace(width);
        if previous_width.is_none_or(|previous| previous == width) {
            return;
        }

        let mut pane_ids = visible_vertical_sidebar_ids(&self.panes);
        if !pane_ids.contains(&plugin_id) {
            pane_ids.push(plugin_id);
            pane_ids.sort_unstable();
        }
        self.pending_width_sync = Some(PendingWidthSync {
            target_width: width,
            pane_ids,
            last_requested_widths: HashMap::new(),
            attempts_remaining: WIDTH_SYNC_MAX_ATTEMPTS,
        });
    }

    fn close_empty_own_tab_if_needed(&mut self) {
        if self.view != View::Vertical {
            return;
        }
        let Some(plugin_id) = self.plugin_id else {
            return;
        };
        let Some((tab_id, has_user_content)) =
            own_tab_content_state(&self.tabs, &self.panes, plugin_id)
        else {
            return;
        };
        if has_user_content {
            self.tabs_with_user_content.insert(tab_id);
        } else if self.tabs_with_user_content.remove(&tab_id) {
            close_tab_with_id(tab_id as u64);
        }
    }

    fn sync_sidebar_widths(&mut self) {
        let Some(sync) = &self.pending_width_sync else {
            return;
        };
        let observed_widths = sync
            .pane_ids
            .iter()
            .map(|pane_id| (*pane_id, sidebar_geometry(&self.panes, *pane_id)))
            .collect::<Vec<_>>();
        let resize_actions =
            plan_width_sync_attempt(&mut self.pending_width_sync, &observed_widths);
        for (pane_id, resize_strategy) in resize_actions {
            resize_pane_with_id(resize_strategy, PaneId::Plugin(pane_id));
        }
    }

    fn handle_agent_event(&mut self, event: AgentEvent) -> bool {
        if let Some(session_end_timestamp) = self
            .session_end_timestamp_by_pane
            .get(&event.pane_id)
            .copied()
        {
            if event.event != "SessionStart"
                || !event
                    .timestamp
                    .is_some_and(|timestamp| timestamp >= session_end_timestamp)
            {
                return false;
            }
            self.session_end_timestamp_by_pane.remove(&event.pane_id);
        }

        if let Some(timestamp) = event.timestamp {
            if self
                .last_hook_timestamp_by_pane
                .get(&event.pane_id)
                .is_some_and(|previous| timestamp < *previous)
            {
                return false;
            }
            self.last_hook_timestamp_by_pane
                .insert(event.pane_id, timestamp);
        }
        let pane_id = event.pane_id;
        let timestamp = event.timestamp;
        let is_session_end = event.event == "SessionEnd";
        let applied = self.apply_agent_event(event);
        if is_session_end {
            if let Some(timestamp) = timestamp {
                self.session_end_timestamp_by_pane
                    .insert(pane_id, timestamp);
            }
        }
        applied
    }

    fn render_horizontal(&mut self, frame: &mut AnsiFrame, rows: usize, cols: usize) {
        self.visible_horizontal_tabs.clear();
        let session = if self.session_name.is_empty() {
            "Zellij"
        } else {
            &self.session_name
        };
        let mode = format!(" {} ", mode_label(self.mode));
        let session = format!(" {session} ");
        let left = format!("{mode}{session}");
        let (context, clock) = self.right_content();
        let (center_context, right_context, right_clock) = horizontal_content_split(
            self.show_tabs,
            !self.agent_statuses.is_empty(),
            &context,
            &clock,
        );
        let left_width = cell_width(&left).min(cols);
        let available_right = cols.saturating_sub(left_width);
        let max_right = available_right.saturating_mul(2) / 5;
        let clock_width = cell_width(&right_clock).min(available_right);
        let right_width = cell_width(&right_context)
            .saturating_add(cell_width(&right_clock))
            .min(max_right.max(clock_width));

        let mode_width = cell_width(&mode).min(left_width);
        frame.put(
            0,
            0,
            self.colors.mode(self.mode),
            &fit_line(&mode, mode_width),
        );
        if left_width > mode_width {
            frame.put(
                mode_width,
                0,
                self.colors.session,
                &fit_line(&session, left_width - mode_width),
            );
        }
        if right_width > 0 {
            let (context, clock) = fit_right_parts(&right_context, &right_clock, right_width);
            let right_start = cols - right_width;
            frame.put(right_start, 0, self.colors.context, &context);
            frame.put(
                right_start + cell_width(&context),
                0,
                self.colors.clock,
                &clock,
            );
        }

        let center_start = left_width.saturating_add(1);
        let center_end = cols.saturating_sub(right_width.saturating_add(1));
        if center_end > center_start {
            let available = center_end - center_start;
            if !self.agent_statuses.is_empty() {
                let segments = self.agent_status_segments(available);
                let total_width = segments
                    .iter()
                    .map(|(text, _)| cell_width(text))
                    .sum::<usize>()
                    .min(available);
                let mut x = center_start + available.saturating_sub(total_width) / 2;
                for (text, style) in segments {
                    frame.put(
                        x,
                        0,
                        style,
                        &truncate_line(&text, center_end.saturating_sub(x)),
                    );
                    x += cell_width(&text);
                }
            } else if let Some(context) = center_context {
                let rendered = fit_line(&context, available);
                let width = cell_width(rendered.trim_end());
                let x = center_start + available.saturating_sub(width) / 2;
                frame.put(x, 0, self.colors.context, rendered.trim_end());
            } else {
                self.render_horizontal_tabs(frame, cols, center_start, center_end);
            }
        }

        if rows > 1 && self.border_enabled {
            frame.put(
                0,
                1,
                self.colors.border,
                &repeat_pattern_to_width(&self.border_char, cols),
            );
        }
    }

    fn render_horizontal_tabs(
        &mut self,
        frame: &mut AnsiFrame,
        cols: usize,
        left_bound: usize,
        right_bound: usize,
    ) {
        let width = right_bound.saturating_sub(left_bound);
        let active = self.tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let candidates = horizontal_visible_indices(&self.tabs, active, width);
        let mut remaining = width;
        let mut rendered_tabs = Vec::with_capacity(candidates.len());
        for index in candidates {
            if remaining == 0 {
                break;
            }
            let rendered = truncate_line(&tab_label(&self.tabs[index]), remaining.min(24));
            remaining = remaining.saturating_sub(cell_width(&rendered));
            rendered_tabs.push((index, rendered));
        }
        let total_width = rendered_tabs
            .iter()
            .map(|(_, rendered)| cell_width(rendered))
            .sum();
        let mut x = horizontal_group_start(cols, total_width, left_bound, right_bound);
        for (index, rendered) in rendered_tabs {
            let tab = &self.tabs[index];
            let rendered_width = cell_width(&rendered);
            let style = if tab.active {
                self.colors.tab_active
            } else {
                self.colors.tab_normal
            };
            frame.put(x, 0, style, &rendered);
            self.visible_horizontal_tabs.push(TabHitbox {
                start: x,
                end: x + rendered_width,
                position: tab.position,
            });
            x += rendered_width;
        }
    }

    fn render_vertical(&mut self, frame: &mut AnsiFrame, rows: usize, cols: usize) {
        self.visible_vertical_tabs.clear();
        self.agent_focus_targets.clear();
        let separator = vertical_separator_content(
            rows,
            cols,
            self.vertical_separator_enabled,
            &self.vertical_separator_char,
        );
        let content_cols = cols.saturating_sub(usize::from(separator.is_some()));
        if let Some((x, lines)) = separator {
            for (y, line) in lines.iter().enumerate() {
                frame.put(x, y, self.colors.border, line);
            }
        }
        if rows < 2 || content_cols == 0 {
            return;
        }

        let agents = self.agent_statuses.len();
        // Agents are why this sidebar exists, so they claim their rows first and
        // tabs fill the rest, always including the active tab.
        let agent_budget = if agents == 0 {
            0
        } else {
            (2 + agents * 2).min((rows * 3 / 5).max(4))
        };
        let tab_budget = rows.saturating_sub(agent_budget);

        let y = self.render_vertical_tabs(frame, tab_budget, content_cols);
        if agents > 0 {
            self.render_vertical_agents(frame, y, rows, content_cols);
        }
    }

    /// Terminal panes across every tab; plugin panes are chrome, not content.
    fn pane_total(&self) -> usize {
        self.panes
            .panes
            .values()
            .flatten()
            .filter(|pane| !pane.is_plugin && !pane.is_suppressed)
            .count()
    }

    /// Tab cards: index and name on the first row, working directory below.
    fn render_vertical_tabs(
        &mut self,
        frame: &mut AnsiFrame,
        budget: usize,
        content_cols: usize,
    ) -> usize {
        if budget < 3 || self.tabs.is_empty() {
            return 0;
        }
        let dim = Style {
            fg: self.colors.cwd_normal.fg,
            bg: self.colors.background,
            bold: false,
        };
        // The section name is the row's heading; its totals stay quiet beside it.
        let heading = Style { bold: true, ..dim };
        frame.put(0, 0, dim, &fit_line("", content_cols));
        frame.put(0, 0, heading, " Tabs");
        // The counts say how much of the session is off screen when the window
        // shows only part of it.
        let totals = tab_totals_label(
            self.tabs.len(),
            self.pane_total(),
            content_cols.saturating_sub(cell_width(" Tabs") + 2),
        );
        frame.put(
            content_cols.saturating_sub(cell_width(&totals) + 1),
            0,
            dim,
            &totals,
        );

        let active_index = self.tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let window = vertical_tab_window(self.tabs.len(), active_index, (budget - 1) / 2);

        let mut y = 1;
        for index in window {
            if y + 2 > budget {
                break;
            }
            let tab = &self.tabs[index];
            let (title_style, cwd_style) = vertical_styles(&self.colors, tab.active);
            // The card's second row is supporting detail: the active tab marks
            // itself with the marker and color, not with a second bold row.
            let cwd_style = Style {
                bold: false,
                ..cwd_style
            };
            let marker = if tab.active { "▸" } else { " " };
            let repo = self.repo_by_tab.get(&tab.position);
            let title = fit_line(
                &format!(
                    "{marker}{} {}",
                    tab.position + 1,
                    tab_display_name(tab, repo)
                ),
                content_cols,
            );
            let cwd = self
                .cwd_by_tab
                .get(&tab.position)
                .map(|path| display_path(path, self.configured_home.as_deref()));
            let detail = tab_detail_line(repo, cwd.as_deref());
            let cwd = fit_line(&format!("   {detail}"), content_cols);
            frame.put(0, y, title_style, &title);
            frame.put(0, y + 1, cwd_style, &cwd);
            self.visible_vertical_tabs.push((y, tab.position));
            y += 2;
        }
        y
    }

    /// Agent cards: state dot with location and agent name, then a dim row with
    /// the state, how long it has held and the current task.
    fn render_vertical_agents(
        &mut self,
        frame: &mut AnsiFrame,
        start: usize,
        rows: usize,
        content_cols: usize,
    ) -> usize {
        let dim = Style {
            fg: self.colors.cwd_normal.fg,
            bg: self.colors.background,
            bold: false,
        };
        let mut y = start;
        if y + 3 > rows {
            return y;
        }
        frame.put(
            0,
            y,
            self.colors.border,
            &repeat_pattern_to_width("─", content_cols),
        );
        y += 1;

        let entries = self.agent_entries();
        frame.put(0, y, dim, &fit_line("", content_cols));
        frame.put(0, y, Style { bold: true, ..dim }, " Agents");
        let count = agent_totals_label(
            entries.len(),
            content_cols.saturating_sub(cell_width(" Agents") + 2),
        );
        frame.put(
            content_cols.saturating_sub(cell_width(&count) + 1),
            y,
            dim,
            &count,
        );
        y += 1;

        for entry in entries {
            if y + 2 > rows {
                break;
            }
            let focused = self.focused_terminal_pane == Some(entry.pane_id);
            let (name_style, detail_style) = vertical_styles(&self.colors, focused);
            // A focused card is highlighted across both of its rows, the way a
            // tab card is; painting only the marker would leave a stray block.
            let row_bg = if focused {
                name_style.bg
            } else {
                self.colors.background
            };
            let accent = readable_on(self.state_accent(entry.state), row_bg, name_style.fg);
            let blank = fit_line("", content_cols);
            for row in [y, y + 1] {
                frame.put(
                    0,
                    row,
                    Style {
                        fg: name_style.fg,
                        bg: row_bg,
                        bold: false,
                    },
                    &blank,
                );
            }
            if focused {
                frame.put(
                    0,
                    y,
                    Style {
                        fg: name_style.fg,
                        bg: row_bg,
                        bold: true,
                    },
                    "▸",
                );
            }
            frame.put(
                1,
                y,
                Style {
                    fg: accent,
                    bg: row_bg,
                    bold: entry.state.urgent(),
                },
                &entry.state.glyph().to_string(),
            );
            frame.put(
                3,
                y,
                Style {
                    fg: name_style.fg,
                    bg: row_bg,
                    bold: true,
                },
                &truncate_line(&entry.name, content_cols.saturating_sub(3)),
            );
            self.agent_focus_targets.push((y, 0, entry.pane_id));

            let state_text = entry.state.label();
            frame.put(
                3,
                y + 1,
                Style {
                    fg: accent,
                    bg: row_bg,
                    bold: false,
                },
                &truncate_line(state_text, content_cols.saturating_sub(3)),
            );
            let used = 3 + cell_width(state_text);
            if let Some(detail) = &entry.detail {
                let room = content_cols.saturating_sub(used);
                if room >= 4 {
                    let detail_style = Style {
                        fg: if focused { detail_style.fg } else { dim.fg },
                        bg: row_bg,
                        bold: false,
                    };
                    frame.put(
                        used,
                        y + 1,
                        detail_style,
                        &truncate_line(&format!(" · {detail}"), room),
                    );
                }
            }
            self.agent_focus_targets.push((y + 1, 0, entry.pane_id));
            y += 2;
        }
        y
    }

    /// One presentable card per tracked agent, ordered by tab then pane.
    fn agent_entries(&self) -> Vec<AgentEntry> {
        let now = unix_seconds();
        self.sorted_agent_statuses()
            .into_iter()
            .map(|status| {
                let name = match self.pane_location(status.pane_id) {
                    Some((tab, pane)) => format!("{}·{} {}", tab + 1, pane + 1, status.source),
                    None => status.source.clone(),
                };
                let elapsed = elapsed_label(now.saturating_sub(status.since));
                // What the agent is doing right now beats the task it is doing
                // it for; the task returns to the card once the tool call ends.
                let task = status
                    .detail
                    .clone()
                    .or_else(|| status.summary.clone())
                    .or_else(|| self.agent_title_suffix(status.pane_id));
                let detail = match task {
                    Some(task) => Some(format!("{elapsed} · {task}")),
                    None => Some(elapsed),
                };
                AgentEntry {
                    pane_id: status.pane_id,
                    state: status.state,
                    name,
                    detail,
                }
            })
            .collect()
    }

    fn state_accent(&self, state: AgentState) -> Rgb {
        match state {
            AgentState::Blocked => self.agent_accent(true),
            AgentState::Thinking | AgentState::Working | AgentState::Compacting => {
                self.agent_accent(false)
            }
            AgentState::Done => self.colors.context.fg,
            AgentState::Idle => self.colors.cwd_normal.fg,
        }
    }

    fn agent_accent(&self, urgent: bool) -> Rgb {
        let style = if urgent {
            self.colors.agent_urgent
        } else {
            self.colors.agent
        };
        if style.bg == self.colors.background {
            style.fg
        } else {
            style.bg
        }
    }

    fn agent_title_suffix(&self, pane_id: u32) -> Option<String> {
        let title = self
            .panes
            .panes
            .values()
            .flatten()
            .find(|pane| pane.id == pane_id && !pane.is_plugin)?
            .title
            .trim()
            .to_string();
        match title.as_str() {
            "" | "zsh" | "bash" | "fish" | "sh" | "nu" | "Terminal" => None,
            _ => Some(title),
        }
    }

    fn agent_rows_for_tab(&self, tab_position: usize) -> Vec<(usize, u32, &AgentStatus)> {
        self.panes
            .panes
            .get(&tab_position)
            .into_iter()
            .flatten()
            .filter(|pane| !pane.is_plugin && !pane.is_suppressed)
            .enumerate()
            .filter_map(|(index, pane)| {
                self.agent_statuses
                    .get(&pane.id)
                    .map(|status| (index, pane.id, status))
            })
            .collect()
    }

    fn apply_agent_event(&mut self, event: AgentEvent) -> bool {
        let tool = tool_detail(&event.source, event.tool.clone());
        let (state, detail, lifetime) = match event.event.as_str() {
            "SessionStart" => (AgentState::Idle, None, None),
            "UserPromptSubmit" => (AgentState::Thinking, None, Some(AGENT_STALL_SECONDS)),
            "PreToolUse" => (AgentState::Working, tool.clone(), Some(AGENT_STALL_SECONDS)),
            "PostToolUse" => (AgentState::Working, tool.clone(), Some(AGENT_STALL_SECONDS)),
            "PostToolUseFailure" => (AgentState::Working, tool.clone(), Some(AGENT_STALL_SECONDS)),
            // Compaction blocks the agent's own turn, so it outlives an ordinary
            // tool call; `tool` carries the trigger (auto or manual) when sent.
            "PreCompact" => (
                AgentState::Compacting,
                tool.clone(),
                Some(AGENT_STALL_SECONDS),
            ),
            "PostCompact" => (AgentState::Thinking, None, Some(AGENT_STALL_SECONDS)),
            "PermissionRequest" => (AgentState::Blocked, tool.clone(), None),
            "Notification" => notification_state(event.tool.as_deref()),
            "SubagentStart" => (
                AgentState::Working,
                Some("subagent".to_string()),
                Some(AGENT_STALL_SECONDS),
            ),
            "SubagentStop" => (
                AgentState::Working,
                Some("subagent".to_string()),
                Some(AGENT_STALL_SECONDS),
            ),
            "Stop" => (AgentState::Done, None, None),
            "StopFailure" => (AgentState::Idle, None, None),
            "SessionEnd" => return self.agent_statuses.remove(&event.pane_id).is_some(),
            _ => return false,
        };
        self.agent_sequence = self.agent_sequence.wrapping_add(1);
        let previous = self.agent_statuses.get(&event.pane_id);
        let summary = event
            .summary
            .or_else(|| previous.and_then(|status| status.summary.clone()));
        // Keep the clock running while an agent stays in the same state, so the
        // sidebar can show how long it has been blocked or working.
        let since = previous
            .filter(|status| status.state == state)
            .map_or_else(unix_seconds, |status| status.since);
        self.agent_statuses.insert(
            event.pane_id,
            AgentStatus {
                pane_id: event.pane_id,
                source: event.source,
                state,
                detail,
                summary,
                since,
                sequence: self.agent_sequence,
                expires_at: lifetime.map(|seconds| unix_seconds().saturating_add(seconds)),
                updated_at: unix_seconds(),
                detected: false,
                clear_on_focus: event.event == "Stop",
            },
        );
        true
    }

    fn debug_snapshot(&self) -> String {
        let view = match self.view {
            View::Horizontal => "horizontal",
            View::Vertical => "vertical",
        };
        let statuses = self
            .sorted_agent_statuses()
            .iter()
            .map(|status| {
                serde_json::json!({
                    "pane_id": status.pane_id,
                    "message": status.message(),
                    "state": status.state.label(),
                    "summary": status.summary,
                    "urgent": status.urgent(),
                    "detected": status.detected,
                    "location": self.pane_location(status.pane_id),
                })
            })
            .collect::<Vec<_>>();
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                serde_json::json!({
                    "position": tab.position,
                    "name": tab_name(tab),
                    "cwd": self.cwd_by_tab.get(&tab.position).map(|cwd| cwd.display().to_string()),
                    "selected_pane": self.selected_pane_for_tab(tab).map(|pane| format!("{pane:?}")),
                    "repo": self.repo_by_tab.get(&tab.position).map(|repo| {
                        serde_json::json!({
                            "repository": repo.repository,
                            "branch": repo.branch,
                            "worktree": repo.worktree,
                        })
                    }),
                    "rows": self
                        .agent_rows_for_tab(tab.position)
                        .iter()
                        .map(|(index, pane_id, status)| {
                            serde_json::json!({
                                "pane_index": index,
                                "pane_id": pane_id,
                                "message": status.message(),
                            })
                        })
                        .collect::<Vec<_>>(),
                    "terminal_panes": self
                        .panes
                        .panes
                        .get(&tab.position)
                        .map(|panes| {
                            panes
                                .iter()
                                .filter(|pane| !pane.is_plugin && !pane.is_suppressed)
                                .map(|pane| {
                                    serde_json::json!({
                                        "id": pane.id,
                                        "title": pane.title,
                                        "command": pane.terminal_command,
                                        "exited": pane.exited,
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "view": view,
            "cwd_error": self.cwd_error,
            "focused_terminal_pane": self.focused_terminal_pane,
            "plugin_id": self.plugin_id,
            "permissions_granted": self.permissions_granted,
            "agent_statuses": statuses,
            "tabs": tabs,
        })
        .to_string()
    }

    fn sorted_agent_statuses(&self) -> Vec<&AgentStatus> {
        let mut statuses: Vec<&AgentStatus> = self.agent_statuses.values().collect();
        statuses.sort_by_key(|status| {
            let location = || self.pane_location(status.pane_id);
            (
                location().map_or(usize::MAX, |(tab, _)| tab),
                location().map_or(usize::MAX, |(_, pane)| pane),
                std::cmp::Reverse(status.sequence),
            )
        });
        statuses
    }

    fn agent_entry_message(&self, status: &AgentStatus) -> String {
        match self.pane_location(status.pane_id) {
            Some((tab_position, pane_index)) => {
                format!(
                    "[{}·{}] {}",
                    tab_position + 1,
                    pane_index + 1,
                    status.message()
                )
            }
            None => status.message(),
        }
    }

    fn agent_style(&self, urgent: bool) -> Style {
        if urgent {
            self.colors.agent_urgent
        } else {
            self.colors.agent
        }
    }

    fn agent_status_segments(&self, available: usize) -> Vec<(String, Style)> {
        const SEPARATOR: &str = " | ";
        if available == 0 || self.agent_statuses.is_empty() {
            return Vec::new();
        }
        let statuses = self.sorted_agent_statuses();
        let separator_width = cell_width(SEPARATOR);
        let mut segments: Vec<(String, Style)> = Vec::new();
        let mut used = 0;
        let mut included = 0;

        for (index, status) in statuses.iter().enumerate() {
            let entry = self.agent_entry_message(status);
            let entry_width = cell_width(&entry);
            let following = statuses.len() - index - 1;
            let reserved = if following > 0 {
                separator_width + cell_width(&format!(" +{following}"))
            } else {
                0
            };
            let separator_cost = if segments.is_empty() {
                0
            } else {
                separator_width
            };
            if !segments.is_empty() && used + separator_cost + entry_width + reserved > available {
                break;
            }

            let (text, width) = if used + separator_cost + entry_width > available {
                let remaining = available.saturating_sub(used + separator_cost);
                let truncated = truncate_line(&entry, remaining);
                let width = cell_width(&truncated);
                (truncated, width)
            } else {
                (entry, entry_width)
            };
            if width == 0 {
                break;
            }
            if !segments.is_empty() {
                let inherited = segments
                    .last()
                    .map_or_else(|| self.agent_style(status.urgent()), |(_, style)| *style);
                segments.push((SEPARATOR.to_string(), inherited));
                used += separator_width;
            }
            segments.push((text, self.agent_style(status.urgent())));
            used += width;
            included += 1;
            if width < entry_width {
                break;
            }
        }

        let hidden = statuses.len() - included;
        if hidden > 0 && !segments.is_empty() {
            let marker = format!(" +{hidden}");
            if used + separator_width + cell_width(&marker) <= available {
                let separator_style = segments
                    .last()
                    .map_or_else(|| self.agent_style(false), |(_, style)| *style);
                segments.push((SEPARATOR.to_string(), separator_style));
                segments.push((marker, self.agent_style(statuses[included].urgent())));
            }
        }
        segments
    }

    fn pane_location(&self, pane_id: u32) -> Option<(usize, usize)> {
        for (tab_position, panes) in &self.panes.panes {
            let index = panes
                .iter()
                .filter(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
                .position(|pane| pane.id == pane_id);
            if let Some(index) = index {
                return Some((*tab_position, index));
            }
        }
        None
    }

    fn track_focused_pane(&mut self) {
        // Every tab reports a focused pane of its own, so only the active tab's
        // says where the user actually is.
        let active_tab = self
            .tabs
            .iter()
            .find(|tab| tab.active)
            .map(|tab| tab.position);
        let focused = active_tab
            .and_then(|position| self.panes.panes.get(&position))
            .into_iter()
            .flatten()
            .find(|pane| pane.is_focused && !pane.is_plugin && !pane.exited)
            .map(|pane| pane.id);
        if focused == self.focused_terminal_pane {
            return;
        }
        self.focused_terminal_pane = focused;
        if let Some(pane_id) = focused {
            let clear = self
                .agent_statuses
                .get(&pane_id)
                .is_some_and(|status| status.clear_on_focus);
            if clear {
                self.agent_statuses.remove(&pane_id);
            }
        }
    }

    fn detect_agents_from_manifest(&mut self) {
        // A pane that no longer exists cannot host an agent any more.
        let live: HashSet<u32> = self
            .panes
            .panes
            .values()
            .flatten()
            .filter(|pane| !pane.is_plugin)
            .map(|pane| pane.id)
            .collect();
        if !live.is_empty() {
            self.agent_statuses
                .retain(|pane_id, _| live.contains(pane_id));
        }
        let running: Vec<(u32, Vec<String>)> = self
            .panes
            .panes
            .values()
            .flatten()
            .filter(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
            .filter_map(|pane| {
                pane.terminal_command.as_deref().map(|command| {
                    (
                        pane.id,
                        command.split_whitespace().map(str::to_string).collect(),
                    )
                })
            })
            .collect();
        for (pane_id, command) in running {
            self.update_detected_agent(pane_id, &command, true);
        }
    }

    fn update_detected_agent(&mut self, pane_id: u32, command: &[String], is_foreground: bool) {
        if !is_foreground {
            return;
        }
        let label = command
            .first()
            .and_then(|arg| arg.rsplit('/').next())
            .map(|name| name.to_ascii_lowercase())
            .and_then(|name| detected_agent_label(&name));
        match label {
            Some(label) => {
                if self.agent_statuses.contains_key(&pane_id) {
                    return;
                }
                self.agent_sequence = self.agent_sequence.wrapping_add(1);
                self.agent_statuses.insert(
                    pane_id,
                    AgentStatus {
                        pane_id,
                        source: label.to_string(),
                        state: AgentState::Idle,
                        detail: None,
                        summary: None,
                        since: unix_seconds(),
                        sequence: self.agent_sequence,
                        expires_at: None,
                        updated_at: unix_seconds(),
                        detected: true,
                        clear_on_focus: false,
                    },
                );
            }
            None => {
                let detected = self
                    .agent_statuses
                    .get(&pane_id)
                    .is_some_and(|status| status.detected);
                if detected {
                    self.agent_statuses.remove(&pane_id);
                }
            }
        }
    }

    fn right_content(&self) -> (String, String) {
        let clock = format!(
            "  {} ",
            current_time(self.timezone_offset_hours, &self.datetime_format)
        );
        let command = self.active_command.as_deref().unwrap_or("shell");
        if let Some(git) = &self.git_context {
            let dirty = if git.dirty { "*" } else { "" };
            return (
                format!(
                    " {}   {}{} · {}",
                    git.repository, git.branch, dirty, command
                ),
                clock,
            );
        }
        let location = self
            .active_cwd
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("workspace");
        (format!(" {location} · {command}"), clock)
    }

    fn refresh_active_pane(&mut self) {
        let Some(active_tab) = self.tabs.iter().find(|tab| tab.active) else {
            return;
        };
        let Some(pane_id) = self.selected_pane_for_tab(active_tab) else {
            return;
        };
        let pane = self
            .panes
            .panes
            .get(&active_tab.position)
            .and_then(|panes| {
                panes
                    .iter()
                    .find(|pane| PaneId::Terminal(pane.id) == pane_id)
            });
        self.active_command = pane
            .and_then(|pane| pane.terminal_command.as_deref())
            .map(|command| command_label(&[command.to_string()]));
        if self.active_pane_id != Some(pane_id) {
            self.active_pane_id = Some(pane_id);
            if let Some(cwd) = self
                .permissions_granted
                .then(|| get_pane_cwd(pane_id))
                .and_then(Result::ok)
            {
                self.update_cwd(cwd);
            }
        }
    }

    /// Zellij delivers application-state events only to a permitted plugin, and a
    /// grant restored from its permission cache arrives without any
    /// `PermissionRequestResult`. Receiving such an event is therefore the only
    /// proof of that grant.
    fn note_permissions_granted(&mut self) {
        if self.permissions_granted {
            return;
        }
        self.permissions_granted = true;
        set_selectable(view_selectable(self.view));
    }

    /// The shared file is the handover point between sidebars: whoever sees an
    /// event writes it, and an instance that starts later reads it instead of
    /// waiting for every agent to speak again.
    fn sync_path(&self) -> Option<String> {
        agent_sync_path(&self.session_name)
    }

    fn persist_agent_statuses(&mut self) {
        let Some(path) = self.sync_path() else {
            return;
        };
        let payload = encode_agent_statuses(&self.agent_statuses);
        if self.agent_sync_payload.as_deref() == Some(payload.as_str()) {
            return;
        }
        // Written beside the target and renamed, so a sidebar reading at the
        // same moment sees either the old file or the new one, never half of it.
        let staging = format!("{}.{}.staging", path, self.plugin_id.unwrap_or(0));
        if std::fs::write(&staging, &payload).is_ok() && std::fs::rename(&staging, &path).is_ok() {
            self.agent_sync_payload = Some(payload);
        } else {
            let _ = std::fs::remove_file(&staging);
        }
    }

    fn hydrate_agent_statuses(&mut self) -> bool {
        let Some(path) = self.sync_path() else {
            return false;
        };
        let Ok(payload) = std::fs::read_to_string(&path) else {
            return false;
        };
        if self.agent_sync_payload.as_deref() == Some(payload.as_str()) {
            return false;
        }
        let incoming = decode_agent_statuses(&payload);
        self.agent_sync_payload = Some(payload);
        self.merge_agent_statuses(incoming)
    }

    fn merge_agent_statuses(&mut self, incoming: Vec<AgentStatus>) -> bool {
        // A pane this instance knows nothing about is not resurrected: the
        // manifest, not the file, decides which panes still exist.
        let live: HashSet<u32> = self
            .panes
            .panes
            .values()
            .flatten()
            .filter(|pane| !pane.is_plugin)
            .map(|pane| pane.id)
            .collect();
        let mut changed = false;
        for mut status in incoming {
            if !live.is_empty() && !live.contains(&status.pane_id) {
                continue;
            }
            if self
                .session_end_timestamp_by_pane
                .contains_key(&status.pane_id)
            {
                continue;
            }
            let stale = self
                .agent_statuses
                .get(&status.pane_id)
                .is_some_and(|known| !known.detected && known.updated_at >= status.updated_at);
            if stale {
                continue;
            }
            self.agent_sequence = self.agent_sequence.wrapping_add(1);
            status.sequence = self.agent_sequence;
            self.agent_statuses.insert(status.pane_id, status);
            changed = true;
        }
        changed
    }

    fn refresh_cwds(&mut self) {
        if !self.permissions_granted {
            return;
        }
        let pane_ids: Vec<(usize, PaneId)> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                self.selected_pane_for_tab(tab)
                    .map(|pane| (tab.position, pane))
            })
            .collect();
        for (tab_position, pane_id) in pane_ids {
            match get_pane_cwd(pane_id) {
                Ok(cwd) => {
                    self.cwd_by_tab.insert(tab_position, cwd);
                }
                Err(error) => self.cwd_error = Some(format!("{pane_id:?}: {error}")),
            }
        }
        self.refresh_repositories();
    }

    /// Repository, branch and worktree per tab. Each tab's working directory is
    /// resolved by one git invocation whose result arrives asynchronously.
    fn refresh_repositories(&mut self) {
        if !self.permissions_granted {
            return;
        }
        let pending: Vec<(usize, PathBuf)> = self
            .cwd_by_tab
            .iter()
            .filter(|(position, cwd)| {
                self.repo_cwd_by_tab
                    .get(*position)
                    .is_none_or(|known| known != *cwd)
            })
            .map(|(position, cwd)| (*position, cwd.clone()))
            .collect();
        for (position, cwd) in pending {
            self.repo_cwd_by_tab.insert(position, cwd.clone());
            let mut context = BTreeMap::new();
            context.insert("kind".to_string(), "repo".to_string());
            context.insert("tab".to_string(), position.to_string());
            run_command_with_env_variables_and_cwd(
                &["sh", "-c", REPO_COMMAND],
                BTreeMap::new(),
                cwd,
                context,
            );
        }
    }

    fn selected_pane_for_tab(&self, tab: &TabInfo) -> Option<PaneId> {
        let panes = self.panes.panes.get(&tab.position)?;
        let visible_floating = tab.are_floating_panes_visible;
        let pane = panes
            .iter()
            .filter(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
            .find(|pane| pane.is_focused && pane.is_floating == visible_floating)
            .or_else(|| {
                panes
                    .iter()
                    .filter(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
                    .find(|pane| pane.is_focused)
            })
            .or_else(|| {
                panes
                    .iter()
                    .find(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
            })?;
        Some(PaneId::Terminal(pane.id))
    }

    fn tab_for_selected_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.tabs.iter().find_map(|tab| {
            (self.selected_pane_for_tab(tab) == Some(pane_id)).then_some(tab.position)
        })
    }

    fn update_cwd(&mut self, cwd: PathBuf) {
        if self.active_cwd.as_ref() != Some(&cwd) {
            self.active_cwd = Some(cwd);
            self.git_context = None;
            self.refresh_git();
        }
    }

    fn refresh_git(&mut self) {
        if self.view != View::Horizontal || !self.permissions_granted || self.git_refresh_pending {
            return;
        }
        let Some(cwd) = self.active_cwd.clone() else {
            return;
        };
        let mut context = BTreeMap::new();
        context.insert("cwd".to_string(), cwd.display().to_string());
        self.git_refresh_pending = true;
        run_command_with_env_variables_and_cwd(
            &["sh", "-c", GIT_COMMAND],
            BTreeMap::new(),
            cwd,
            context,
        );
    }
}

fn parse_agent_event(payload: &str) -> Option<AgentEvent> {
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

fn agent_label(source: &str) -> String {
    match source {
        "choco-pi" => "choco-pi",
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        other => other,
    }
    .to_string()
}

fn detected_agent_label(command: &str) -> Option<&'static str> {
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

fn horizontal_visible_indices(tabs: &[TabInfo], active: usize, width: usize) -> Vec<usize> {
    if tabs.is_empty() || width == 0 {
        return Vec::new();
    }
    let mut selected = vec![active.min(tabs.len() - 1)];
    let mut used = cell_width(&tab_label(&tabs[selected[0]])).min(24);
    let mut distance = 1;
    while selected.len() < tabs.len() {
        let mut added = false;
        for candidate in [active.checked_sub(distance), active.checked_add(distance)]
            .into_iter()
            .flatten()
        {
            if candidate >= tabs.len() || selected.contains(&candidate) {
                continue;
            }
            let candidate_width = cell_width(&tab_label(&tabs[candidate])).min(24);
            if used + candidate_width <= width {
                selected.push(candidate);
                used += candidate_width;
                added = true;
            }
        }
        if !added && active.saturating_add(distance) >= tabs.len() && distance > active {
            break;
        }
        distance += 1;
    }
    selected.sort_unstable();
    selected
}

fn tab_label(tab: &TabInfo) -> String {
    let bell = if tab.has_bell_notification || tab.is_flashing_bell {
        " ●"
    } else {
        ""
    };
    format!(" {} {}{} ", tab.position + 1, tab_name(tab), bell)
}

fn tab_name(tab: &TabInfo) -> String {
    if tab.name.is_empty() {
        format!("Tab {}", tab.position + 1)
    } else {
        tab.name.clone()
    }
}

fn display_path(path: &Path, configured_home: Option<&Path>) -> String {
    let home = configured_home
        .filter(|home| path.starts_with(home))
        .map(Path::to_path_buf)
        .or_else(|| inferred_home(path));
    if let Some(relative) = home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        if relative.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

fn inferred_home(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    let root = components.next()?;
    let base = components.next()?.as_os_str();
    let user = components.next()?.as_os_str();
    if root == std::path::Component::RootDir && (base == "Users" || base == "home") {
        let mut home = PathBuf::from("/");
        home.push(base);
        home.push(user);
        Some(home)
    } else {
        None
    }
}

fn parse_git_context(stdout: &[u8], cwd: PathBuf) -> Option<GitContext> {
    let output = String::from_utf8_lossy(stdout);
    let mut lines = output.lines();
    let root = PathBuf::from(lines.next()?);
    let status = lines.next()?.strip_prefix("## ")?;
    let branch_status = status.strip_prefix("No commits yet on ").unwrap_or(status);
    let branch = branch_status
        .split_once("...")
        .map(|(branch, _)| branch)
        .unwrap_or_else(|| {
            branch_status
                .split_whitespace()
                .next()
                .unwrap_or(branch_status)
        });
    let dirty = lines.next().is_some();
    Some(GitContext {
        cwd,
        repository: root.file_name()?.to_str()?.to_string(),
        branch: branch.to_string(),
        dirty,
    })
}

fn command_label(command: &[String]) -> String {
    command
        .first()
        .and_then(|command| Path::new(command).file_name())
        .and_then(|command| command.to_str())
        .unwrap_or("shell")
        .to_string()
}

fn mode_label(mode: InputMode) -> String {
    format!("{mode:?}").to_uppercase()
}

fn subscribe_to_events() {
    subscribe(&[
        EventType::ModeUpdate,
        EventType::TabUpdate,
        EventType::PaneUpdate,
        EventType::CwdChanged,
        EventType::CommandChanged,
        EventType::RunCommandResult,
        EventType::PermissionRequestResult,
        EventType::Timer,
        EventType::Mouse,
    ]);
}

fn permissions_for_view(view: View) -> &'static [PermissionType] {
    match view {
        View::Horizontal => &[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadCliPipes,
        ],
        View::Vertical => &[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadCliPipes,
        ],
    }
}

fn view_selectable(_view: View) -> bool {
    false
}

fn width_sync_strategy(
    current_width: usize,
    target_width: usize,
    movable_edge: Direction,
) -> Option<ResizeStrategy> {
    let resize = match current_width.cmp(&target_width) {
        std::cmp::Ordering::Less => Resize::Increase,
        std::cmp::Ordering::Greater => Resize::Decrease,
        std::cmp::Ordering::Equal => return None,
    };
    Some(ResizeStrategy {
        resize,
        direction: Some(movable_edge),
        invert_on_boundaries: false,
    })
}

fn plan_width_sync_attempt(
    pending: &mut Option<PendingWidthSync>,
    observed_widths: &[(u32, Option<(usize, Direction)>)],
) -> Vec<(u32, ResizeStrategy)> {
    let Some(sync) = pending.as_mut() else {
        return Vec::new();
    };
    let target_width = sync.target_width;
    let all_equal = sync.pane_ids.iter().all(|pane_id| {
        observed_widths
            .iter()
            .find(|(observed_id, _)| observed_id == pane_id)
            .and_then(|(_, geometry)| geometry.map(|(width, _)| width))
            == Some(target_width)
    });
    if all_equal {
        *pending = None;
        return Vec::new();
    }

    let mut actions = Vec::new();
    for (pane_id, geometry) in observed_widths {
        if !sync.pane_ids.contains(pane_id) {
            continue;
        }
        let Some((width, movable_edge)) = *geometry else {
            continue;
        };
        if width == target_width {
            sync.last_requested_widths.remove(pane_id);
            continue;
        }
        if sync.last_requested_widths.get(pane_id) == Some(&width) {
            continue;
        }
        if let Some(strategy) = width_sync_strategy(width, target_width, movable_edge) {
            sync.last_requested_widths.insert(*pane_id, width);
            actions.push((*pane_id, strategy));
        }
    }
    sync.attempts_remaining = sync.attempts_remaining.saturating_sub(1);
    if sync.attempts_remaining == 0 {
        *pending = None;
    }
    actions
}

fn visible_vertical_sidebar_ids(panes: &PaneManifest) -> Vec<u32> {
    let mut pane_ids = panes
        .panes
        .values()
        .flatten()
        .filter(|pane| !pane.is_suppressed && is_vertical_sidebar_plugin(pane))
        .map(|pane| pane.id)
        .collect::<Vec<_>>();
    pane_ids.sort_unstable();
    pane_ids.dedup();
    pane_ids
}

fn sidebar_geometry(panes: &PaneManifest, plugin_id: u32) -> Option<(usize, Direction)> {
    panes.panes.values().flatten().find_map(|pane| {
        (pane.id == plugin_id && !pane.is_suppressed && is_vertical_sidebar_plugin(pane)).then_some(
            (
                pane.pane_columns,
                if pane.pane_x == 0 {
                    Direction::Right
                } else {
                    Direction::Left
                },
            ),
        )
    })
}

fn active_sidebar_state(
    tabs: &[TabInfo],
    panes: &PaneManifest,
    plugin_id: u32,
) -> Option<(usize, bool)> {
    let active_position = tabs.iter().find(|tab| tab.active)?.position;
    panes.panes.get(&active_position)?.iter().find_map(|pane| {
        (pane.id == plugin_id && !pane.is_suppressed && is_vertical_sidebar_plugin(pane))
            .then_some((pane.pane_columns, pane.is_focused))
    })
}

fn own_tab_content_state(
    tabs: &[TabInfo],
    panes: &PaneManifest,
    plugin_id: u32,
) -> Option<(usize, bool)> {
    let (tab_position, tab_panes) = panes.panes.iter().find(|(_, tab_panes)| {
        tab_panes
            .iter()
            .any(|pane| pane.id == plugin_id && pane.is_plugin)
    })?;
    let tab_id = tabs
        .iter()
        .find(|tab| tab.position == *tab_position)?
        .tab_id;
    Some((
        tab_id,
        tab_panes.iter().any(|pane| !is_layout_ui_pane(pane)),
    ))
}

fn is_layout_ui_pane(pane: &PaneInfo) -> bool {
    if !pane.is_plugin {
        return false;
    }
    pane.plugin_url.as_deref().is_some_and(|url| {
        url.ends_with(VERTICAL_SIDEBAR_URL_SUFFIX)
            || url.ends_with("/vertical-tabs.wasm")
            || (pane.is_suppressed && url == "zellij:link")
    })
}

fn is_vertical_sidebar_plugin(pane: &PaneInfo) -> bool {
    pane.is_plugin
        && pane
            .plugin_url
            .as_deref()
            .is_some_and(|url| url.ends_with(VERTICAL_SIDEBAR_URL_SUFFIX))
}

fn parse_hex_color(value: &str) -> Option<Rgb> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(Rgb(
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

fn configured_color(configuration: &BTreeMap<String, String>, key: &str, fallback: Rgb) -> Rgb {
    configuration
        .get(key)
        .and_then(|value| parse_hex_color(value))
        .unwrap_or(fallback)
}

fn ansi_style(style: Style) -> String {
    let bold = if style.bold { "1;" } else { "22;" };
    format!(
        "\x1b[{bold}38;2;{};{};{};48;2;{};{};{}m",
        style.fg.0, style.fg.1, style.fg.2, style.bg.0, style.bg.1, style.bg.2
    )
}

fn horizontal_group_start(
    cols: usize,
    total_width: usize,
    left_bound: usize,
    right_bound: usize,
) -> usize {
    let latest_start = right_bound.saturating_sub(total_width).max(left_bound);
    cols.saturating_sub(total_width)
        .checked_div(2)
        .unwrap_or(0)
        .clamp(left_bound, latest_start)
}

/// Repository identity for one tab, resolved with a single git invocation.
const REPO_COMMAND: &str =
    "git rev-parse --show-toplevel --abbrev-ref HEAD --git-dir --git-common-dir";

/// Parse `git rev-parse` output: toplevel, branch, git dir, common dir.
fn parse_repo_info(stdout: &[u8]) -> Option<RepoInfo> {
    let text = String::from_utf8_lossy(stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let toplevel = PathBuf::from(lines.next()?);
    let branch = lines.next()?.to_string();
    let git_dir = PathBuf::from(lines.next()?);
    let common_dir = lines.next().map(PathBuf::from);

    let worktree = git_dir
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| *name == "worktrees")
        .and_then(|_| git_dir.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string);

    // A linked worktree lives beside the repository, so its own directory name
    // would be misleading; name it after the repository the common dir belongs to.
    let repository = match (&worktree, &common_dir) {
        (Some(_), Some(common)) => {
            project_name_for_git_dir(common).or_else(|| directory_name(&toplevel))?
        }
        _ => directory_name(&toplevel)?,
    };

    Some(RepoInfo {
        repository,
        branch: if branch == "HEAD" {
            "detached".to_string()
        } else {
            branch
        },
        worktree,
    })
}

fn directory_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

/// Repository name owning a git directory such as `/repo/.git` or `/repo/.bare`.
fn project_name_for_git_dir(git_dir: &Path) -> Option<String> {
    let mut candidate = git_dir;
    while candidate
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') || name == "worktrees")
    {
        candidate = candidate.parent()?;
    }
    directory_name(candidate)
}

/// Zellij's own tab names carry no meaning, so a repository name may replace them.
fn tab_display_name(tab: &TabInfo, repo: Option<&RepoInfo>) -> String {
    let name = tab_name(tab);
    let generated = name
        .strip_prefix("Tab #")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
    match repo {
        Some(repo) if generated => repo.repository.clone(),
        _ => name,
    }
}

/// Second row of a tab card: branch and worktree when the tab sits in a
/// repository, the working directory otherwise.
fn tab_detail_line(repo: Option<&RepoInfo>, cwd: Option<&str>) -> String {
    match repo {
        Some(repo) => match &repo.worktree {
            Some(worktree) if repo.branch == *worktree => format!("⑂{worktree}"),
            Some(worktree) => format!("{} · ⑂{}", repo.branch, worktree),
            None => repo.branch.clone(),
        },
        None => cwd.unwrap_or("—").to_string(),
    }
}

/// Tabs visible around the active one, two rows per tab.
fn vertical_tab_window(total: usize, active_index: usize, capacity: usize) -> Vec<usize> {
    if total == 0 || capacity == 0 {
        return Vec::new();
    }
    if total <= capacity {
        return (0..total).collect();
    }
    let half = capacity / 2;
    let start = active_index
        .saturating_sub(half)
        .min(total.saturating_sub(capacity));
    (start..start + capacity).collect()
}

/// Compact duration for a state that has been held for `seconds`.
fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        _ => format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// Spelling the units out is worth the room when there is room; a narrow
/// sidebar falls back to the bare counts rather than truncating a word.
fn tab_totals_label(tabs: usize, panes: usize, room: usize) -> String {
    let plural = |count: usize, noun: &str| {
        if count == 1 {
            format!("{count} {noun}")
        } else {
            format!("{count} {noun}s")
        }
    };
    // One pane per tab is the ordinary case, and repeating the same number
    // twice says nothing, so the pane count only appears when it differs.
    if tabs == panes {
        let spelled = plural(tabs, "tab");
        return if cell_width(&spelled) <= room {
            spelled
        } else {
            tabs.to_string()
        };
    }
    let spelled = format!("{} · {}", plural(tabs, "tab"), plural(panes, "pane"));
    if cell_width(&spelled) <= room {
        return spelled;
    }
    format!("{tabs} · {panes}")
}

/// The agent count reads as sessions, matching the tab header's spelled-out
/// totals, and falls back to the bare number when the sidebar is narrow.
fn agent_totals_label(sessions: usize, room: usize) -> String {
    let spelled = if sessions == 1 {
        format!("{sessions} session")
    } else {
        format!("{sessions} sessions")
    };
    if cell_width(&spelled) <= room {
        return spelled;
    }
    sessions.to_string()
}

/// Perceived brightness, per the WCAG relative-luminance formula.
fn relative_luminance(color: Rgb) -> f64 {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
}

fn contrast_ratio(one: Rgb, other: Rgb) -> f64 {
    let (a, b) = (relative_luminance(one), relative_luminance(other));
    let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// A state accent is chosen against the panel background, so on a highlighted
/// card it can land too close to that highlight to read - an idle agent is the
/// usual victim, since its accent is the same muted tone. Fall back to the
/// card's own foreground whenever the accent stops being legible.
fn readable_on(accent: Rgb, background: Rgb, fallback: Rgb) -> Rgb {
    const LEGIBLE_CONTRAST: f64 = 2.5;
    if contrast_ratio(accent, background) >= LEGIBLE_CONTRAST {
        accent
    } else {
        fallback
    }
}

fn vertical_styles(colors: &Colors, active: bool) -> (Style, Style) {
    if active {
        (colors.tab_active, colors.cwd_active)
    } else {
        (colors.tab_normal, colors.cwd_normal)
    }
}

fn validated_datetime_format(configured: Option<&str>) -> String {
    let format = configured.unwrap_or(DEFAULT_DATETIME_FORMAT);
    if StrftimeItems::new(format).any(|item| matches!(item, Item::Error)) {
        DEFAULT_DATETIME_FORMAT.to_string()
    } else {
        format.to_string()
    }
}

fn current_time(offset_hours: i32, format: &str) -> String {
    let timezone = FixedOffset::east_opt(offset_hours * 60 * 60).expect("validated offset");
    Utc::now()
        .with_timezone(&timezone)
        .format(format)
        .to_string()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn printable_character(character: char) -> char {
    if character.is_control() {
        '\u{fffd}'
    } else {
        character
    }
}

fn sanitize_text(value: &str) -> String {
    value.chars().map(printable_character).collect()
}

fn sanitize_and_clip(value: &str, width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    for character in value.chars().map(printable_character) {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current_width + character_width > width {
            break;
        }
        result.push(character);
        current_width += character_width;
    }
    result
}

fn repeat_pattern_to_width(pattern: &str, width: usize) -> String {
    if width == 0
        || !pattern.chars().any(|character| {
            !character.is_control() && UnicodeWidthChar::width(character).unwrap_or(0) > 0
        })
    {
        return " ".repeat(width);
    }

    let pattern = sanitize_text(pattern);
    let pattern_width = cell_width(&pattern);
    if pattern_width == 0 {
        return " ".repeat(width);
    }

    let mut result = String::new();
    let repetitions = width / pattern_width;
    for _ in 0..repetitions {
        result.push_str(&pattern);
    }

    let remaining = width - repetitions * pattern_width;
    let partial = sanitize_and_clip(&pattern, remaining);
    let partial_width = cell_width(&partial);
    result.push_str(&partial);
    result.push_str(&" ".repeat(remaining - partial_width));
    result
}

fn fit_line(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let value = sanitize_text(value);
    let value_width = cell_width(&value);
    if value_width <= width {
        return format!("{value}{}", " ".repeat(width - value_width));
    }
    if width == 1 {
        return "…".to_string();
    }
    let target = width - 1;
    let mut result = String::new();
    let mut current_width = 0;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current_width + character_width > target {
            break;
        }
        result.push(character);
        current_width += character_width;
    }
    result.push('…');
    result
}

fn truncate_line(value: &str, width: usize) -> String {
    if cell_width(value) <= width {
        sanitize_text(value)
    } else {
        fit_line(value, width)
    }
}

fn fit_right_parts(context: &str, clock: &str, width: usize) -> (String, String) {
    let clock_width = cell_width(clock);
    if width <= clock_width {
        return (String::new(), fit_line(clock, width));
    }
    (fit_line(context, width - clock_width), sanitize_text(clock))
}

fn horizontal_content_split(
    show_tabs: bool,
    has_agent_status: bool,
    context: &str,
    clock: &str,
) -> (Option<String>, String, String) {
    if show_tabs {
        return (None, context.to_string(), clock.to_string());
    }

    let center = (!has_agent_status).then(|| context.to_string());
    (center, String::new(), clock.to_string())
}

fn vertical_separator_content(
    rows: usize,
    cols: usize,
    enabled: bool,
    pattern: &str,
) -> Option<(usize, Vec<String>)> {
    if !enabled || rows == 0 || cols == 0 {
        return None;
    }
    let separator = repeat_pattern_to_width(pattern, 1);
    Some((cols - 1, vec![separator; rows]))
}

fn cell_width(value: &str) -> usize {
    value
        .chars()
        .map(printable_character)
        .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviates_macos_home_directory() {
        assert_eq!(
            display_path(Path::new("/Users/Nebuleto/Workspace/choco-pi"), None),
            "~/Workspace/choco-pi"
        );
    }

    #[test]
    fn parses_clean_git_context() {
        let context = parse_git_context(
            b"/Users/Nebuleto/Workspace/choco-pi\n## main...origin/main\n",
            PathBuf::from("/Users/Nebuleto/Workspace/choco-pi"),
        )
        .unwrap();
        assert_eq!(context.repository, "choco-pi");
        assert_eq!(context.branch, "main");
        assert!(!context.dirty);
    }

    #[test]
    fn parses_branch_before_the_first_commit() {
        let context = parse_git_context(
            b"/Users/Nebuleto/Workspace/choco-pi\n## No commits yet on main\n",
            PathBuf::from("/Users/Nebuleto/Workspace/choco-pi"),
        )
        .unwrap();
        assert_eq!(context.branch, "main");
    }

    #[test]
    fn parses_codex_agent_event() {
        let event = parse_agent_event(
            r#"{"source_agent":"codex","hook_event":"PermissionRequest","tool_name":"Bash","pane_id":7,"ts_ms":9}"#,
        )
        .unwrap();
        assert_eq!(event.source, "Codex");
        assert_eq!(event.event, "PermissionRequest");
        assert_eq!(event.tool.as_deref(), Some("Bash"));
        assert_eq!(event.pane_id, 7);
    }

    #[test]
    fn a_failed_turn_leaves_the_agent_idle_rather_than_failed() {
        let mut state = State::default();
        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "StopFailure".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(9),
        }));
        let status = state.agent_statuses.get(&7).unwrap();
        assert!(
            !status.urgent(),
            "only waiting on the user is worth an alarm"
        );
        assert_eq!(status.state, AgentState::Idle);
        assert_eq!(status.message(), "○ Codex idle");

        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "SessionEnd".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(10),
        }));
        assert!(state.agent_statuses.is_empty());
    }

    #[test]
    fn events_map_onto_the_shared_state_vocabulary() {
        let mut state = State::default();
        for (event, tool, expected, glyph) in [
            ("SessionStart", None, AgentState::Idle, '○'),
            ("UserPromptSubmit", None, AgentState::Thinking, '●'),
            ("PreToolUse", Some("Bash"), AgentState::Working, '●'),
            ("PermissionRequest", Some("Bash"), AgentState::Blocked, '◉'),
            (
                "Notification",
                Some("needs your permission to use Bash"),
                AgentState::Blocked,
                '◉',
            ),
            // A bare notification means the turn ended, not that it stalled.
            ("Notification", None, AgentState::Done, '✓'),
            ("Stop", None, AgentState::Done, '✓'),
            // Neither a failing tool call nor a failed turn is a state of its own.
            ("PostToolUseFailure", Some("Bash"), AgentState::Working, '●'),
            ("StopFailure", None, AgentState::Idle, '○'),
        ] {
            assert!(state.apply_agent_event(AgentEvent {
                source: "choco-pi".to_string(),
                event: event.to_string(),
                tool: tool.map(str::to_string),
                summary: None,
                pane_id: 7,
                timestamp: Some(1),
            }));
            let status = state.agent_statuses.get(&7).unwrap();
            assert_eq!(status.state, expected, "event {event}");
            assert_eq!(status.state.glyph(), glyph, "event {event}");
            assert_eq!(status.urgent(), expected.urgent(), "event {event}");
        }
    }

    #[test]
    fn multiple_agent_sessions_are_tracked_per_pane() {
        let mut state = State::default();
        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Bash".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));
        assert!(state.apply_agent_event(AgentEvent {
            source: "Claude Code".to_string(),
            event: "Notification".to_string(),
            tool: Some("Claude needs your permission to use Edit".to_string()),
            summary: None,
            pane_id: 12,
            timestamp: Some(2),
        }));
        assert_eq!(state.agent_statuses.len(), 2);

        let segments = state.agent_status_segments(200);
        assert_eq!(segments.len(), 3);
        assert_eq!(
            segments[0].0,
            "◉ Claude Code blocked · Claude needs your permission to use Edit"
        );
        assert_eq!(segments[0].1, state.colors.agent_urgent);
        assert_eq!(segments[1].0, " | ");
        assert_eq!(segments[2].0, "● Codex working · Bash");
        assert_eq!(segments[2].1, state.colors.agent);

        assert!(state.apply_agent_event(AgentEvent {
            source: "Claude Code".to_string(),
            event: "SessionEnd".to_string(),
            tool: None,
            summary: None,
            pane_id: 12,
            timestamp: Some(3),
        }));
        assert_eq!(state.agent_statuses.len(), 1);
    }

    #[test]
    fn agent_cards_carry_location_state_and_task() {
        let mut state = State::default();
        for (tab, pane_id) in [(0, 7), (0, 8), (1, 12)] {
            state.panes.panes.entry(tab).or_default().push(PaneInfo {
                id: pane_id,
                ..PaneInfo::default()
            });
        }
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Exec".to_string()),
            summary: Some("fix coding agent integration".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));
        assert!(state.apply_agent_event(AgentEvent {
            source: "Claude Code".to_string(),
            event: "PermissionRequest".to_string(),
            tool: Some("Bash".to_string()),
            summary: None,
            pane_id: 8,
            timestamp: Some(2),
        }));
        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "Stop".to_string(),
            tool: None,
            summary: None,
            pane_id: 12,
            timestamp: Some(3),
        }));

        let entries = state.agent_entries();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["1·1 choco-pi", "1·2 Claude Code", "2·1 Codex"],
            "cards follow tab then pane order"
        );
        assert_eq!(entries[0].state, AgentState::Working);
        assert!(
            entries[0].detail.as_deref().unwrap().ends_with("· exec"),
            "the running tool names what the agent is doing now"
        );
        assert_eq!(entries[1].state, AgentState::Blocked);
        assert!(entries[1].detail.as_deref().unwrap().ends_with("· Bash"));
        assert_eq!(entries[2].state, AgentState::Done);
    }

    #[test]
    fn summary_survives_later_events_and_state_keeps_its_clock() {
        let mut state = State::default();
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "UserPromptSubmit".to_string(),
            tool: None,
            summary: Some("fix the sidebar".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));
        let started = state.agent_statuses[&7].since;
        state.agent_statuses.get_mut(&7).unwrap().since = started - 90;

        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "UserPromptSubmit".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(2),
        }));
        let status = &state.agent_statuses[&7];
        assert_eq!(status.summary.as_deref(), Some("fix the sidebar"));
        assert_eq!(
            status.since,
            started - 90,
            "an unchanged state keeps counting from when it began"
        );

        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Exec".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(3),
        }));
        assert!(
            state.agent_statuses[&7].since > started - 90,
            "a new state restarts the clock"
        );
    }

    #[test]
    fn elapsed_labels_stay_compact() {
        assert_eq!(elapsed_label(0), "0s");
        assert_eq!(elapsed_label(59), "59s");
        assert_eq!(elapsed_label(60), "1m");
        assert_eq!(elapsed_label(3599), "59m");
        assert_eq!(elapsed_label(3600), "1h0m");
        assert_eq!(elapsed_label(7860), "2h11m");
    }

    #[test]
    fn the_active_tabs_detail_row_is_not_bold() {
        let mut state = State::default();
        state.view = View::Vertical;
        state.tabs = vec![TabInfo {
            position: 0,
            name: "work".to_string(),
            active: true,
            ..TabInfo::default()
        }];
        state.cwd_by_tab.insert(0, PathBuf::from("/tmp/work"));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(6, 30, &colors);
        state.render_vertical(&mut frame, 6, 30);
        let output = frame.finish();

        // Read the style the frame emits immediately before the detail text.
        let text = output
            .find("/tmp/work")
            .expect("the active tab shows its directory");
        let style_start = output[..text]
            .rfind("\u{1b}[")
            .expect("the text carries a style");
        let style = &output[style_start..text];
        assert!(
            style.starts_with("\u{1b}[22;"),
            "the detail row stays unbolded, got {style:?}"
        );

        let title = output.find("▸1 work").expect("the active tab is marked");
        let title_style_start = output[..title].rfind("\u{1b}[").unwrap();
        assert!(
            output[title_style_start..title].starts_with("\u{1b}[1;"),
            "the title row keeps its bold"
        );
    }

    #[test]
    fn the_tab_header_spells_out_its_totals_when_they_fit() {
        assert_eq!(tab_totals_label(2, 3, 24), "2 tabs · 3 panes");
        // One pane per tab: the second number would only repeat the first.
        assert_eq!(tab_totals_label(3, 3, 24), "3 tabs");
        assert_eq!(tab_totals_label(1, 1, 24), "1 tab");
        // A narrow sidebar keeps the counts rather than a truncated word.
        assert_eq!(tab_totals_label(2, 3, 8), "2 · 3");
        assert_eq!(tab_totals_label(3, 3, 4), "3");

        assert_eq!(agent_totals_label(3, 20), "3 sessions");
        assert_eq!(agent_totals_label(1, 20), "1 session");
        assert_eq!(agent_totals_label(3, 4), "3");
    }

    #[test]
    fn focus_follows_the_active_tab_not_whichever_tab_comes_first() {
        let mut state = State::default();
        state.tabs = vec![
            TabInfo {
                position: 0,
                ..TabInfo::default()
            },
            TabInfo {
                position: 1,
                active: true,
                ..TabInfo::default()
            },
        ];
        // Each tab keeps its own focused pane; only the active tab counts.
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                is_focused: true,
                ..PaneInfo::default()
            }],
        );
        state.panes.panes.insert(
            1,
            vec![PaneInfo {
                id: 9,
                is_focused: true,
                ..PaneInfo::default()
            }],
        );

        state.track_focused_pane();
        assert_eq!(state.focused_terminal_pane, Some(9));
    }

    #[test]
    fn the_section_names_are_bold_and_their_totals_are_not() {
        let mut state = State::default();
        state.view = View::Vertical;
        state.tabs = vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }];
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("exec".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(12, 30, &colors);
        state.render_vertical(&mut frame, 12, 30);
        let output = frame.finish();

        let weight_before = |needle: &str| {
            let at = output.find(needle).expect("the section is rendered");
            let style_at = output[..at].rfind("\u{1b}[").expect("it carries a style");
            output[style_at..at].to_string()
        };
        assert!(weight_before(" Tabs").starts_with("\u{1b}[1;"));
        assert!(weight_before(" Agents").starts_with("\u{1b}[1;"));
        assert!(
            weight_before("1 tab").starts_with("\u{1b}[22;"),
            "the totals stay unbolded"
        );
    }

    #[test]
    fn an_idle_state_stays_legible_on_a_focused_card() {
        let colors = Colors::default();
        // The idle accent is a muted tone, and so is the highlight behind it.
        assert!(
            contrast_ratio(colors.cwd_normal.fg, colors.tab_active.bg) < 2.5,
            "the fixture needs an accent that collides with the highlight"
        );
        assert_eq!(
            readable_on(
                colors.cwd_normal.fg,
                colors.tab_active.bg,
                colors.tab_active.fg
            ),
            colors.tab_active.fg,
            "an illegible accent gives way to the card's foreground"
        );
        assert_eq!(
            readable_on(
                colors.cwd_normal.fg,
                colors.background,
                colors.tab_active.fg
            ),
            colors.cwd_normal.fg,
            "the same accent is kept on the flat background, where it does read"
        );

        let mut state = State::default();
        state.view = View::Vertical;
        state.tabs = vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }];
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                is_focused: true,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "SessionStart".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));
        state.track_focused_pane();

        let mut frame = AnsiFrame::new(10, 30, &colors);
        state.render_vertical(&mut frame, 10, 30);
        let output = frame.finish();

        let at = output.find("idle").expect("the idle agent is listed");
        let style_at = output[..at].rfind("\u{1b}[").unwrap();
        let style = &output[style_at..at];
        let dim = format!(
            "38;2;{};{};{};",
            colors.cwd_normal.fg.0, colors.cwd_normal.fg.1, colors.cwd_normal.fg.2
        );
        assert!(
            !style.contains(&dim),
            "the state text drops the accent that vanishes on the highlight, got {style:?}"
        );
    }

    #[test]
    fn a_focused_agent_card_is_highlighted_across_its_whole_width() {
        let mut state = State::default();
        state.view = View::Vertical;
        state.tabs = vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }];
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                is_focused: true,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("exec".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));
        state.track_focused_pane();

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(10, 30, &colors);
        state.render_vertical(&mut frame, 10, 30);
        let output = frame.finish();

        // Every run on the focused card carries one background, trailing pad
        // included, instead of a single highlighted marker cell.
        let active_bg = format!(
            "48;2;{};{};{}m",
            colors.tab_active.bg.0, colors.tab_active.bg.1, colors.tab_active.bg.2
        );
        let flat_bg = format!(
            "48;2;{};{};{}m",
            colors.background.0, colors.background.1, colors.background.2
        );
        let card = output
            .find("1·1 choco-pi")
            .expect("the focused agent is listed");
        // The frame emits one row per cursor move, so slice the card's row out
        // and check every run in it.
        let row_start = output[..card].rfind("\u{1b}[").unwrap();
        let row_end = output[card..]
            .find("H\u{1b}[")
            .map(|offset| card + offset)
            .unwrap_or(output.len());
        let row = &output[row_start..row_end];
        assert!(
            row.contains(&active_bg),
            "the focused card uses the active background, got {row:?}"
        );
        assert!(
            !row.contains(&flat_bg),
            "no part of the focused card keeps the flat background, got {row:?}"
        );
    }

    #[test]
    fn the_focused_panes_agent_card_is_marked_like_the_active_tab() {
        let mut state = State::default();
        state.view = View::Vertical;
        state.tabs = vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }];
        state.panes.panes.insert(
            0,
            vec![
                PaneInfo {
                    id: 7,
                    is_focused: true,
                    ..PaneInfo::default()
                },
                PaneInfo {
                    id: 8,
                    ..PaneInfo::default()
                },
            ],
        );
        for pane_id in [7, 8] {
            assert!(state.apply_agent_event(AgentEvent {
                source: "choco-pi".to_string(),
                event: "PreToolUse".to_string(),
                tool: Some("exec".to_string()),
                summary: None,
                pane_id,
                timestamp: Some(1),
            }));
        }
        state.track_focused_pane();
        assert_eq!(state.focused_terminal_pane, Some(7));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(14, 30, &colors);
        state.render_vertical(&mut frame, 14, 30);
        let output = frame.finish();

        let marked: Vec<&str> = output
            .lines()
            .filter(|line| line.contains("choco-pi") && line.contains('▸'))
            .collect();
        assert_eq!(
            marked.len(),
            1,
            "only the focused pane's agent card carries the marker"
        );
        assert!(
            marked[0].contains("1·1"),
            "the marked card is the focused pane, got {:?}",
            marked[0]
        );
    }

    #[test]
    fn vertical_sidebar_renders_tab_and_agent_sections() {
        let mut state = State::default();
        state.view = View::Vertical;
        state.tabs = vec![
            TabInfo {
                position: 0,
                active: true,
                ..TabInfo::default()
            },
            TabInfo {
                position: 1,
                ..TabInfo::default()
            },
        ];
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PermissionRequest".to_string(),
            tool: Some("Bash".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(12, 30, &colors);
        state.render_vertical(&mut frame, 12, 30);
        let output = frame.finish();

        assert!(output.contains(" Tabs"), "the tab section has a header");
        assert!(output.contains(" Agents"), "the agent section has a header");
        assert!(
            output.contains("2 tabs · 1 pane"),
            "the tab header counts tabs and panes"
        );
        assert!(
            output.contains("1·1 choco-pi"),
            "cards show tab·pane and agent"
        );
        assert!(output.contains("blocked"), "cards show the state text");
        assert!(output.contains('◉'), "blocked uses its own glyph");

        assert_eq!(
            state.visible_vertical_tabs,
            vec![(1, 0), (3, 1)],
            "tab cards occupy two rows each below the section header"
        );
        let agent_rows: Vec<usize> = state
            .agent_focus_targets
            .iter()
            .map(|(line, _, _)| *line)
            .collect();
        assert_eq!(agent_rows.len(), 2, "both card rows focus the agent pane");
        assert!(
            agent_rows.iter().all(|line| *line > 3),
            "agent cards render below the tab section"
        );
    }

    #[test]
    fn git_rev_parse_output_yields_repository_branch_and_worktree() {
        let plain = parse_repo_info(
            b"/Users/me/Workspace/novaid\nmain\n/Users/me/Workspace/novaid/.git\n/Users/me/Workspace/novaid/.git\n",
        )
        .unwrap();
        assert_eq!(plain.repository, "novaid");
        assert_eq!(plain.branch, "main");
        assert_eq!(plain.worktree, None);
        assert_eq!(tab_detail_line(Some(&plain), Some("~/x")), "main");

        let worktree = parse_repo_info(
            b"/Users/me/Workspace/novaid-fix\nfix/auth\n/Users/me/Workspace/novaid/.bare/worktrees/fix-auth\n/Users/me/Workspace/novaid/.bare\n",
        )
        .unwrap();
        assert_eq!(
            worktree.repository, "novaid",
            "a worktree is named after its repository, not its own folder"
        );
        assert_eq!(worktree.branch, "fix/auth");
        assert_eq!(worktree.worktree.as_deref(), Some("fix-auth"));
        assert_eq!(
            tab_detail_line(Some(&worktree), None),
            "fix/auth · ⑂fix-auth"
        );

        let matching_worktree = RepoInfo {
            repository: "medpath".to_string(),
            branch: "medpath-wt-2".to_string(),
            worktree: Some("medpath-wt-2".to_string()),
        };
        assert_eq!(
            tab_detail_line(Some(&matching_worktree), None),
            "⑂medpath-wt-2"
        );

        let detached = parse_repo_info(
            b"/Users/me/Workspace/novaid\nHEAD\n/Users/me/Workspace/novaid/.git\n/Users/me/Workspace/novaid/.git\n",
        )
        .unwrap();
        assert_eq!(detached.branch, "detached");

        assert!(parse_repo_info(b"").is_none());
        assert_eq!(
            tab_detail_line(None, Some("~/Workspace/x")),
            "~/Workspace/x"
        );
    }

    #[test]
    fn generated_tab_names_give_way_to_the_repository() {
        let repo = RepoInfo {
            repository: "novaid".to_string(),
            branch: "main".to_string(),
            worktree: None,
        };
        let generated = TabInfo {
            position: 4,
            name: "Tab #5".to_string(),
            ..TabInfo::default()
        };
        let named = TabInfo {
            position: 0,
            name: "NovaID - Main".to_string(),
            ..TabInfo::default()
        };
        assert_eq!(tab_display_name(&generated, Some(&repo)), "novaid");
        assert_eq!(tab_display_name(&generated, None), "Tab #5");
        assert_eq!(
            tab_display_name(&named, Some(&repo)),
            "NovaID - Main",
            "a name the user chose is never replaced"
        );
    }

    #[test]
    fn a_cached_permission_grant_is_recognised_without_a_result_event() {
        let mut state = State::default();
        assert!(
            !state.permissions_granted,
            "a plugin starts out unpermitted"
        );

        // Zellij sends no PermissionRequestResult when it restores a grant from
        // its cache; the first application-state event is the only evidence.
        state.update(Event::TabUpdate(vec![TabInfo {
            position: 0,
            name: "Tab #1".to_string(),
            active: true,
            ..TabInfo::default()
        }]));

        assert!(
            state.permissions_granted,
            "state updates only reach a permitted plugin, so they prove the grant"
        );
    }

    #[test]
    fn an_active_agent_stays_active_while_the_model_is_quiet() {
        let mut state = State::default();
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "UserPromptSubmit".to_string(),
            tool: None,
            summary: Some("fix the sidebar".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));

        // A minute of silence is ordinary: the model is still producing the turn.
        let now = unix_seconds();
        let status = &state.agent_statuses[&7];
        assert_eq!(status.state, AgentState::Thinking);
        assert!(
            status.expires_at.is_some_and(|expires| expires > now + 300),
            "a thinking agent must outlive a long quiet stretch, got {:?}",
            status.expires_at.map(|expires| expires.saturating_sub(now))
        );

        state.update(Event::Timer(1.0));
        assert_eq!(
            state.agent_statuses[&7].state,
            AgentState::Thinking,
            "the timer must not report a working agent as idle"
        );
    }

    #[test]
    fn an_agent_card_shows_the_running_tool_then_falls_back_to_the_task() {
        let mut state = State::default();
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("read_symbol".to_string()),
            summary: Some("ship the sidebar".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));
        let entry = state.agent_entries().remove(0);
        assert!(
            entry
                .detail
                .as_deref()
                .is_some_and(|detail| detail.ends_with("· reading code")),
            "the card names the running tool, got {:?}",
            entry.detail
        );

        // With the turn over there is no tool, so the task takes the slot back.
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "Stop".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(2),
        }));
        let entry = state.agent_entries().remove(0);
        assert!(
            entry
                .detail
                .as_deref()
                .is_some_and(|detail| detail.ends_with("· ship the sidebar")),
            "the task returns once the tool call ends, got {:?}",
            entry.detail
        );
    }

    #[test]
    fn choco_pi_code_mode_reads_differently_from_a_single_tool_call() {
        assert_eq!(
            tool_detail("choco-pi", Some("exec".to_string())).as_deref(),
            Some("code mode")
        );
        assert_eq!(
            tool_detail("choco-pi", Some("read_text".to_string())).as_deref(),
            Some("reading"),
            "a single tool call reads as what it does"
        );
        assert_eq!(
            tool_detail("choco-pi", Some("apply_patch".to_string())).as_deref(),
            Some("editing")
        );
        // An unknown tool still reads as words rather than as an identifier.
        assert_eq!(
            tool_detail("choco-pi", Some("evaluate_browser".to_string())).as_deref(),
            Some("evaluate browser")
        );
        assert_eq!(humanised_tool_name("agentBrowser"), "agent browser");
        assert_eq!(
            tool_detail("claude-code", Some("exec".to_string())).as_deref(),
            Some("exec"),
            "only choco-pi has a code mode"
        );
    }

    #[test]
    fn a_new_sidebar_adopts_the_statuses_its_siblings_already_have() {
        let mut first = State::default();
        assert!(first.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("exec".to_string()),
            summary: Some("fix the sidebar".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));

        // A sidebar loaded into a new tab starts empty and reads the shared file.
        let mut second = State::default();
        second.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        let payload = encode_agent_statuses(&first.agent_statuses);
        assert!(second.merge_agent_statuses(decode_agent_statuses(&payload)));

        let status = second.agent_statuses.get(&7).expect("the agent is adopted");
        assert_eq!(status.state, AgentState::Working);
        assert_eq!(status.detail.as_deref(), Some("code mode"));
        assert_eq!(status.summary.as_deref(), Some("fix the sidebar"));
        assert_eq!(second.agent_entries().len(), 1);
    }

    #[test]
    fn a_sidebar_keeps_its_own_newer_status_and_skips_departed_panes() {
        let mut state = State::default();
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PermissionRequest".to_string(),
            tool: Some("Bash".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(2),
        }));
        let known = state.agent_statuses[&7].updated_at;

        let stale = vec![
            AgentStatus {
                pane_id: 7,
                source: "choco-pi".to_string(),
                state: AgentState::Idle,
                detail: None,
                summary: None,
                since: 0,
                sequence: 0,
                expires_at: None,
                updated_at: known.saturating_sub(1),
                detected: false,
                clear_on_focus: false,
            },
            AgentStatus {
                pane_id: 42,
                source: "claude-code".to_string(),
                state: AgentState::Working,
                detail: None,
                summary: None,
                since: 0,
                sequence: 0,
                expires_at: None,
                updated_at: known + 10,
                detected: false,
                clear_on_focus: false,
            },
        ];
        assert!(!state.merge_agent_statuses(stale));

        assert_eq!(
            state.agent_statuses[&7].state,
            AgentState::Blocked,
            "an older copy must not overwrite a newer one"
        );
        assert!(
            !state.agent_statuses.contains_key(&42),
            "a pane this session does not have is not resurrected"
        );
    }

    #[test]
    fn a_finished_agent_waiting_for_input_is_done_not_blocked() {
        let mut state = State::default();
        // Claude Code raises this once a turn ends and nobody has replied.
        assert!(state.apply_agent_event(AgentEvent {
            source: "claude-code".to_string(),
            event: "Notification".to_string(),
            tool: Some("Claude is waiting for your input".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));
        let status = &state.agent_statuses[&7];
        assert_eq!(status.state, AgentState::Done);
        assert!(!status.urgent(), "a finished turn must not read as urgent");

        // The approval notification is the one that really blocks the agent.
        assert!(state.apply_agent_event(AgentEvent {
            source: "claude-code".to_string(),
            event: "Notification".to_string(),
            tool: Some("Claude needs your permission to use Bash".to_string()),
            summary: None,
            pane_id: 7,
            timestamp: Some(2),
        }));
        let status = &state.agent_statuses[&7];
        assert_eq!(status.state, AgentState::Blocked);
        assert_eq!(
            status.detail.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
    }

    #[test]
    fn compaction_events_show_a_compacting_state() {
        let mut state = State::default();
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreCompact".to_string(),
            tool: Some("auto".to_string()),
            summary: Some("rebuild the sidebar".to_string()),
            pane_id: 3,
            timestamp: Some(1),
        }));

        let status = state.agent_statuses.get(&3).unwrap();
        assert_eq!(status.state, AgentState::Compacting);
        assert_eq!(status.state.glyph(), '◍');
        assert_eq!(status.state.label(), "compacting");
        assert_eq!(status.detail.as_deref(), Some("auto"));
        assert!(!status.urgent(), "compaction is routine, not a prompt");
        assert!(
            status.expires_at.is_some(),
            "a compaction that never reports back must decay to idle"
        );

        // Compaction ends and the agent picks its turn back up.
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PostCompact".to_string(),
            tool: Some("auto".to_string()),
            summary: None,
            pane_id: 3,
            timestamp: Some(2),
        }));
        let status = state.agent_statuses.get(&3).unwrap();
        assert_eq!(status.state, AgentState::Thinking);
        assert_eq!(status.detail, None);
        assert_eq!(
            status.summary.as_deref(),
            Some("rebuild the sidebar"),
            "the task survives compaction"
        );
    }

    #[test]
    fn quiet_agents_stay_listed_as_idle_until_their_pane_closes() {
        let mut state = State::default();
        state.panes.panes.insert(
            0,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Exec".to_string()),
            summary: Some("fix the sidebar".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));

        // Expire the working status the way the timer would.
        state.agent_statuses.get_mut(&7).unwrap().expires_at = Some(1);
        state.update(Event::Timer(1.0));

        let status = state.agent_statuses.get(&7).expect("agent stays listed");
        assert_eq!(status.state, AgentState::Idle);
        assert_eq!(status.detail, None, "the stale tool name is dropped");
        assert_eq!(
            status.summary.as_deref(),
            Some("fix the sidebar"),
            "the last task stays as context"
        );
        assert_eq!(state.agent_entries().len(), 1);

        // Closing the pane removes the agent.
        state.update(Event::PaneUpdate(PaneManifest {
            panes: HashMap::from([(
                0,
                vec![PaneInfo {
                    id: 9,
                    ..PaneInfo::default()
                }],
            )]),
        }));
        assert!(state.agent_statuses.is_empty());
    }

    #[test]
    fn tab_window_keeps_the_active_tab_visible() {
        assert_eq!(vertical_tab_window(3, 0, 5), vec![0, 1, 2]);
        assert_eq!(vertical_tab_window(10, 0, 3), vec![0, 1, 2]);
        assert_eq!(vertical_tab_window(10, 5, 3), vec![4, 5, 6]);
        assert_eq!(vertical_tab_window(10, 9, 3), vec![7, 8, 9]);
        assert!(vertical_tab_window(0, 0, 4).is_empty());
        assert!(vertical_tab_window(4, 0, 0).is_empty());
    }
    #[test]
    fn session_end_tombstone_rejects_delayed_events() {
        let mut state = State::default();
        assert!(state.handle_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "PreToolUse".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(200),
        }));

        assert!(state.handle_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "SessionEnd".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(300),
        }));
        assert_eq!(state.last_hook_timestamp_by_pane.get(&7), Some(&300));
        assert!(state.agent_statuses.is_empty());

        assert!(!state.handle_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "PreToolUse".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(300),
        }));
        assert_eq!(state.last_hook_timestamp_by_pane.get(&7), Some(&300));
        assert!(state.agent_statuses.is_empty());

        assert!(state.handle_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "SessionStart".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(300),
        }));
        assert_eq!(state.last_hook_timestamp_by_pane.get(&7), Some(&300));
        assert_eq!(state.agent_statuses.get(&7).unwrap().pane_id, 7);

        assert!(state.handle_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "PostToolUse".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(301),
        }));
    }

    #[test]
    fn invalid_datetime_format_falls_back_to_default() {
        assert_eq!(
            validated_datetime_format(Some("%Y-%Q")),
            DEFAULT_DATETIME_FORMAT
        );
    }

    #[test]
    fn permissions_match_view_requirements() {
        for view in [View::Horizontal, View::Vertical] {
            let permissions = permissions_for_view(view);
            assert!(
                permissions.contains(&PermissionType::ReadCliPipes),
                "both views consume coding-agent pipe events"
            );
            assert!(
                permissions.contains(&PermissionType::RunCommands),
                "both views resolve repository state with git"
            );
            assert!(permissions.contains(&PermissionType::ReadApplicationState));
        }
    }

    #[test]
    fn parses_hex_colors_and_falls_back_for_invalid_values() {
        assert_eq!(parse_hex_color("#88C0D0"), Some(Rgb(136, 192, 208)));
        assert_eq!(parse_hex_color("bf616a"), Some(Rgb(191, 97, 106)));
        assert_eq!(parse_hex_color("#xyzxyz"), None);

        let mut configuration = BTreeMap::new();
        configuration.insert("color_background".to_string(), "not-a-color".to_string());
        assert_eq!(
            Colors::from_config(&configuration).background,
            Rgb(46, 52, 64)
        );
    }

    #[test]
    fn generates_24_bit_ansi_styles() {
        assert_eq!(
            ansi_style(Style {
                fg: Rgb(1, 2, 3),
                bg: Rgb(4, 5, 6),
                bold: true,
            }),
            "\x1b[1;38;2;1;2;3;48;2;4;5;6m"
        );
    }

    #[test]
    fn sanitizes_and_clips_dynamic_text_before_ansi_output() {
        let malicious = "\0\u{1b}[2J\n\r\t\u{7f}\u{85}界";
        let sanitized = sanitize_and_clip(malicious, 4);
        assert_eq!(sanitized, "��[2");
        assert_eq!(cell_width(&sanitized), 4);
        assert!(sanitized.chars().all(|character| !character.is_control()));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(1, 8, &colors);
        frame.put(6, 0, colors.agent, malicious);
        let output = frame.finish();
        assert!(!output.contains("\u{1b}[2J"));

        let mut remainder = output.as_str();
        while let Some(index) = remainder.find('\u{1b}') {
            remainder = &remainder[index + 1..];
            assert!(remainder.starts_with('['));
            let Some(end) = remainder.find(['H', 'm']) else {
                assert!(false, "renderer escape sequence must be terminated");
                break;
            };
            assert!(remainder[1..end]
                .chars()
                .all(|character| character.is_ascii_digit() || character == ';'));
            remainder = &remainder[end + 1..];
        }
    }

    #[test]
    fn repeats_border_patterns_to_the_exact_cell_width() {
        let border = repeat_pattern_to_width("界", 80);
        assert_eq!(cell_width(&border), 80);
        assert_eq!(border.chars().count(), 40);
        assert_eq!(sanitize_and_clip(&border, 80), border);

        assert_eq!(repeat_pattern_to_width("界", 3), "界 ");
        assert_eq!(repeat_pattern_to_width("", 5), "     ");
        assert_eq!(repeat_pattern_to_width("\u{1b}\n\t", 5), "     ");
    }

    #[test]
    fn centers_horizontal_tabs_on_the_full_pane_and_clamps_around_content() {
        assert_eq!(horizontal_group_start(100, 20, 15, 85), 40);
        assert_eq!(horizontal_group_start(100, 60, 25, 90), 25);
        assert_eq!(horizontal_group_start(100, 30, 10, 60), 30);
    }

    #[test]
    fn horizontal_tab_labels_keep_their_natural_width() {
        assert_eq!(truncate_line(" 1 main ", 24), " 1 main ");
        assert_eq!(
            truncate_line(" 1 a-very-long-tab-name ", 12),
            " 1 a-very-l…"
        );
    }

    #[test]
    fn active_vertical_lines_use_distinct_configured_backgrounds() {
        let mut configuration = BTreeMap::new();
        configuration.insert("color_tab_active_bg".to_string(), "#112233".to_string());
        configuration.insert("color_cwd_active_bg".to_string(), "#445566".to_string());
        let colors = Colors::from_config(&configuration);
        let (title, cwd) = vertical_styles(&colors, true);
        assert_eq!(title.bg, Rgb(17, 34, 51));
        assert_eq!(cwd.bg, Rgb(68, 85, 102));
        assert_ne!(title.bg, cwd.bg);
        assert!(title.bold);
        assert!(cwd.bold);

        let (title, cwd) = vertical_styles(&colors, false);
        assert!(!title.bold);
        assert!(!cwd.bold);
    }

    #[test]
    fn hidden_tabs_move_context_to_center_and_leave_only_the_clock_on_the_right() {
        let context = " choco-pi   main* · cargo";
        let clock = "  2025-01-02 03:04 ";
        let (center, right_context, right_clock) =
            horizontal_content_split(false, false, context, clock);
        assert_eq!(center.as_deref(), Some(context));
        assert!(right_context.is_empty());
        assert_eq!(right_clock, clock);

        let (center, right_context, right_clock) =
            horizontal_content_split(false, true, context, clock);
        assert!(center.is_none(), "agent status owns the center");
        assert!(right_context.is_empty());
        assert_eq!(right_clock, clock);
    }

    #[test]
    fn vertical_separator_is_one_cell_wide_and_full_height() {
        let (x, lines) = vertical_separator_content(4, 12, true, "│").unwrap();
        assert_eq!(x, 11);
        assert_eq!(lines.len(), 4);
        assert!(lines
            .iter()
            .all(|line| line == "│" && cell_width(line) == 1));

        let (_, wide_lines) = vertical_separator_content(2, 1, true, "界").unwrap();
        assert!(wide_lines
            .iter()
            .all(|line| line == " " && cell_width(line) == 1));
        let (_, invalid_lines) = vertical_separator_content(2, 1, true, "\u{1b}\n").unwrap();
        assert!(invalid_lines
            .iter()
            .all(|line| line == " " && cell_width(line) == 1));
        assert!(vertical_separator_content(2, 0, true, "│").is_none());
        assert!(vertical_separator_content(2, 12, false, "│").is_none());
    }

    #[test]
    fn layout_ui_views_are_not_selectable() {
        assert!(!view_selectable(View::Horizontal));
        assert!(!view_selectable(View::Vertical));
    }

    #[test]
    fn width_sync_decisions_are_absolute_and_use_exact_directions() {
        let target = 29;
        assert_eq!(
            [15, 87, 29].map(|width| width_sync_strategy(width, target, Direction::Right)),
            [
                Some(ResizeStrategy {
                    resize: Resize::Increase,
                    direction: Some(Direction::Right),
                    invert_on_boundaries: false,
                }),
                Some(ResizeStrategy {
                    resize: Resize::Decrease,
                    direction: Some(Direction::Right),
                    invert_on_boundaries: false,
                }),
                None,
            ]
        );
        assert_eq!(
            width_sync_strategy(15, target, Direction::Left),
            Some(ResizeStrategy {
                resize: Resize::Increase,
                direction: Some(Direction::Left),
                invert_on_boundaries: false,
            })
        );
    }

    #[test]
    fn exact_widths_complete_pending_sync_without_actions() {
        let mut pending = Some(PendingWidthSync {
            target_width: 29,
            pane_ids: vec![11, 12],
            last_requested_widths: HashMap::new(),
            attempts_remaining: WIDTH_SYNC_MAX_ATTEMPTS,
        });

        assert!(plan_width_sync_attempt(
            &mut pending,
            &[
                (11, Some((29, Direction::Right))),
                (12, Some((29, Direction::Right))),
            ],
        )
        .is_empty());
        assert_eq!(pending, None);
    }

    #[test]
    fn pending_sync_plans_from_live_widths_not_payload_deltas() {
        let mut pending = Some(PendingWidthSync {
            target_width: 29,
            pane_ids: vec![11, 12, 13],
            last_requested_widths: HashMap::new(),
            attempts_remaining: WIDTH_SYNC_MAX_ATTEMPTS,
        });

        let actions = plan_width_sync_attempt(
            &mut pending,
            &[
                (11, Some((29, Direction::Right))),
                (12, Some((15, Direction::Right))),
                (13, Some((87, Direction::Right))),
            ],
        );
        assert_eq!(
            actions,
            vec![
                (
                    12,
                    ResizeStrategy {
                        resize: Resize::Increase,
                        direction: Some(Direction::Right),
                        invert_on_boundaries: false,
                    },
                ),
                (
                    13,
                    ResizeStrategy {
                        resize: Resize::Decrease,
                        direction: Some(Direction::Right),
                        invert_on_boundaries: false,
                    },
                ),
            ]
        );
        assert_eq!(
            pending.as_ref().unwrap().attempts_remaining,
            WIDTH_SYNC_MAX_ATTEMPTS - 1
        );
        assert!(plan_width_sync_attempt(
            &mut pending,
            &[
                (11, Some((29, Direction::Right))),
                (12, Some((15, Direction::Right))),
                (13, Some((87, Direction::Right))),
            ],
        )
        .is_empty());
        assert_eq!(
            plan_width_sync_attempt(
                &mut pending,
                &[
                    (11, Some((29, Direction::Right))),
                    (12, Some((19, Direction::Right))),
                    (13, Some((83, Direction::Right))),
                ],
            )
            .len(),
            2
        );
    }

    #[test]
    fn pending_sync_stops_after_bounded_attempts_without_progress() {
        let mut pending = Some(PendingWidthSync {
            target_width: 29,
            pane_ids: vec![11],
            last_requested_widths: HashMap::new(),
            attempts_remaining: 1,
        });

        let actions = plan_width_sync_attempt(&mut pending, &[(11, Some((15, Direction::Right)))]);
        assert_eq!(actions.len(), 1);
        assert_eq!(pending, None);
    }

    #[test]
    fn finds_every_visible_sidebar_across_tabs_including_self() {
        let pane = |id, is_plugin, is_focused, is_suppressed, plugin_url: Option<&str>| PaneInfo {
            id,
            is_plugin,
            is_focused,
            is_suppressed,
            plugin_url: plugin_url.map(str::to_string),
            ..PaneInfo::default()
        };
        let mut panes = PaneManifest::default();
        panes.panes.insert(
            0,
            vec![
                pane(
                    11,
                    true,
                    true,
                    false,
                    Some("file:/plugins/vertical-sidebar.wasm"),
                ),
                pane(
                    12,
                    true,
                    false,
                    false,
                    Some("file:/plugins/vertical-sidebar.wasm"),
                ),
                pane(
                    14,
                    true,
                    false,
                    true,
                    Some("file:/plugins/vertical-sidebar.wasm"),
                ),
            ],
        );
        let mut right_sidebar = pane(
            13,
            true,
            false,
            false,
            Some("file:/other/vertical-sidebar.wasm"),
        );
        right_sidebar.pane_x = 40;
        panes.panes.insert(
            1,
            vec![
                right_sidebar,
                pane(
                    15,
                    true,
                    false,
                    false,
                    Some("file:/plugins/vertical-tabs.wasm"),
                ),
                pane(16, false, false, false, None),
            ],
        );

        assert_eq!(visible_vertical_sidebar_ids(&panes), vec![11, 12, 13]);
        assert_eq!(sidebar_geometry(&panes, 11), Some((0, Direction::Right)));
        assert_eq!(sidebar_geometry(&panes, 13), Some((0, Direction::Left)));
        let tabs = vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }];
        assert_eq!(active_sidebar_state(&tabs, &panes, 11), Some((0, true)));
        assert_eq!(active_sidebar_state(&tabs, &panes, 13), None);
    }

    #[test]
    fn detects_when_own_tab_has_only_layout_ui_plugins() {
        let plugin = |id, url: &str, is_suppressed| PaneInfo {
            id,
            is_plugin: true,
            is_suppressed,
            plugin_url: Some(url.to_string()),
            ..PaneInfo::default()
        };
        let tabs = vec![TabInfo {
            position: 0,
            tab_id: 42,
            ..TabInfo::default()
        }];
        let mut panes = PaneManifest::default();
        panes.panes.insert(
            0,
            vec![
                plugin(11, "file:/plugins/vertical-sidebar.wasm", false),
                plugin(12, "file:/plugins/vertical-tabs.wasm", false),
                PaneInfo {
                    id: 7,
                    ..PaneInfo::default()
                },
            ],
        );

        assert_eq!(own_tab_content_state(&tabs, &panes, 11), Some((42, true)));
        panes.panes.get_mut(&0).unwrap().pop();
        panes
            .panes
            .get_mut(&0)
            .unwrap()
            .push(plugin(13, "zellij:link", true));
        assert_eq!(own_tab_content_state(&tabs, &panes, 11), Some((42, false)));
        panes
            .panes
            .get_mut(&0)
            .unwrap()
            .push(plugin(14, "zellij:strider", false));
        assert_eq!(own_tab_content_state(&tabs, &panes, 11), Some((42, true)));
    }

    #[test]
    fn preserves_complete_clock_in_a_common_horizontal_budget() {
        let clock = "  2025-01-02 03:04 ";
        let right_budget = (80_usize - 15) * 2 / 5;
        let (context, clock) = fit_right_parts(
            " choco-pi   feature/long-branch · cargo-watch",
            clock,
            right_budget,
        );
        let rendered = format!("{context}{clock}");
        assert_eq!(cell_width(&rendered), right_budget);
        assert!(rendered.contains('…'));
        assert!(rendered.ends_with(&clock));
    }

    #[test]
    fn truncates_to_the_available_cell_width() {
        assert_eq!(fit_line(" 1  choco-pi", 10), " 1  choco…");
        assert_eq!(fit_line("abc", 5), "abc  ");
    }
}
