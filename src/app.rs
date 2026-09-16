use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use zellij_tile::prelude::*;

use crate::agent::*;
use crate::music::*;
use crate::spaces::{self, Space};
use crate::ui::Style;
use crate::ui::*;

const AGENT_PIPE: &str = "coding-agent-status";
const AGENT_FOCUS_PIPE: &str = "coding-agent-status:focus";
const SPACE_PIPE_PREFIX: &str = "vtabs:";
const SPACE_PIPE_LOG: &str = "/tmp/zellij-vtabs-pipe.log";
/// Long enough to cover the spread between instances receiving one broadcast,
/// short enough to keep key repeat responsive.
const SPACE_CLAIM_WINDOW_MS: u64 = 150;
const DEBUG_TRIGGER_PATH: &str = "/host/.zellij-vtabs-debug";
const WIDTH_SYNC_MAX_ATTEMPTS: u8 = 64;
const GIT_COMMAND: &str =
    "root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 1; printf '%s\n' \"$root\"; git status --porcelain=v1 --branch";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum View {
    #[default]
    Horizontal,
    Vertical,
    /// One-line tab strip for the active space, meant to sit at the top of the
    /// space's content so the status bar keeps its room for agents and music.
    Tabs,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) plugin_id: Option<u32>,
    pub(crate) view: View,
    pub(crate) mode: InputMode,
    pub(crate) session_name: String,
    pub(crate) tabs: Vec<TabInfo>,
    pub(crate) panes: PaneManifest,
    pub(crate) active_pane_id: Option<PaneId>,
    pub(crate) active_cwd: Option<PathBuf>,
    pub(crate) active_command: Option<String>,
    pub(crate) cwd_by_tab: HashMap<usize, PathBuf>,
    pub(crate) repo_by_tab: HashMap<usize, RepoInfo>,
    pub(crate) repo_cwd_by_tab: HashMap<usize, PathBuf>,
    /// The shared status file's content as this instance last saw it, so a write
    /// happens only on a real change and a read only on a foreign one.
    pub(crate) agent_sync_payload: Option<String>,
    pub(crate) cwd_error: Option<String>,
    pub(crate) configured_home: Option<PathBuf>,
    pub(crate) git_context: Option<GitContext>,
    pub(crate) git_refresh_pending: bool,
    pub(crate) permissions_granted: bool,
    pub(crate) agent_statuses: HashMap<u32, AgentStatus>,
    pub(crate) agent_sequence: u64,
    pub(crate) focused_terminal_pane: Option<u32>,
    pub(crate) agent_focus_targets: Vec<(usize, usize, u32)>,
    pub(crate) timer_ticks: u8,
    pub(crate) git_refresh_interval: u8,
    pub(crate) timezone_offset_hours: i32,
    pub(crate) datetime_format: String,
    pub(crate) right_panel: RightPanel,
    pub(crate) center_anchor: CenterAnchor,
    pub(crate) music_format: String,
    pub(crate) music_max_width: usize,
    /// Unknown until `uname` answers; Apple Music is only polled on Darwin.
    pub(crate) host_is_macos: Option<bool>,
    pub(crate) now_playing: Option<NowPlaying>,
    pub(crate) music_refresh_pending: bool,
    pub(crate) music_next_poll_at: u64,
    pub(crate) show_tabs: bool,
    pub(crate) spaces_enabled: bool,
    pub(crate) space_separator: String,
    pub(crate) default_space_name: String,
    pub(crate) zellij_pid: Option<u32>,
    pub(crate) last_focused_tab_by_space: HashMap<String, usize>,
    pub(crate) visible_vertical_spaces: Vec<(usize, usize)>,
    /// Columns of the new-tab button on the tab strip.
    pub(crate) plus_hitbox: Option<(usize, usize)>,
    pub(crate) border_enabled: bool,
    pub(crate) border_char: String,
    pub(crate) vertical_separator_enabled: bool,
    pub(crate) vertical_separator_char: String,
    pub(crate) initial_sidebar_width: Option<usize>,
    pub(crate) colors: Colors,
    pub(crate) visible_vertical_tabs: Vec<(usize, usize)>,
    pub(crate) visible_horizontal_tabs: Vec<TabHitbox>,
    pub(crate) last_hook_timestamp_by_pane: HashMap<u32, u64>,
    pub(crate) session_end_timestamp_by_pane: HashMap<u32, u64>,
    pub(crate) pending_width_sync: Option<PendingWidthSync>,
    pub(crate) last_observed_sidebar_width: Option<usize>,
    pub(crate) last_observed_tab_width: Option<usize>,
    pub(crate) tabs_with_user_content: HashSet<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingWidthSync {
    pub(crate) target_width: usize,
    pub(crate) pane_ids: Vec<u32>,
    pub(crate) last_requested_widths: HashMap<u32, usize>,
    pub(crate) attempts_remaining: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitContext {
    pub(crate) cwd: PathBuf,
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) dirty: bool,
}

