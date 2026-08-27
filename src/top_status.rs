use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{FixedOffset, Utc};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;
use zellij_tile::ui_components::{print_text_with_coordinates, Text};

const GIT_COMMAND: &str =
    "root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 1; printf '%s\\n' \"$root\"; git status --porcelain=v1 --branch";

#[derive(Default)]
struct State {
    mode: InputMode,
    session_name: String,
    tabs: Vec<TabInfo>,
    panes: PaneManifest,
    active_pane_id: Option<PaneId>,
    active_cwd: Option<PathBuf>,
    active_command: Option<String>,
    git_context: Option<GitContext>,
    git_refresh_pending: bool,
    permissions_granted: bool,
    alert: Option<Alert>,
    timer_ticks: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitContext {
    cwd: PathBuf,
    repository: String,
    branch: String,
    dirty: bool,
}

#[derive(Clone, Debug)]
struct Alert {
    message: String,
    expires_at: u64,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadCliPipes,
        ]);
        subscribe(&[
            EventType::ModeUpdate,
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::CwdChanged,
            EventType::CommandChanged,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
            EventType::Timer,
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
                self.refresh_active_pane();
            }
            Event::PaneUpdate(panes) => {
                self.panes = panes;
                self.refresh_active_pane();
            }
            Event::CwdChanged(pane_id, cwd, _) if Some(pane_id) == self.active_pane_id => {
                self.update_cwd(cwd);
            }
            Event::CommandChanged(pane_id, command, is_foreground, _)
                if Some(pane_id) == self.active_pane_id =>
            {
                self.active_command = is_foreground.then(|| command_label(&command));
            }
            Event::RunCommandResult(exit_code, stdout, _, context) => {
                self.git_refresh_pending = false;
                if exit_code == Some(0) {
                    if let Some(cwd) = context.get("cwd").map(PathBuf::from) {
                        if self.active_cwd.as_ref() == Some(&cwd) {
                            self.git_context = parse_git_context(&stdout, cwd);
                        }
                    }
                } else {
                    self.git_context = None;
                }
            }
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                set_selectable(false);
                self.permissions_granted = true;
                self.active_pane_id = None;
                self.refresh_active_pane();
                self.refresh_git();
            }
            Event::Timer(_) => {
                set_timeout(1.0);
                if self
                    .alert
                    .as_ref()
                    .is_some_and(|alert| alert.expires_at <= unix_seconds())
                {
                    self.alert = None;
                }
                self.timer_ticks = self.timer_ticks.wrapping_add(1);
                if self.timer_ticks >= 10 {
                    self.timer_ticks = 0;
                    self.refresh_git();
                }
            }
            _ => {}
        }
        true
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name != "zellaude" {
            return false;
        }
        let Some(payload) = pipe_message.payload else {
            return false;
        };
        let Ok(payload) = serde_json::from_str::<Value>(&payload) else {
            return false;
        };
        let event = payload
            .get("hook_event")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool = payload
            .get("tool_name")
            .and_then(Value::as_str)
            .filter(|tool| !tool.is_empty());

        match event {
            "PermissionRequest" => {
                let suffix = tool.map(|tool| format!(" · {tool}")).unwrap_or_default();
                self.set_alert(format!("⚠ Claude: permission required{suffix}"), 30);
            }
            "Stop" => self.set_alert("✓ Claude: response complete".to_string(), 8),
            "Notification" => self.set_alert("● Claude notification".to_string(), 12),
            "SessionEnd" => self.alert = None,
            _ => return false,
        }
        true
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if rows == 0 || cols == 0 {
            return;
        }

        let left = format!(
            " {}  {} ",
            mode_label(self.mode),
            if self.session_name.is_empty() {
                "Zellij"
            } else {
                &self.session_name
            }
        );
        let right = format!(" {} ", current_time());
        let (center, center_is_alert) = self.center_content();
        let left_width = cell_width(&left).min(cols);
        let right_width = cell_width(&right).min(cols.saturating_sub(left_width));

        print_text_with_coordinates(
            Text::new(fit_line(&left, left_width)).opaque().color_all(2),
            0,
            0,
            Some(left_width),
            Some(1),
        );

        if right_width > 0 {
            print_text_with_coordinates(
                Text::new(fit_line(&right, right_width)).opaque().selected(),
                cols - right_width,
                0,
                Some(right_width),
                Some(1),
            );
        }

        let center_start = left_width.saturating_add(2);
        let center_end = cols.saturating_sub(right_width.saturating_add(2));
        if center_end > center_start {
            let available = center_end - center_start;
            let rendered = fit_line(&center, available);
            let content_width = cell_width(rendered.trim_end());
            let x = ((cols.saturating_sub(content_width)) / 2)
                .max(center_start)
                .min(center_end.saturating_sub(content_width));
            let mut text = Text::new(rendered.trim_end().to_string()).opaque();
            text = if center_is_alert {
                text.error_color_all()
            } else {
                text.success_color_all()
            };
            print_text_with_coordinates(text, x, 0, Some(content_width), Some(1));
        }

        if rows > 1 {
            print_text_with_coordinates(
                Text::new("─".repeat(cols)).dim_all(),
                0,
                1,
                Some(cols),
                Some(1),
            );
        }
    }
}

