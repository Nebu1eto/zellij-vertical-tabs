use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::format::{strftime::StrftimeItems, Item};
use chrono::{FixedOffset, Utc};
use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;

use crate::app::{GitContext, PendingWidthSync, RepoInfo, View};

pub(crate) const DEFAULT_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M";
const VERTICAL_SIDEBAR_URL_SUFFIX: &str = "/vertical-sidebar.wasm";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rgb(pub(crate) u8, pub(crate) u8, pub(crate) u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) fg: Rgb,
    pub(crate) bg: Rgb,
    pub(crate) bold: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Colors {
    pub(crate) background: Rgb,
    pub(crate) session: Style,
    pub(crate) mode_normal: Style,
    pub(crate) mode_locked: Style,
    pub(crate) mode_resize: Style,
    pub(crate) mode_pane: Style,
    pub(crate) mode_tab: Style,
    pub(crate) mode_search: Style,
    pub(crate) mode_rename_tab: Style,
    pub(crate) mode_rename_pane: Style,
    pub(crate) mode_move: Style,
    pub(crate) mode_default: Style,
    pub(crate) tab_normal: Style,
    pub(crate) tab_active: Style,
    pub(crate) cwd_normal: Style,
    pub(crate) cwd_active: Style,
    pub(crate) context: Style,
    pub(crate) clock: Style,
    pub(crate) border: Style,
    pub(crate) agent: Style,
    pub(crate) agent_urgent: Style,
}

impl Default for Colors {
    fn default() -> Self {
        Self::from_config(&BTreeMap::new())
    }
}

