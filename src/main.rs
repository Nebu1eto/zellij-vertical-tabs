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

#[derive(Clone, Debug)]
struct AgentStatus {
    pane_id: u32,
    message: String,
    summary: Option<String>,
    urgent: bool,
    sequence: u64,
    expires_at: Option<u64>,
    detected: bool,
    clear_on_focus: bool,
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
        set_selectable(false);
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

        request_permission(permissions_for_view(self.view));
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
        set_timeout(1.0);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::ModeUpdate(mode_info) => {
                self.mode = mode_info.mode;
                self.session_name = mode_info.session_name.unwrap_or_default();
            }
            Event::TabUpdate(mut tabs) => {
                tabs.sort_by_key(|tab| tab.position);
                self.tabs = tabs;
                self.close_empty_own_tab_if_needed();
                self.refresh_active_pane();
                self.refresh_cwds();
            }
            Event::PaneUpdate(panes) => {
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
                self.active_pane_id = None;
                self.refresh_active_pane();
                self.refresh_cwds();
                self.refresh_git();
            }
            Event::Timer(_) => {
                set_timeout(1.0);
                let now = unix_seconds();
                self.agent_statuses.retain(|_, status| {
                    status.expires_at.is_none_or(|expires_at| expires_at > now)
                });
                self.timer_ticks = self.timer_ticks.wrapping_add(1);
                if self.timer_ticks >= self.git_refresh_interval {
                    self.timer_ticks = 0;
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
        self.handle_agent_event(event)
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
        if rows < 2 {
            return;
        }
        let active_index = self.tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let window = self.vertical_window(active_index, rows);

        let mut y = 0;
        for index in window {
            let tab = &self.tabs[index];
            self.visible_vertical_tabs.push((y, tab.position));
            let marker = if tab.active { "▸" } else { " " };
            let (title_style, cwd_style) = vertical_styles(&self.colors, tab.active);
            let rollup = self.tab_rollup_glyph(tab.position).map(|(glyph, urgent)| {
                (
                    glyph,
                    Style {
                        fg: if urgent {
                            self.colors.agent_urgent.bg
                        } else {
                            self.colors.agent.bg
                        },
                        bg: title_style.bg,
                        bold: title_style.bold,
                    },
                )
            });
            let rollup_width = usize::from(rollup.is_some()) + usize::from(rollup.is_some());
            let title = fit_line(
                &format!(" {marker} {}  {}", tab.position + 1, tab_name(tab)),
                content_cols.saturating_sub(rollup_width),
            );
            let cwd = self
                .cwd_by_tab
                .get(&tab.position)
                .map(|path| display_path(path, self.configured_home.as_deref()))
                .unwrap_or_else(|| "—".to_string());
            let cwd = fit_line(&format!("    {cwd}"), content_cols);
            frame.put(0, y, title_style, &title);
            if let Some((glyph, style)) = rollup {
                frame.put(content_cols - 1, y, style, &glyph.to_string());
            }
            frame.put(0, y + 1, cwd_style, &cwd);
            y += 2;
            let agent_rows = self
                .agent_rows_for_tab(tab.position)
                .into_iter()
                .map(|(index, pane_id, status)| {
                    (
                        index,
                        pane_id,
                        status.urgent,
                        status.message.clone(),
                        status.summary.clone(),
                    )
                })
                .collect::<Vec<_>>();
            for (pane_index, pane_id, urgent, message, summary) in agent_rows {
                let message =
                    truncate_line(&format!("  ▸p{} {}", pane_index + 1, message), content_cols);
                let remaining = content_cols.saturating_sub(cell_width(&message));
                let suffix = summary
                    .or_else(|| self.agent_title_suffix(pane_id))
                    .filter(|_| remaining >= 8)
                    .map(|text| truncate_line(&format!(" · {text}"), remaining));
                frame.put(0, y, self.vertical_agent_row_style(urgent), &message);
                if let Some(suffix) = &suffix {
                    frame.put(
                        cell_width(&message),
                        y,
                        Style {
                            fg: self.colors.cwd_normal.fg,
                            bg: self.colors.background,
                            bold: false,
                        },
                        suffix,
                    );
                }
                self.agent_focus_targets.push((y, 0, pane_id));
                y += 1;
            }
            if y >= rows {
                break;
            }
        }
    }

    fn vertical_tab_height(&self, tab_position: usize) -> usize {
        2 + self.agent_rows_for_tab(tab_position).len()
    }

    fn vertical_window(&self, active_index: usize, rows: usize) -> Vec<usize> {
        if self.tabs.is_empty() {
            return Vec::new();
        }
        let active_index = active_index.min(self.tabs.len() - 1);
        let mut selected = vec![active_index];
        let mut used = self.vertical_tab_height(self.tabs[active_index].position);
        let mut up = active_index;
        let mut down = active_index;
        loop {
            if down + 1 < self.tabs.len()
                && used + self.vertical_tab_height(self.tabs[down + 1].position) <= rows
            {
                down += 1;
                used += self.vertical_tab_height(self.tabs[down].position);
                selected.push(down);
            } else if up > 0 && used + self.vertical_tab_height(self.tabs[up - 1].position) <= rows
            {
                up -= 1;
                used += self.vertical_tab_height(self.tabs[up].position);
                selected.push(up);
            } else {
                break;
            }
        }
        selected.sort_unstable();
        selected
    }

    fn tab_rollup_glyph(&self, tab_position: usize) -> Option<(char, bool)> {
        let rows = self.agent_rows_for_tab(tab_position);
        if rows.is_empty() {
            return None;
        }
        if rows.iter().any(|(_, _, status)| status.urgent) {
            return Some(('⚠', true));
        }
        if rows
            .iter()
            .any(|(_, _, status)| status.message.starts_with('…'))
        {
            return Some(('…', false));
        }
        let newest = rows.iter().max_by_key(|(_, _, status)| status.sequence)?.2;
        let glyph = newest.message.chars().next().unwrap_or('●');
        Some((glyph, false))
    }

    fn vertical_agent_row_style(&self, urgent: bool) -> Style {
        Style {
            fg: if urgent {
                self.colors.agent_urgent.bg
            } else {
                self.colors.agent.bg
            },
            bg: self.colors.background,
            bold: urgent,
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
        let tool = event
            .tool
            .as_deref()
            .map(|tool| format!(" · {tool}"))
            .unwrap_or_default();
        let (message, urgent, lifetime) = match event.event.as_str() {
            "SessionStart" => (format!("◆ {} connected", event.source), false, Some(5)),
            "UserPromptSubmit" => (format!("… {} thinking", event.source), false, Some(60)),
            "PreToolUse" => (format!("… {} working{tool}", event.source), false, Some(60)),
            "PostToolUse" => (format!("… {} working", event.source), false, Some(30)),
            "PostToolUseFailure" => (
                format!("✕ {} tool failed{tool}", event.source),
                true,
                Some(12),
            ),
            "PermissionRequest" => (
                format!("⚠ {} permission required{tool}", event.source),
                true,
                None,
            ),
            "Notification" => (format!("● {} notification", event.source), true, Some(12)),
            "SubagentStart" => (
                format!("◇ {} subagent started", event.source),
                false,
                Some(12),
            ),
            "SubagentStop" => (
                format!("◇ {} subagent complete", event.source),
                false,
                Some(8),
            ),
            "Stop" => (format!("✓ {} response complete", event.source), false, None),
            "StopFailure" => (
                format!("✕ {} response failed", event.source),
                true,
                Some(12),
            ),
            "SessionEnd" => return self.agent_statuses.remove(&event.pane_id).is_some(),
            _ => return false,
        };
        self.agent_sequence = self.agent_sequence.wrapping_add(1);
        let summary = event.summary.or_else(|| {
            self.agent_statuses
                .get(&event.pane_id)
                .and_then(|status| status.summary.clone())
        });
        self.agent_statuses.insert(
            event.pane_id,
            AgentStatus {
                pane_id: event.pane_id,
                message,
                summary,
                urgent,
                sequence: self.agent_sequence,
                expires_at: lifetime.map(|seconds| unix_seconds().saturating_add(seconds)),
                detected: false,
                clear_on_focus: event.event == "Stop",
            },
        );
        true
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
                    status.message
                )
            }
            None => status.message.clone(),
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
                    .map_or_else(|| self.agent_style(status.urgent), |(_, style)| *style);
                segments.push((SEPARATOR.to_string(), inherited));
                used += separator_width;
            }
            segments.push((text, self.agent_style(status.urgent)));
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
                segments.push((marker, self.agent_style(statuses[included].urgent)));
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
        let focused = self
            .panes
            .panes
            .values()
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
                        message: format!("◆ {label} detected"),
                        summary: None,
                        urgent: false,
                        sequence: self.agent_sequence,
                        expires_at: None,
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
            if let Ok(cwd) = get_pane_cwd(pane_id) {
                self.cwd_by_tab.insert(tab_position, cwd);
            }
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
    fn agent_failure_and_session_end_update_status() {
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
        assert!(status.urgent);
        assert_eq!(status.message, "✕ Codex response failed");

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
            tool: None,
            summary: None,
            pane_id: 12,
            timestamp: Some(2),
        }));
        assert_eq!(state.agent_statuses.len(), 2);

        let segments = state.agent_status_segments(200);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].0, "● Claude Code notification");
        assert_eq!(segments[0].1, state.colors.agent_urgent);
        assert_eq!(segments[1].0, " | ");
        assert_eq!(segments[2].0, "… Codex working · Bash");
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
        let segments = state.agent_status_segments(200);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].0, "… Codex working · Bash");
    }

    #[test]
    fn done_status_stays_until_the_pane_gains_focus() {
        let mut state = State::default();
        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "PermissionRequest".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));
        assert!(
            state.agent_statuses[&7].expires_at.is_none(),
            "blocked prompt must not fade on its own"
        );

        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "Stop".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(2),
        }));
        let status = &state.agent_statuses[&7];
        assert!(status.expires_at.is_none(), "done stays until viewed");
        assert!(status.clear_on_focus);

        let unrelated_focus = || PaneManifest {
            panes: HashMap::from([(
                0,
                vec![
                    PaneInfo {
                        id: 12,
                        is_focused: true,
                        ..PaneInfo::default()
                    },
                    PaneInfo {
                        id: 7,
                        ..PaneInfo::default()
                    },
                ],
            )]),
        };
        state.update(Event::PaneUpdate(unrelated_focus()));
        assert!(state.agent_statuses.contains_key(&7));

        let mut refocus = unrelated_focus();
        refocus.panes.get_mut(&0).unwrap()[0].is_focused = false;
        refocus.panes.get_mut(&0).unwrap()[1].is_focused = true;
        state.update(Event::PaneUpdate(refocus));
        assert!(
            state.agent_statuses.is_empty(),
            "viewing the pane clears its done status"
        );
    }

    #[test]
    fn vertical_agent_rows_register_click_focus_targets() {
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
            source: "Codex".to_string(),
            event: "PreToolUse".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(4, 30, &colors);
        state.render_vertical(&mut frame, 4, 30);

        assert_eq!(state.agent_focus_targets.len(), 1);
        let (line, start, pane_id) = state.agent_focus_targets[0];
        assert_eq!(line, 2, "the agent row sits below title and cwd");
        assert_eq!(pane_id, 7);
        assert_eq!(start, 0, "the whole agent row focuses the pane");
        assert_eq!(state.visible_vertical_tabs, vec![(0, 0)]);
    }

    #[test]
    fn rollup_glyph_marks_worst_state_and_summary_sticks() {
        let mut state = State::default();
        for (pane_id, event, summary) in [
            (7, "PreToolUse", Some("fix coding agent integration")),
            (8, "UserPromptSubmit", None),
        ] {
            state.panes.panes.entry(0).or_default().push(PaneInfo {
                id: pane_id,
                title: "novaid".to_string(),
                ..PaneInfo::default()
            });
            assert!(state.apply_agent_event(AgentEvent {
                source: "choco-pi".to_string(),
                event: event.to_string(),
                tool: None,
                summary: summary.map(str::to_string),
                pane_id,
                timestamp: Some(1),
            }));
        }

        let (glyph, urgent) = state.tab_rollup_glyph(0).unwrap();
        assert_eq!((glyph, urgent), ('…', false));

        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "Stop".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(2),
        }));
        let status = &state.agent_statuses[&7];
        assert_eq!(
            status.summary.as_deref(),
            Some("fix coding agent integration"),
            "the task label survives later events without one"
        );
        let (glyph, _) = state.tab_rollup_glyph(0).unwrap();
        assert_eq!(glyph, '…', "a working agent keeps the working glyph");

        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "Notification".to_string(),
            tool: None,
            summary: None,
            pane_id: 8,
            timestamp: Some(3),
        }));
        let (glyph, urgent) = state.tab_rollup_glyph(0).unwrap();
        assert_eq!((glyph, urgent), ('⚠', true));

        assert_eq!(state.agent_title_suffix(7).as_deref(), Some("novaid"));
        assert_eq!(state.agent_title_suffix(99), None);

        let pane = state.panes.panes.get_mut(&0).unwrap();
        pane[0].title.clear();
        pane[1].title = "zsh".to_string();
        assert_eq!(state.agent_title_suffix(7), None);
        assert_eq!(state.agent_title_suffix(8), None);
    }

    #[test]
    fn agent_rows_render_neutral_with_summary_suffix() {
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
                title: "novaid".to_string(),
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "choco-pi".to_string(),
            event: "PreToolUse".to_string(),
            tool: None,
            summary: Some("fix coding agent integration".to_string()),
            pane_id: 7,
            timestamp: Some(1),
        }));

        let colors = Colors::default();
        let mut frame = AnsiFrame::new(4, 80, &colors);
        state.render_vertical(&mut frame, 4, 80);
        let output = frame.finish();

        assert!(output.contains("▸p1 … choco-pi working"));
        assert!(
            output.contains("· fix coding agent integration"),
            "the summary suffix follows the status message"
        );
        let row_style = state.vertical_agent_row_style(false);
        assert_eq!(row_style.bg, colors.background, "agent rows stay neutral");
        assert_eq!(row_style.fg, colors.agent.bg);
        assert!(state.vertical_agent_row_style(true).bold);
    }

    #[test]
    fn foreground_agent_processes_are_detected_without_hooks() {
        let mut state = State::default();
        state.update(Event::PaneUpdate(PaneManifest {
            panes: HashMap::from([(
                0,
                vec![
                    PaneInfo {
                        id: 21,
                        terminal_command: Some("pi".to_string()),
                        ..PaneInfo::default()
                    },
                    PaneInfo {
                        id: 22,
                        terminal_command: Some("/opt/homebrew/bin/zsh -l".to_string()),
                        ..PaneInfo::default()
                    },
                ],
            )]),
        }));
        assert_eq!(
            state
                .agent_statuses
                .get(&21)
                .map(|status| status.message.as_str()),
            Some("◆ choco-pi detected"),
            "already-running command panes are detected from the manifest"
        );
        assert!(!state.agent_statuses.contains_key(&22));

        state.update(Event::CommandChanged(
            PaneId::Terminal(7),
            vec!["claude".to_string()],
            true,
            vec![],
        ));
        let status = state.agent_statuses.get(&7).unwrap();
        assert_eq!(status.message, "◆ Claude Code detected");
        assert!(status.detected);

        state.update(Event::CommandChanged(
            PaneId::Terminal(7),
            vec!["ls".to_string()],
            false,
            vec![],
        ));
        assert!(
            state.agent_statuses.contains_key(&7),
            "background command changes do not disturb detection"
        );

        assert!(state.apply_agent_event(AgentEvent {
            source: "Claude Code".to_string(),
            event: "PreToolUse".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));
        let status = state.agent_statuses.get(&7).unwrap();
        assert!(!status.detected, "hook events take over the pane's state");
        assert_eq!(status.message, "… Claude Code working");

        state.update(Event::CommandChanged(
            PaneId::Terminal(7),
            vec!["zsh".to_string()],
            true,
            vec![],
        ));
        assert!(
            state.agent_statuses.contains_key(&7),
            "hook-reported state survives the agent exiting"
        );

        state.update(Event::CommandChanged(
            PaneId::Terminal(9),
            vec!["/usr/local/bin/opencode".to_string()],
            true,
            vec![],
        ));
        assert_eq!(
            state.agent_statuses.get(&9).unwrap().message,
            "◆ OpenCode detected"
        );
        state.update(Event::CommandChanged(
            PaneId::Terminal(9),
            vec!["zsh".to_string()],
            true,
            vec![],
        ));
        assert!(
            !state.agent_statuses.contains_key(&9),
            "detected placeholder disappears when the agent exits"
        );
    }

    #[test]
    fn horizontal_status_line_fits_entries_and_counts_overflow() {
        let mut state = State::default();
        for (pane_id, source, event, timestamp) in [
            (7, "Codex", "PreToolUse", 1),
            (12, "Claude Code", "Notification", 2),
        ] {
            assert!(state.apply_agent_event(AgentEvent {
                source: source.to_string(),
                event: event.to_string(),
                tool: None,
                summary: None,
                pane_id,
                timestamp: Some(timestamp),
            }));
        }

        let fits_both = state
            .agent_status_segments(60)
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>();
        assert_eq!(
            fits_both,
            ["● Claude Code notification", " | ", "… Codex working"]
        );

        let limited = state.agent_status_segments(40);
        let total: usize = limited.iter().map(|(text, _)| cell_width(text)).sum();
        assert!(total <= 40);
        assert_eq!(limited.last().unwrap().0, " +1");

        let tiny = state.agent_status_segments(10);
        let total: usize = tiny.iter().map(|(text, _)| cell_width(text)).sum();
        assert!(total <= 10);
        assert_eq!(
            tiny.len(),
            1,
            "a lone entry truncates instead of overflowing"
        );
        assert!(tiny[0].0.ends_with('…'));
    }

    #[test]
    fn agent_rows_and_segments_follow_tab_then_pane_order() {
        let mut state = State::default();
        for (tab_position, pane_id, event, timestamp) in [
            (0, 7, "PreToolUse", 1),
            (0, 8, "PermissionRequest", 2),
            (1, 12, "Stop", 3),
        ] {
            state
                .panes
                .panes
                .entry(tab_position)
                .or_default()
                .push(PaneInfo {
                    id: pane_id,
                    ..PaneInfo::default()
                });
            assert!(state.apply_agent_event(AgentEvent {
                source: "Codex".to_string(),
                event: event.to_string(),
                tool: None,
                summary: None,
                pane_id,
                timestamp: Some(timestamp),
            }));
        }

        let rows = state.agent_rows_for_tab(0);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .map(|(index, pane_id, status)| (*index, *pane_id, status.urgent))
                .collect::<Vec<_>>(),
            [(0, 7, false), (1, 8, true)],
            "rows follow pane order within the tab"
        );
        let rows = state.agent_rows_for_tab(1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 0);
        assert_eq!(rows[0].1, 12);
        assert!(state.agent_rows_for_tab(2).is_empty());

        assert_eq!(state.pane_location(8), Some((0, 1)));
        assert_eq!(state.pane_location(99), None);
        let rendered = state
            .agent_status_segments(500)
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            [
                "[1·1] … Codex working",
                " | ",
                "[1·2] ⚠ Codex permission required",
                " | ",
                "[2·1] ✓ Codex response complete",
            ]
        );
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
        assert!(permissions_for_view(View::Horizontal).contains(&PermissionType::RunCommands));
        assert!(!permissions_for_view(View::Vertical).contains(&PermissionType::RunCommands));
        for view in [View::Horizontal, View::Vertical] {
            assert!(
                permissions_for_view(view).contains(&PermissionType::ReadCliPipes),
                "both views consume coding-agent pipe events"
            );
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

    #[test]
    fn vertical_window_keeps_active_visible_and_respects_rows() {
        let mut state = State::default();
        state.tabs = (0..5)
            .map(|position| TabInfo {
                position,
                active: position == 2,
                ..TabInfo::default()
            })
            .collect();
        // tab 1 hosts one agent, so it costs 3 rows instead of 2
        state.panes.panes.insert(
            1,
            vec![PaneInfo {
                id: 7,
                ..PaneInfo::default()
            }],
        );
        assert!(state.apply_agent_event(AgentEvent {
            source: "Codex".to_string(),
            event: "PreToolUse".to_string(),
            tool: None,
            summary: None,
            pane_id: 7,
            timestamp: Some(1),
        }));

        // 8 rows: tab1 (3) + tab2 (2) + tab0 or tab3 (2 each, both fit only once more)
        let window = state.vertical_window(2, 8);
        assert!(window.contains(&2), "the active tab stays visible");
        let used: usize = window
            .iter()
            .map(|i| state.vertical_tab_height(state.tabs[*i].position))
            .sum();
        assert!(used <= 8);

        let window = state.vertical_window(4, 2);
        assert_eq!(
            window,
            vec![4],
            "a too-short view still shows the active tab"
        );

        state.tabs = Vec::new();
        assert!(state.vertical_window(0, 10).is_empty());
    }
}