/// Repository identity for a tab, read straight from the git directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoInfo {
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) worktree: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct TabHitbox {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) position: usize,
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        let ids = get_plugin_ids();
        self.plugin_id = Some(ids.plugin_id);
        self.zellij_pid = Some(ids.zellij_pid);
        // Stay focusable until permissions are granted: zellij renders its
        // permission prompt inside this pane and withholds every event until it
        // is answered, which is impossible on a pane nobody can focus.
        set_selectable(true);
        self.view = match configuration.get("view").map(String::as_str) {
            Some("vertical") => View::Vertical,
            Some("tabs") => View::Tabs,
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
        self.right_panel = RightPanel::from_config(&configuration);
        self.center_anchor = CenterAnchor::from_config(&configuration);
        self.music_format = configured_music_format(&configuration);
        self.music_max_width = configured_music_max_width(&configuration);
        self.show_tabs = configuration
            .get("show_tabs")
            .is_none_or(|value| value != "false");
        self.spaces_enabled = configuration
            .get("spaces")
            .is_some_and(|value| value == "true");
        self.space_separator = configuration
            .get("space_separator")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| spaces::DEFAULT_SEPARATOR.to_string());
        self.default_space_name = configuration
            .get("default_space_name")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| spaces::DEFAULT_SPACE_NAME.to_string());
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
        self.initial_sidebar_width = configuration
            .get("initial_width")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|width| *width > 0);
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
                self.prune_closed_tab_state();
                self.remember_active_space_tab();
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
                match context.get("kind").map(String::as_str) {
                    Some("uname") => {
                        let stdout = String::from_utf8_lossy(&stdout);
                        self.host_is_macos =
                            Some(exit_code == Some(0) && stdout.trim() == "Darwin");
                        self.music_refresh_pending = false;
                        self.refresh_music();
                        return false;
                    }
                    Some("music") => {
                        self.music_refresh_pending = false;
                        let previous = self.now_playing.take();
                        if exit_code == Some(0) {
                            self.now_playing = parse_now_playing(&String::from_utf8_lossy(&stdout));
                        }
                        // A failing osascript (typically a denied automation
                        // prompt) backs off further so it cannot spam the host.
                        let delay = if exit_code == Some(0) {
                            next_poll_delay_seconds(self.now_playing.as_ref())
                        } else {
                            30
                        };
                        self.music_next_poll_at = unix_seconds() + delay;
                        return previous != self.now_playing;
                    }
                    _ => {}
                }
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
                self.refresh_music();
                if std::path::Path::new(DEBUG_TRIGGER_PATH).exists() {
                    let view = match self.view {
                        View::Horizontal => "horizontal",
                        View::Vertical => "vertical",
                        View::Tabs => "tabs",
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
                        if let Some((_, space_index)) = self
                            .visible_vertical_spaces
                            .iter()
                            .find(|(space_line, _)| *space_line == line_index)
                        {
                            let space_index = *space_index;
                            self.switch_to_space(space_index + 1);
                        } else if let Some((_, _, pane_id)) =
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
                        if let Some(hitbox) = self
                            .visible_horizontal_tabs
                            .iter()
                            .find(|hitbox| column >= hitbox.start && column < hitbox.end)
                        {
                            switch_tab_to((hitbox.position + 1) as u32);
                        }
                    }
                    View::Tabs => {
                        let plus = self
                            .plus_hitbox
                            .is_some_and(|(start, end)| column >= start && column < end);
                        if plus {
                            self.create_tab_in_active_space();
                        } else if let Some(hitbox) = self
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
        if let Some(command) = pipe_message.name.strip_prefix(SPACE_PIPE_PREFIX) {
            let command = command.to_string();
            let payload = pipe_message.payload.clone();
            return self.handle_space_command(&command, payload.as_deref());
        }
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
            View::Tabs => self.render_space_tabs(&mut frame, rows, cols),
        }
        print!("{}", frame.finish());
    }
}

impl State {
    pub(crate) fn separator(&self) -> &str {
        if self.space_separator.is_empty() {
            spaces::DEFAULT_SEPARATOR
        } else {
            &self.space_separator
        }
    }

    pub(crate) fn default_space(&self) -> &str {
        if self.default_space_name.is_empty() {
            spaces::DEFAULT_SPACE_NAME
        } else {
            &self.default_space_name
        }
    }

    pub(crate) fn space_model(&self) -> Vec<Space> {
        spaces::group_spaces(&self.tabs, self.separator(), self.default_space())
    }