impl Colors {
    pub(crate) fn from_config(configuration: &BTreeMap<String, String>) -> Self {
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

    pub(crate) fn mode(&self, mode: InputMode) -> Style {
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

pub(crate) struct AnsiFrame {
    pub(crate) output: String,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
}

impl AnsiFrame {
    pub(crate) fn new(rows: usize, cols: usize, colors: &Colors) -> Self {
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

    pub(crate) fn put(&mut self, x: usize, y: usize, style: Style, value: &str) {
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

    pub(crate) fn finish(mut self) -> String {
        self.output.push_str("\x1b[0m");
        self.output
    }
}

pub(crate) fn horizontal_visible_indices(
    tabs: &[TabInfo],
    active: usize,
    width: usize,
) -> Vec<usize> {
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

pub(crate) fn tab_label(tab: &TabInfo) -> String {
    let bell = if tab.has_bell_notification || tab.is_flashing_bell {
        " ●"
    } else {
        ""
    };
    format!(" {} {}{} ", tab.position + 1, tab_name(tab), bell)
}

pub(crate) fn tab_name(tab: &TabInfo) -> String {
    if tab.name.is_empty() {
        format!("Tab {}", tab.position + 1)
    } else {
        tab.name.clone()
    }
}

pub(crate) fn display_path(path: &Path, configured_home: Option<&Path>) -> String {
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

pub(crate) fn inferred_home(path: &Path) -> Option<PathBuf> {
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

pub(crate) fn parse_git_context(stdout: &[u8], cwd: PathBuf) -> Option<GitContext> {
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

pub(crate) fn command_label(command: &[String]) -> String {
    command
        .first()
        .and_then(|command| Path::new(command).file_name())
        .and_then(|command| command.to_str())
        .unwrap_or("shell")
        .to_string()
}

pub(crate) fn mode_label(mode: InputMode) -> String {
    format!("{mode:?}").to_uppercase()
}

pub(crate) fn subscribe_to_events() {
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

pub(crate) fn permissions_for_view(view: View) -> &'static [PermissionType] {
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

pub(crate) fn view_selectable(_view: View) -> bool {
    false
}

pub(crate) fn width_sync_strategy(
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

pub(crate) fn plan_width_sync_attempt(
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

pub(crate) fn visible_vertical_sidebar_ids(panes: &PaneManifest) -> Vec<u32> {
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

pub(crate) fn sidebar_geometry(panes: &PaneManifest, plugin_id: u32) -> Option<(usize, Direction)> {
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

pub(crate) fn active_sidebar_state(
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

pub(crate) fn active_tab_width(tabs: &[TabInfo], panes: &PaneManifest) -> Option<usize> {
    let active_position = tabs.iter().find(|tab| tab.active)?.position;
    panes
        .panes
        .get(&active_position)?
        .iter()
        .filter(|pane| !pane.is_suppressed)
        .map(|pane| pane.pane_x.saturating_add(pane.pane_columns))
        .max()
}

pub(crate) fn own_tab_content_state(
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

pub(crate) fn is_layout_ui_pane(pane: &PaneInfo) -> bool {
    if !pane.is_plugin {
        return false;
    }
    pane.plugin_url.as_deref().is_some_and(|url| {
        url.ends_with(VERTICAL_SIDEBAR_URL_SUFFIX)
            || url.ends_with("/vertical-tabs.wasm")
            || (pane.is_suppressed && url == "zellij:link")
    })
}

pub(crate) fn is_vertical_sidebar_plugin(pane: &PaneInfo) -> bool {
    pane.is_plugin
        && pane
            .plugin_url
            .as_deref()
            .is_some_and(|url| url.ends_with(VERTICAL_SIDEBAR_URL_SUFFIX))
}

pub(crate) fn parse_hex_color(value: &str) -> Option<Rgb> {
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

pub(crate) fn configured_color(
    configuration: &BTreeMap<String, String>,
    key: &str,
    fallback: Rgb,
) -> Rgb {
    configuration
        .get(key)
        .and_then(|value| parse_hex_color(value))
        .unwrap_or(fallback)
}

pub(crate) fn ansi_style(style: Style) -> String {
    let bold = if style.bold { "1;" } else { "22;" };
    format!(
        "\x1b[{bold}38;2;{};{};{};48;2;{};{};{}m",
        style.fg.0, style.fg.1, style.fg.2, style.bg.0, style.bg.1, style.bg.2
    )
}

pub(crate) fn horizontal_group_start(
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
pub(crate) const REPO_COMMAND: &str =
    "git rev-parse --show-toplevel --abbrev-ref HEAD --git-dir --git-common-dir";

/// Parse `git rev-parse` output: toplevel, branch, git dir, common dir.
pub(crate) fn parse_repo_info(stdout: &[u8]) -> Option<RepoInfo> {
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

pub(crate) fn directory_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

/// Repository name owning a git directory such as `/repo/.git` or `/repo/.bare`.
pub(crate) fn project_name_for_git_dir(git_dir: &Path) -> Option<String> {
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
pub(crate) fn tab_display_name(tab: &TabInfo, repo: Option<&RepoInfo>) -> String {
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
pub(crate) fn tab_detail_line(repo: Option<&RepoInfo>, cwd: Option<&str>) -> String {
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
pub(crate) fn vertical_tab_window(
    total: usize,
    active_index: usize,
    capacity: usize,
) -> Vec<usize> {
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
pub(crate) fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        _ => format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// Spelling the units out is worth the room when there is room; a narrow
/// sidebar falls back to the bare counts rather than truncating a word.
pub(crate) fn tab_totals_label(tabs: usize, panes: usize, room: usize) -> String {
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
pub(crate) fn agent_totals_label(sessions: usize, room: usize) -> String {
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
pub(crate) fn relative_luminance(color: Rgb) -> f64 {
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

pub(crate) fn contrast_ratio(one: Rgb, other: Rgb) -> f64 {
    let (a, b) = (relative_luminance(one), relative_luminance(other));
    let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// A state accent is chosen against the panel background, so on a highlighted
/// card it can land too close to that highlight to read - an idle agent is the
/// usual victim, since its accent is the same muted tone. Fall back to the
/// card's own foreground whenever the accent stops being legible.
pub(crate) fn readable_on(accent: Rgb, background: Rgb, fallback: Rgb) -> Rgb {
    const LEGIBLE_CONTRAST: f64 = 2.5;
    if contrast_ratio(accent, background) >= LEGIBLE_CONTRAST {
        accent
    } else {
        fallback
    }
}

pub(crate) fn vertical_styles(colors: &Colors, active: bool) -> (Style, Style) {
    if active {
        (colors.tab_active, colors.cwd_active)
    } else {
        (colors.tab_normal, colors.cwd_normal)
    }
}

pub(crate) fn validated_datetime_format(configured: Option<&str>) -> String {
    let format = configured.unwrap_or(DEFAULT_DATETIME_FORMAT);
    if StrftimeItems::new(format).any(|item| matches!(item, Item::Error)) {
        DEFAULT_DATETIME_FORMAT.to_string()
    } else {
        format.to_string()
    }
}

pub(crate) fn current_time(offset_hours: i32, format: &str) -> String {
    let timezone = FixedOffset::east_opt(offset_hours * 60 * 60).expect("validated offset");
    Utc::now()
        .with_timezone(&timezone)
        .format(format)
        .to_string()
}

pub(crate) fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn printable_character(character: char) -> char {
    if character.is_control() {
        '\u{fffd}'
    } else {
        character
    }
}

pub(crate) fn sanitize_text(value: &str) -> String {
    value.chars().map(printable_character).collect()
}

pub(crate) fn sanitize_and_clip(value: &str, width: usize) -> String {
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

pub(crate) fn repeat_pattern_to_width(pattern: &str, width: usize) -> String {
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

pub(crate) fn fit_line(value: &str, width: usize) -> String {
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

pub(crate) fn truncate_line(value: &str, width: usize) -> String {
    if cell_width(value) <= width {
        sanitize_text(value)
    } else {
        fit_line(value, width)
    }
}

pub(crate) fn fit_right_parts(context: &str, clock: &str, width: usize) -> (String, String) {
    let clock_width = cell_width(clock);
    if width <= clock_width {
        return (String::new(), fit_line(clock, width));
    }
    (fit_line(context, width - clock_width), sanitize_text(clock))
}

pub(crate) fn horizontal_content_split(
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

pub(crate) fn vertical_separator_content(
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

pub(crate) fn cell_width(value: &str) -> usize {
    value
        .chars()
        .map(printable_character)
        .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
        .sum()
}
