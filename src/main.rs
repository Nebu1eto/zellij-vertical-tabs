use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;
use zellij_tile::ui_components::{print_text_with_coordinates, Text};

#[derive(Default)]
struct State {
    tabs: Vec<TabInfo>,
    panes: PaneManifest,
    cwd_by_tab: HashMap<usize, PathBuf>,
    configured_home: Option<PathBuf>,
    visible_tab_positions: Vec<usize>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        set_selectable(false);
        self.configured_home = configuration
            .get("home")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from);

        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::CwdChanged,
            EventType::Mouse,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::TabUpdate(mut tabs) => {
                tabs.sort_by_key(|tab| tab.position);
                self.tabs = tabs;
                self.refresh_cwds();
                true
            }
            Event::PaneUpdate(panes) => {
                self.panes = panes;
                self.refresh_cwds();
                true
            }
            Event::CwdChanged(pane_id, cwd, _) => {
                if let Some(tab_position) = self.tab_for_selected_pane(pane_id) {
                    self.cwd_by_tab.insert(tab_position, cwd);
                    true
                } else {
                    false
                }
            }
            Event::Mouse(Mouse::LeftClick(line, _)) if line >= 0 => {
                let visible_index = line as usize / 2;
                if let Some(tab_position) = self.visible_tab_positions.get(visible_index) {
                    switch_tab_to((*tab_position + 1) as u32);
                }
                false
            }
            Event::PermissionRequestResult(_) => {
                self.refresh_cwds();
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.visible_tab_positions.clear();
        if rows < 2 || cols == 0 {
            return;
        }

        let capacity = rows / 2;
        let active_index = self.tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let range = visible_range(self.tabs.len(), active_index, capacity);

        for (visible_index, tab) in self.tabs[range].iter().enumerate() {
            self.visible_tab_positions.push(tab.position);

            let index = tab.position + 1;
            let name = if tab.name.is_empty() {
                format!("Tab {index}")
            } else {
                tab.name.clone()
            };
            let marker = if tab.active { "▸" } else { " " };
            let title = fit_line(&format!(" {marker} {index}  {name}"), cols);
            let cwd = self
                .cwd_by_tab
                .get(&tab.position)
                .map(|path| display_path(path, self.configured_home.as_deref()))
                .unwrap_or_else(|| "—".to_string());
            let cwd = fit_line(&format!("    {cwd}"), cols);

            let y = visible_index * 2;
            let mut title_text = Text::new(title).opaque();
            let mut cwd_text = Text::new(cwd).opaque().dim_all();
            if tab.active {
                title_text = title_text.selected();
                cwd_text = cwd_text.selected();
            }

            print_text_with_coordinates(title_text, 0, y, Some(cols), Some(1));
            print_text_with_coordinates(cwd_text, 0, y + 1, Some(cols), Some(1));
        }
    }
}

impl State {
    fn refresh_cwds(&mut self) {
        let pane_ids: Vec<(usize, PaneId)> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                self.selected_pane_for_tab(tab)
                    .map(|pane_id| (tab.position, pane_id))
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
}

fn visible_range(total: usize, active_index: usize, capacity: usize) -> std::ops::Range<usize> {
    if total <= capacity {
        return 0..total;
    }

    let half = capacity / 2;
    let start = active_index
        .saturating_sub(half)
        .min(total.saturating_sub(capacity));
    start..start + capacity
}

fn display_path(path: &Path, configured_home: Option<&Path>) -> String {
    let home = configured_home
        .filter(|home| path.starts_with(home))
        .map(Path::to_path_buf)
        .or_else(|| inferred_home(path));

    if let Some(home) = home {
        if let Ok(relative) = path.strip_prefix(&home) {
            if relative.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", relative.display());
        }
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

fn fit_line(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let value_width = value.chars().map(char_width).sum::<usize>();
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
        let character_width = char_width(character);
        if current_width + character_width > target {
            break;
        }
        result.push(character);
        current_width += character_width;
    }
    result.push('…');
    result
}

fn char_width(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0)
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
    fn respects_explicit_nonstandard_home_directory() {
        assert_eq!(
            display_path(
                Path::new("/srv/home/neb/Workspace/choco-pi"),
                Some(Path::new("/srv/home/neb"))
            ),
            "~/Workspace/choco-pi"
        );
    }

    #[test]
    fn truncates_to_the_available_cell_width() {
        assert_eq!(fit_line(" 1  choco-pi", 10), " 1  choco…");
        assert_eq!(fit_line("abc", 5), "abc  ");
    }

    #[test]
    fn keeps_active_tab_in_the_visible_window() {
        assert_eq!(visible_range(10, 0, 3), 0..3);
        assert_eq!(visible_range(10, 5, 3), 4..7);
        assert_eq!(visible_range(10, 9, 3), 7..10);
    }
}