impl State {
    fn refresh_active_pane(&mut self) {
        let Some(active_tab) = self.tabs.iter().find(|tab| tab.active) else {
            return;
        };
        let Some(panes) = self.panes.panes.get(&active_tab.position) else {
            return;
        };
        let pane = panes
            .iter()
            .filter(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
            .find(|pane| {
                pane.is_focused && pane.is_floating == active_tab.are_floating_panes_visible
            })
            .or_else(|| {
                panes
                    .iter()
                    .filter(|pane| !pane.is_plugin && !pane.is_suppressed && !pane.exited)
                    .find(|pane| pane.is_focused)
            });
        let Some(pane) = pane else {
            return;
        };

        let pane_id = PaneId::Terminal(pane.id);
        self.active_command = pane
            .terminal_command
            .as_deref()
            .map(|command| command_label(&[command.to_string()]));
        if self.active_pane_id != Some(pane_id) {
            self.active_pane_id = Some(pane_id);
            if self.permissions_granted {
                if let Ok(cwd) = get_pane_cwd(pane_id) {
                    self.update_cwd(cwd);
                }
            }
        }
    }

    fn update_cwd(&mut self, cwd: PathBuf) {
        if self.active_cwd.as_ref() != Some(&cwd) {
            self.active_cwd = Some(cwd);
            self.git_context = None;
            self.refresh_git();
        }
    }

    fn refresh_git(&mut self) {
        if !self.permissions_granted || self.git_refresh_pending {
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

    fn set_alert(&mut self, message: String, seconds: u64) {
        self.alert = Some(Alert {
            message,
            expires_at: unix_seconds().saturating_add(seconds),
        });
    }

    fn center_content(&self) -> (String, bool) {
        if let Some(alert) = &self.alert {
            return (alert.message.clone(), true);
        }

        let bells: Vec<&TabInfo> = self
            .tabs
            .iter()
            .filter(|tab| tab.has_bell_notification || tab.is_flashing_bell)
            .collect();
        if bells.len() == 1 {
            let tab = bells[0];
            return (format!("● Tab {}: {}", tab.position + 1, tab.name), true);
        }
        if bells.len() > 1 {
            return (format!("● {} tab notifications", bells.len()), true);
        }

        let command = self.active_command.as_deref().unwrap_or("shell");
        if let Some(git) = &self.git_context {
            let dirty = if git.dirty { "*" } else { "" };
            return (
                format!(
                    "{}   {}{}  ·  {}",
                    git.repository, git.branch, dirty, command
                ),
                false,
            );
        }

        let location = self
            .active_cwd
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .or_else(|| {
                self.tabs
                    .iter()
                    .find(|tab| tab.active)
                    .map(|tab| tab.name.as_str())
            })
            .unwrap_or("workspace");
        (format!("{location}  ·  {command}"), false)
    }
}

fn parse_git_context(stdout: &[u8], cwd: PathBuf) -> Option<GitContext> {
    let output = String::from_utf8_lossy(stdout);
    let mut lines = output.lines();
    let root = PathBuf::from(lines.next()?);
    let status = lines.next()?.strip_prefix("## ")?;
    let branch = status
        .split_once("...")
        .map(|(branch, _)| branch)
        .unwrap_or_else(|| status.split_whitespace().next().unwrap_or(status));
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

fn current_time() -> String {
    let timezone = FixedOffset::east_opt(9 * 60 * 60).expect("valid Seoul offset");
    Utc::now()
        .with_timezone(&timezone)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn fit_line(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let value_width = cell_width(value);
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

fn cell_width(value: &str) -> usize {
    value
        .chars()
        .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn detects_dirty_git_context() {
        let context = parse_git_context(
            b"/tmp/project\n## feature\n M src/main.rs\n",
            PathBuf::from("/tmp/project"),
        )
        .unwrap();
        assert_eq!(context.branch, "feature");
        assert!(context.dirty);
    }

    #[test]
    fn truncates_wide_center_content() {
        assert_eq!(fit_line("choco-pi · cargo", 10), "choco-pi …");
    }
}