    /// Indices into `self.tabs` that belong to the active space; every tab when
    /// spaces are disabled.
    pub(crate) fn visible_tab_indices(&self) -> Vec<usize> {
        if !self.spaces_enabled {
            return (0..self.tabs.len()).collect();
        }
        let model = self.space_model();
        let Some(active) =
            spaces::active_space_index(&model).or(if model.is_empty() { None } else { Some(0) })
        else {
            return (0..self.tabs.len()).collect();
        };
        let tab_ids: HashSet<usize> = model[active].tabs.iter().map(|tab| tab.tab_id).collect();
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab_ids.contains(&tab.tab_id))
            .map(|(index, _)| index)
            .collect()
    }

    /// Bar entries for the visible tabs. The raw name stays visible while the
    /// user is renaming a tab, since that is the text being edited.
    pub(crate) fn tab_bar_labels(&self, view: &[usize]) -> Vec<String> {
        if !self.spaces_enabled || self.mode == InputMode::RenameTab {
            return view
                .iter()
                .map(|index| tab_label(&self.tabs[*index]))
                .collect();
        }
        view.iter()
            .enumerate()
            .map(|(slot, index)| {
                let tab = &self.tabs[*index];
                let (_, label) =
                    spaces::split_tab_name(&tab.name, self.separator(), self.default_space());
                let label = if label.is_empty() {
                    tab_name(tab)
                } else {
                    label
                };
                numbered_tab_label(
                    slot,
                    &label,
                    tab.has_bell_notification || tab.is_flashing_bell,
                )
            })
            .collect()
    }

    /// Tabs and spaces disappear; their remembered state must not outlive them.
    pub(crate) fn prune_closed_tab_state(&mut self) {
        let live: HashSet<usize> = self.tabs.iter().map(|tab| tab.tab_id).collect();
        self.tabs_with_user_content
            .retain(|tab_id| live.contains(tab_id));
        let known: HashSet<String> = self
            .space_model()
            .into_iter()
            .map(|space| space.name)
            .collect();
        self.last_focused_tab_by_space
            .retain(|name, tab_id| known.contains(name) && live.contains(tab_id));
    }

    pub(crate) fn remember_active_space_tab(&mut self) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.active) else {
            return;
        };
        let (space, _) = spaces::split_tab_name(&tab.name, self.separator(), self.default_space());
        let tab_id = tab.tab_id;
        // A tab update only reaches the plugins of the focused tab, so the
        // active entry really is the focused one whenever this runs.
        if self.last_focused_tab_by_space.get(&space) == Some(&tab_id) {
            return;
        }
        self.last_focused_tab_by_space.insert(space, tab_id);
        self.persist_focus_memory();
    }

    /// Session-wide file holding the tab each space was last left on. The
    /// instance that performs the next switch lives in another tab and never
    /// observed the previous focus, so this cannot be kept in memory alone.
    pub(crate) fn focus_memory_path(&self) -> Option<PathBuf> {
        self.zellij_pid
            .map(|pid| PathBuf::from(format!("/tmp/zellij-vtabs-focus-{pid}")))
    }

    pub(crate) fn load_focus_memory(&mut self) {
        let Some(path) = self.focus_memory_path() else {
            return;
        };
        if let Ok(raw) = std::fs::read_to_string(&path) {
            for (space, tab_id) in spaces::parse_focus_memory(&raw) {
                self.last_focused_tab_by_space
                    .entry(space)
                    .or_insert(tab_id);
            }
        }
    }

    pub(crate) fn persist_focus_memory(&self) {
        let Some(path) = self.focus_memory_path() else {
            return;
        };
        let encoded = spaces::format_focus_memory(&self.last_focused_tab_by_space);
        // Rename so a reader never sees half a file.
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, encoded).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }

    pub(crate) fn focus_tab_id(&self, tab_id: usize) {
        // Positions shift when an earlier tab closes, so resolve the id now.
        if let Some(tab) = self.tabs.iter().find(|tab| tab.tab_id == tab_id) {
            switch_tab_to(tab.position as u32 + 1);
        }
    }

    pub(crate) fn switch_to_space(&mut self, index: usize) {
        self.load_focus_memory();
        let model = self.space_model();
        if let Some(tab_id) =
            spaces::resolve_space_switch(&model, index, &self.last_focused_tab_by_space)
        {
            self.focus_tab_id(tab_id);
        }
    }

    /// A keybind pipe reaches every instance in every tab, so only the instance
    /// holding current state may act on it: see spaces::instance_is_in_focused_tab.
    pub(crate) fn handle_space_command(&mut self, command: &str, payload: Option<&str>) -> bool {
        self.log_space_command(command, payload);
        if !self.has_command_authority() {
            return false;
        }
        if !self.claim_command(command, payload) {
            return false;
        }
        if !self.spaces_enabled {
            self.run_space_fallback(command, payload);
            return false;
        }
        self.load_focus_memory();
        let focused = self.tabs.iter().find(|tab| tab.active).cloned();
        let model = self.space_model();
        let active_index = focused
            .as_ref()
            .map(|tab| spaces::split_tab_name(&tab.name, self.separator(), self.default_space()).0)
            .and_then(|name| model.iter().position(|space| space.name == name))
            .or_else(|| spaces::active_space_index(&model));
        let cwd = focused
            .as_ref()
            .and_then(|tab| self.cwd_by_tab.get(&tab.position))
            .map(|cwd| cwd.display().to_string());
        match command {
            "space-new" => {
                let name = spaces::next_space_name(&model);
                let tab_name = format!("{}{}1", name, self.separator());
                new_tab(Some(tab_name), cwd);
            }
            "tab-new" => {
                let tab_name = active_index
                    .and_then(|index| model.get(index))
                    .map(|space| spaces::next_tab_name(space, self.separator()));
                new_tab(tab_name, cwd);
            }
            "space-switch" => {
                if let Some(index) = payload.and_then(|value| value.trim().parse::<usize>().ok()) {
                    if let Some(tab_id) =
                        spaces::resolve_space_switch(&model, index, &self.last_focused_tab_by_space)
                    {
                        self.focus_tab_id(tab_id);
                    }
                }
            }
            "tab-next" | "tab-prev" => {
                if let Some(space) = active_index.and_then(|index| model.get(index)) {
                    if let Some(tab_id) = spaces::resolve_tab_cycle(
                        space,
                        focused.as_ref().map(|tab| tab.tab_id),
                        command == "tab-next",
                    ) {
                        self.focus_tab_id(tab_id);
                    }
                }
            }
            _ => {}
        }
        false
    }

    /// True only for the instance of the focused tab that owns the action; every
    /// other instance would decide from a tab list frozen at its last focus.
    pub(crate) fn has_command_authority(&self) -> bool {
        let Ok((focused_tab_id, _)) = get_focused_pane_info() else {
            // Without an authoritative answer, dropping the keypress is safer
            // than several instances acting on stale lists.
            return false;
        };
        if !spaces::instance_is_in_focused_tab(&self.tabs, focused_tab_id) {
            return false;
        }
        let Some(position) = self
            .tabs
            .iter()
            .find(|tab| tab.tab_id == focused_tab_id)
            .map(|tab| tab.position)
        else {
            return false;
        };
        spaces::view_acts(&self.panes, position, self.view == View::Vertical)
    }

    /// Reserves this command against the other instances of the session.
    pub(crate) fn claim_command(&self, command: &str, payload: Option<&str>) -> bool {
        let Some(plugin_id) = self.plugin_id else {
            return true;
        };
        let Some(path) = self
            .zellij_pid
            .map(|pid| PathBuf::from(format!("/tmp/zellij-vtabs-claim-{pid}")))
        else {
            return true;
        };
        let payload = payload.unwrap_or_default();
        let now = unix_millis();
        let existing = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| spaces::parse_claim(&raw));
        if spaces::claim_blocks(
            existing.as_ref(),
            plugin_id,
            command,
            payload,
            now,
            SPACE_CLAIM_WINDOW_MS,
        ) {
            return false;
        }
        let temporary = path.with_extension("tmp");
        if std::fs::write(
            &temporary,
            spaces::format_claim(plugin_id, command, payload, now),
        )
        .is_ok()
        {
            let _ = std::fs::rename(&temporary, &path);
        }
        true
    }

    /// With spaces disabled the same keybinds keep their plain Zellij meaning.
    pub(crate) fn run_space_fallback(&self, command: &str, payload: Option<&str>) {
        match command {
            "space-new" | "tab-new" => {
                new_tab(None::<String>, None::<String>);
            }
            "space-switch" => {
                if let Some(index) = payload.and_then(|value| value.trim().parse::<u32>().ok()) {
                    switch_tab_to(index);
                }
            }
            "tab-next" => go_to_next_tab(),
            "tab-prev" => go_to_previous_tab(),
            _ => {}
        }
    }

    /// Fan-out and election are only observable from inside the instances, so
    /// the debug trigger records what each one received.
    pub(crate) fn log_space_command(&self, command: &str, payload: Option<&str>) {
        if !std::path::Path::new(DEBUG_TRIGGER_PATH).exists() {
            return;
        }
        let view = match self.view {
            View::Horizontal => "horizontal",
            View::Vertical => "vertical",
            View::Tabs => "tabs",
        };
        let plugin_id = self.plugin_id.unwrap_or(0);
        let leader = self.has_command_authority();
        let line = format!(
            "{} {} {} plugin={} view={} leader={} tabs={} active={:?}\n",
            unix_seconds(),
            command,
            payload.unwrap_or("-"),
            plugin_id,
            view,
            leader,
            self.tabs.len(),
            self.tabs
                .iter()
                .find(|tab| tab.active)
                .map(|tab| (tab.tab_id, tab.name.clone())),
        );
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(SPACE_PIPE_LOG)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    pub(crate) fn observe_sidebar_width_change(&mut self) {
        if self.view != View::Vertical {
            return;
        }
        let Some(plugin_id) = self.plugin_id else {
            return;
        };
        let Some((width, _)) = active_sidebar_state(&self.tabs, &self.panes, plugin_id) else {
            return;
        };
        let Some(tab_width) = active_tab_width(&self.tabs, &self.panes) else {
            return;
        };
        let previous_width = self.last_observed_sidebar_width.replace(width);
        let previous_tab_width = self.last_observed_tab_width.replace(tab_width);
        if self.pending_width_sync.is_some() {
            return;
        }
        let target_width = match previous_width {
            None => self.initial_sidebar_width.filter(|target| *target != width),
            Some(previous) if previous_tab_width.is_some_and(|value| value != tab_width) => {
                (previous != width).then_some(previous)
            }
            Some(previous) if previous != width => Some(width),
            Some(_) => None,
        };
        let Some(target_width) = target_width else {
            return;
        };

        let mut pane_ids = visible_vertical_sidebar_ids(&self.panes);
        if !pane_ids.contains(&plugin_id) {
            pane_ids.push(plugin_id);
            pane_ids.sort_unstable();
        }
        self.pending_width_sync = Some(PendingWidthSync {
            target_width,
            pane_ids,
            last_requested_widths: HashMap::new(),
            attempts_remaining: WIDTH_SYNC_MAX_ATTEMPTS,
        });
    }

    pub(crate) fn close_empty_own_tab_if_needed(&mut self) {
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

    pub(crate) fn sync_sidebar_widths(&mut self) {
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

    pub(crate) fn handle_agent_event(&mut self, event: AgentEvent) -> bool {
        if let Some(session_end_timestamp) = self
            .session_end_timestamp_by_pane
            .get(&event.pane_id)
            .copied()
        {
            if event.event != "SessionStart"
                || event
                    .timestamp
                    .is_none_or(|timestamp| timestamp < session_end_timestamp)
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

    pub(crate) fn render_horizontal(&mut self, frame: &mut AnsiFrame, rows: usize, cols: usize) {
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
        let music = self.music_content();
        let (center_context, right_context, right_clock) = horizontal_content_split(
            self.show_tabs,
            !self.agent_statuses.is_empty(),
            &context,
            &clock,
        );
        let left_width = cell_width(&left).min(cols);
        let available_right = cols.saturating_sub(left_width);
        // Music may widen the right side by up to another fifth of the bar, so
        // it does not have to fit in the space context and clock already fill.
        let max_right =
            available_right.saturating_mul(2) / 5 + cell_width(&music).min(available_right / 5);
        let clock_width = cell_width(&right_clock).min(available_right);
        let right_width = cell_width(&right_context)
            .saturating_add(cell_width(&music))
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
            let (context, music, clock) =
                fit_right_segments(&right_context, &music, &right_clock, right_width);
            let right_start = cols - right_width;
            frame.put(right_start, 0, self.colors.context, &context);
            let music_start = right_start + cell_width(&context);
            frame.put(music_start, 0, self.colors.music, &music);
            frame.put(
                music_start + cell_width(&music),
                0,
                self.colors.clock,
                &clock,
            );
        }

        let center_start = left_width.saturating_add(1);
        let center_end = cols.saturating_sub(right_width.saturating_add(1));
        if center_end > center_start {
            let available = center_end - center_start;
            let anchor = self.center_anchor_column(cols);
            if !self.agent_statuses.is_empty() {
                let segments = self.agent_status_segments(available);
                let total_width = segments
                    .iter()
                    .map(|(text, _)| cell_width(text))
                    .sum::<usize>()
                    .min(available);
                let mut x = horizontal_group_start(anchor, total_width, center_start, center_end);
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
                let x = horizontal_group_start(anchor, width, center_start, center_end);
                frame.put(x, 0, self.colors.context, rendered.trim_end());
            } else {
                self.render_horizontal_tabs(frame, anchor, center_start, center_end);
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

    /// Screen column the centered group is balanced around.
    pub(crate) fn center_anchor_column(&self, cols: usize) -> usize {
        match self.center_anchor {
            CenterAnchor::Bar => cols / 2,
            CenterAnchor::Content => content_center(&self.tabs, &self.panes, self.plugin_id)
                .filter(|center| *center < cols)
                .unwrap_or(cols / 2),
        }
    }

    pub(crate) fn render_horizontal_tabs(
        &mut self,
        frame: &mut AnsiFrame,
        anchor: usize,
        left_bound: usize,
        right_bound: usize,
    ) {
        let width = right_bound.saturating_sub(left_bound);
        let view = self.visible_tab_indices();
        if view.is_empty() {
            return;
        }
        let labels = self.tab_bar_labels(&view);
        // The active index must be the one inside the filtered view: a tab that
        // is momentarily outside it would otherwise highlight the first entry.
        let active = view
            .iter()
            .position(|index| self.tabs[*index].active)
            .unwrap_or(0);
        let candidates = horizontal_visible_indices(&labels, active, width);
        let mut remaining = width;
        let mut rendered_tabs = Vec::with_capacity(candidates.len());
        for slot in candidates {
            if remaining == 0 {
                break;
            }
            let rendered = truncate_line(&labels[slot], remaining.min(24));
            remaining = remaining.saturating_sub(cell_width(&rendered));
            rendered_tabs.push((view[slot], rendered));
        }
        let total_width = rendered_tabs
            .iter()
            .map(|(_, rendered)| cell_width(rendered))
            .sum();
        let mut x = horizontal_group_start(anchor, total_width, left_bound, right_bound);
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

    /// The space's tab strip: tabs from the left edge with the new-tab button
    /// directly after the last one, all on a single line.
    pub(crate) fn render_space_tabs(&mut self, frame: &mut AnsiFrame, rows: usize, cols: usize) {
        self.visible_horizontal_tabs.clear();
        self.plus_hitbox = None;
        let background = Style {
            fg: self.colors.tab_normal.fg,
            bg: self.colors.background,
            bold: false,
        };
        for y in 0..rows {
            frame.put(0, y, background, &fit_line("", cols));
        }
        if cols == 0 {
            return;
        }

        let plus = " + ";
        let plus_width = cell_width(plus);
        // The button trails the tabs, so it only takes the room they leave.
        let room = cols;
        let view = self.visible_tab_indices();
        let labels = self.tab_bar_labels(&view);
        let mut x = 0;
        for (slot, label) in labels.iter().enumerate() {
            if x >= room {
                break;
            }
            let rendered = truncate_line(label, (room - x).min(24));
            let rendered_width = cell_width(&rendered);
            if rendered_width == 0 {
                break;
            }
            let tab = &self.tabs[view[slot]];
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

        if cols.saturating_sub(x) >= plus_width {
            frame.put(x, 0, self.colors.tab_normal, plus);
            self.plus_hitbox = Some((x, x + plus_width));
        }
    }

    /// New tab in the space the focused tab belongs to; a plain new tab when
    /// spaces are off.
    pub(crate) fn create_tab_in_active_space(&mut self) {
        let focused = self.tabs.iter().find(|tab| tab.active).cloned();
        let cwd = focused
            .as_ref()
            .and_then(|tab| self.cwd_by_tab.get(&tab.position))
            .map(|cwd| cwd.display().to_string());
        if !self.spaces_enabled {
            new_tab(None::<String>, cwd);
            return;
        }
        let model = self.space_model();
        let name = focused
            .as_ref()
            .map(|tab| spaces::split_tab_name(&tab.name, self.separator(), self.default_space()).0)
            .and_then(|name| model.iter().find(|space| space.name == name))
            .map(|space| spaces::next_tab_name(space, self.separator()));
        new_tab(name, cwd);
    }

    pub(crate) fn render_vertical(&mut self, frame: &mut AnsiFrame, rows: usize, cols: usize) {
        self.visible_vertical_tabs.clear();
        self.visible_vertical_spaces.clear();
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
        let model = if self.spaces_enabled {
            self.space_model()
        } else {
            Vec::new()
        };
        let budget = rows.saturating_sub(agent_budget);
        // With spaces on, the sidebar lists spaces in the card the tabs used to
        // occupy: the tabs of the active space live in the horizontal bar.
        let y = if model.is_empty() {
            self.render_vertical_tabs(frame, 0, budget, content_cols)
        } else {
            self.render_vertical_space_cards(frame, &model, budget, content_cols)
        };
        if agents > 0 {
            self.render_vertical_agents(frame, y, rows, content_cols);
        }
    }

    /// Space cards, in the same two-row shape the tab cards use: the name with
    /// its number, then the repository and working directory of the tab the
    /// space is currently on.
    pub(crate) fn render_vertical_space_cards(
        &mut self,
        frame: &mut AnsiFrame,
        model: &[Space],
        budget: usize,
        content_cols: usize,
    ) -> usize {
        if budget < 3 {
            return 0;
        }
        let dim = Style {
            fg: self.colors.cwd_normal.fg,
            bg: self.colors.background,
            bold: false,
        };
        let heading = Style { bold: true, ..dim };
        frame.put(0, 0, dim, &fit_line("", content_cols));
        frame.put(0, 0, heading, " Spaces");
        let tabs: usize = model.iter().map(|space| space.tabs.len()).sum();
        let totals = totals_label(
            model.len(),
            "space",
            tabs,
            "tab",
            content_cols.saturating_sub(cell_width(" Spaces") + 2),
        );
        frame.put(
            content_cols.saturating_sub(cell_width(&totals) + 1),
            0,
            dim,
            &totals,
        );

        let active = spaces::active_space_index(model);
        let window = vertical_tab_window(model.len(), active.unwrap_or(0), (budget - 1) / 2);
        let mut y = 1;
        for index in window {
            if y + 2 > budget {
                break;
            }
            let space = &model[index];
            let is_active = Some(index) == active;
            let (title_style, cwd_style) = vertical_styles(&self.colors, is_active);
            let cwd_style = Style {
                bold: false,
                ..cwd_style
            };
            let marker = if is_active { "▸" } else { " " };
            // The card describes the tab the space sits on, which is the one a
            // click returns to.
            let current = space
                .tabs
                .iter()
                .find(|tab| tab.active)
                .or_else(|| {
                    self.last_focused_tab_by_space
                        .get(&space.name)
                        .and_then(|tab_id| space.tabs.iter().find(|tab| tab.tab_id == *tab_id))
                })
                .or_else(|| space.tabs.first());
            let title = fit_line(
                &format!("{marker}{} {}", index + 1, space.name),
                content_cols,
            );
            let repo = current.and_then(|tab| self.repo_by_tab.get(&tab.position));
            let cwd = current
                .and_then(|tab| self.cwd_by_tab.get(&tab.position))
                .map(|path| display_path(path, self.configured_home.as_deref()));
            let detail = tab_detail_line(repo, cwd.as_deref());
            let detail = fit_line(&format!("   {detail}"), content_cols);
            frame.put(0, y, title_style, &title);
            frame.put(0, y + 1, cwd_style, &detail);
            self.visible_vertical_spaces.push((y, index));
            self.visible_vertical_spaces.push((y + 1, index));
            y += 2;
        }
        y
    }

    /// Terminal panes of the listed tabs, so the counts match the visible list.
    /// Plugin panes are chrome, not content.
    pub(crate) fn pane_total_for(&self, view: &[usize]) -> usize {
        let positions: HashSet<usize> = view
            .iter()
            .filter_map(|index| self.tabs.get(*index))
            .map(|tab| tab.position)
            .collect();
        self.panes
            .panes
            .iter()
            .filter(|(position, _)| positions.contains(position))
            .flat_map(|(_, panes)| panes)
            .filter(|pane| !pane.is_plugin && !pane.is_suppressed)
            .count()
    }

    /// Tab cards: index and name on the first row, working directory below.
    pub(crate) fn render_vertical_tabs(
        &mut self,
        frame: &mut AnsiFrame,
        start: usize,
        budget: usize,
        content_cols: usize,
    ) -> usize {
        if budget < 3 || self.tabs.is_empty() {
            return start;
        }
        let view = self.visible_tab_indices();
        if view.is_empty() {
            return start;
        }
        let dim = Style {
            fg: self.colors.cwd_normal.fg,
            bg: self.colors.background,
            bold: false,
        };
        // The section name is the row's heading; its totals stay quiet beside it.
        let heading = Style { bold: true, ..dim };
        frame.put(0, start, dim, &fit_line("", content_cols));
        frame.put(0, start, heading, " Tabs");
        // The counts say how much of the session is off screen when the window
        // shows only part of it.
        let totals = tab_totals_label(
            view.len(),
            self.pane_total_for(&view),
            content_cols.saturating_sub(cell_width(" Tabs") + 2),
        );
        frame.put(
            content_cols.saturating_sub(cell_width(&totals) + 1),
            start,
            dim,
            &totals,
        );

        let active_index = view
            .iter()
            .position(|index| self.tabs[*index].active)
            .unwrap_or(0);
        let window = vertical_tab_window(view.len(), active_index, (budget - 1) / 2);

        let mut y = start + 1;
        for slot in window {
            if y + 2 > start + budget {
                break;
            }
            let index = view[slot];
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
            let display_index = if self.spaces_enabled {
                slot
            } else {
                tab.position
            };
            let display_name = if self.spaces_enabled && self.mode != InputMode::RenameTab {
                let (_, label) =
                    spaces::split_tab_name(&tab.name, self.separator(), self.default_space());
                if label.is_empty() {
                    tab_display_name(tab, repo)
                } else {
                    label
                }
            } else {
                tab_display_name(tab, repo)
            };
            let title = fit_line(
                &format!("{marker}{} {}", display_index + 1, display_name),
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

    /// Agent cards: state dot with location and session name, then a dim row
    /// with the state, coding-agent name and elapsed time.
    pub(crate) fn render_vertical_agents(
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
            let room = content_cols.saturating_sub(used);
            let separators = cell_width(" · ") + cell_width(" · ");
            let reserved = separators + cell_width(&entry.elapsed);
            if room > reserved {
                let agent_name = truncate_line(&entry.agent_name, room - reserved);
                let detail_style = Style {
                    fg: if focused { detail_style.fg } else { dim.fg },
                    bg: row_bg,
                    bold: false,
                };
                frame.put(
                    used,
                    y + 1,
                    detail_style,
                    &format!(" · {agent_name} · {}", entry.elapsed),
                );
            }
            self.agent_focus_targets.push((y + 1, 0, entry.pane_id));
            y += 2;
        }
        y
    }

    /// One presentable card per tracked agent, ordered by tab then pane.
    pub(crate) fn agent_entries(&self) -> Vec<AgentEntry> {
        let now = unix_seconds();
        self.sorted_agent_statuses()
            .into_iter()
            .map(|status| {
                let session_name = status
                    .summary
                    .clone()
                    .or_else(|| self.agent_title_suffix(status.pane_id))
                    .unwrap_or_else(|| status.source.clone());
                let name = match self.pane_location(status.pane_id) {
                    Some((tab, pane)) => format!("{}·{} {session_name}", tab + 1, pane + 1),
                    None => session_name,
                };
                let elapsed = elapsed_label(now.saturating_sub(status.since));
                AgentEntry {
                    pane_id: status.pane_id,
                    state: status.state,
                    name,
                    agent_name: status.source.clone(),
                    elapsed,
                }
            })
            .collect()
    }

    pub(crate) fn state_accent(&self, state: AgentState) -> Rgb {
        match state {
            AgentState::Blocked => self.agent_accent(true),
            AgentState::Thinking | AgentState::Working | AgentState::Compacting => {
                self.agent_accent(false)
            }
            AgentState::Done => self.colors.context.fg,
            AgentState::Idle => self.colors.cwd_normal.fg,
        }
    }

    pub(crate) fn agent_accent(&self, urgent: bool) -> Rgb {
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

    pub(crate) fn agent_title_suffix(&self, pane_id: u32) -> Option<String> {
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

    pub(crate) fn agent_rows_for_tab(
        &self,
        tab_position: usize,
    ) -> Vec<(usize, u32, &AgentStatus)> {
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

    pub(crate) fn apply_agent_event(&mut self, event: AgentEvent) -> bool {
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

    pub(crate) fn debug_snapshot(&self) -> String {
        let view = match self.view {
            View::Horizontal => "horizontal",
            View::Vertical => "vertical",
            View::Tabs => "tabs",
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
            "music": serde_json::json!({
                "right_panel": format!("{:?}", self.right_panel),
                "host_is_macos": self.host_is_macos,
                "refresh_pending": self.music_refresh_pending,
                "next_poll_at": self.music_next_poll_at,
                "now_playing": self.now_playing.as_ref().map(|track| format!("{track:?}")),
            }),
            "agent_statuses": statuses,
            "tabs": tabs,
        })
        .to_string()
    }

    pub(crate) fn sorted_agent_statuses(&self) -> Vec<&AgentStatus> {
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

    pub(crate) fn agent_entry_message(&self, status: &AgentStatus) -> String {
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

    pub(crate) fn agent_style(&self, urgent: bool) -> Style {
        if urgent {
            self.colors.agent_urgent
        } else {
            self.colors.agent
        }
    }

    pub(crate) fn agent_status_segments(&self, available: usize) -> Vec<(String, Style)> {
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

    pub(crate) fn pane_location(&self, pane_id: u32) -> Option<(usize, usize)> {
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

    pub(crate) fn track_focused_pane(&mut self) {
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

    pub(crate) fn detect_agents_from_manifest(&mut self) {
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

    pub(crate) fn update_detected_agent(
        &mut self,
        pane_id: u32,
        command: &[String],
        is_foreground: bool,
    ) {
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

    pub(crate) fn right_content(&self) -> (String, String) {
        let clock = if self.right_panel.shows_clock() {
            format!(
                "  {} ",
                current_time(self.timezone_offset_hours, &self.datetime_format)
            )
        } else {
            String::new()
        };
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

    /// The now-playing segment, already clipped to its configured width; empty
    /// when music is not shown or nothing is playing.
    pub(crate) fn music_content(&self) -> String {
        if !self.right_panel.shows_music() {
            return String::new();
        }
        let Some(track) = &self.now_playing else {
            return String::new();
        };
        if track.state != PlayerState::Playing {
            return String::new();
        }
        let text = format_now_playing(&self.music_format, track);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return String::new();
        }
        format!("  {} ", truncate_line(&text, self.music_max_width))
    }

    /// Polls Apple Music when due. The first call asks the host for its OS
    /// instead, reusing the pending flag so only one command is in flight.
    pub(crate) fn refresh_music(&mut self) {
        if self.view != View::Horizontal
            || !self.permissions_granted
            || !self.right_panel.shows_music()
            || self.music_refresh_pending
        {
            return;
        }
        let mut context = BTreeMap::new();
        match self.host_is_macos {
            None => {
                context.insert("kind".to_string(), "uname".to_string());
                self.music_refresh_pending = true;
                run_command(&["uname", "-s"], context);
            }
            Some(true) if unix_seconds() >= self.music_next_poll_at => {
                context.insert("kind".to_string(), "music".to_string());
                self.music_refresh_pending = true;
                run_command(&["osascript", "-e", NOW_PLAYING_SCRIPT], context);
            }
            _ => {}
        }
    }

    pub(crate) fn refresh_active_pane(&mut self) {
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
    pub(crate) fn note_permissions_granted(&mut self) {
        if self.permissions_granted {
            return;
        }
        self.permissions_granted = true;
        set_selectable(view_selectable(self.view));
    }

    /// The shared file is the handover point between sidebars: whoever sees an
    /// event writes it, and an instance that starts later reads it instead of
    /// waiting for every agent to speak again.
    pub(crate) fn sync_path(&self) -> Option<String> {
        agent_sync_path(&self.session_name)
    }

    pub(crate) fn persist_agent_statuses(&mut self) {
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

    pub(crate) fn hydrate_agent_statuses(&mut self) -> bool {
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

    pub(crate) fn merge_agent_statuses(&mut self, incoming: Vec<AgentStatus>) -> bool {
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

    pub(crate) fn refresh_cwds(&mut self) {
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
    pub(crate) fn refresh_repositories(&mut self) {
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

    pub(crate) fn selected_pane_for_tab(&self, tab: &TabInfo) -> Option<PaneId> {
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

    pub(crate) fn tab_for_selected_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.tabs.iter().find_map(|tab| {
            (self.selected_pane_for_tab(tab) == Some(pane_id)).then_some(tab.position)
        })
    }

    pub(crate) fn update_cwd(&mut self, cwd: PathBuf) {
        if self.active_cwd.as_ref() != Some(&cwd) {
            self.active_cwd = Some(cwd);
            self.git_context = None;
            self.refresh_git();
        }
    }

    pub(crate) fn refresh_git(&mut self) {
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

#[cfg(test)]
mod tests;
