//! Spaces: a grouping level above Zellij tabs, synthesised from tab names.
//!
//! Zellij's model is session > tab > pane and a plugin cannot add a nesting
//! level, so a space is a prefix shared by the names of several tabs. Every
//! function here is pure so the decisions can be tested without a Zellij host.

use crate::ui::is_vertical_sidebar_plugin;
use std::collections::HashMap;
use zellij_tile::prelude::*;

pub(crate) const DEFAULT_SEPARATOR: &str = "/";
pub(crate) const DEFAULT_SPACE_NAME: &str = "main";

/// One tab as seen from inside its space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceTab {
    /// Stable identifier; positions shift whenever an earlier tab closes.
    pub(crate) tab_id: usize,
    pub(crate) position: usize,
    /// Name with the space prefix removed.
    pub(crate) label: String,
    pub(crate) active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Space {
    pub(crate) name: String,
    pub(crate) tabs: Vec<SpaceTab>,
}

/// Splits a "space/tab" name into its two parts. A name without the separator,
/// or with an empty space part, belongs to the default space.
pub(crate) fn split_tab_name(raw: &str, separator: &str, default_space: &str) -> (String, String) {
    if separator.is_empty() {
        return (default_space.to_string(), raw.trim().to_string());
    }
    match raw.split_once(separator) {
        Some((space, rest)) if !space.trim().is_empty() => {
            (space.trim().to_string(), rest.trim().to_string())
        }
        _ => (default_space.to_string(), raw.trim().to_string()),
    }
}

/// Groups tabs into spaces, ordered by each space's first tab. Zellij appends
/// new tabs, so a space gaining a tab never changes that order: the space
/// shortcuts stay stable without persisting anything.
pub(crate) fn group_spaces(tabs: &[TabInfo], separator: &str, default_space: &str) -> Vec<Space> {
    let mut spaces: Vec<Space> = Vec::new();
    let mut ordered: Vec<&TabInfo> = tabs.iter().collect();
    ordered.sort_by_key(|tab| tab.position);
    for tab in ordered {
        let (space_name, label) = split_tab_name(&tab.name, separator, default_space);
        let label = if label.is_empty() {
            format!("Tab {}", tab.position + 1)
        } else {
            label
        };
        let space_tab = SpaceTab {
            tab_id: tab.tab_id,
            position: tab.position,
            label,
            active: tab.active,
        };
        match spaces.iter_mut().find(|space| space.name == space_name) {
            Some(space) => space.tabs.push(space_tab),
            None => spaces.push(Space {
                name: space_name,
                tabs: vec![space_tab],
            }),
        }
    }
    spaces
}

/// The space holding the active tab. Derived on every read so it cannot drift.
pub(crate) fn active_space_index(spaces: &[Space]) -> Option<usize> {
    spaces
        .iter()
        .position(|space| space.tabs.iter().any(|tab| tab.active))
}

/// First unused generated space name.
pub(crate) fn next_space_name(spaces: &[Space]) -> String {
    (1..)
        .map(|index| format!("space-{index}"))
        .find(|candidate| !spaces.iter().any(|space| &space.name == candidate))
        .unwrap_or_else(|| "space".to_string())
}

/// First unused numeric tab name inside a space.
pub(crate) fn next_tab_name(space: &Space, separator: &str) -> String {
    let suffix = (1..)
        .map(|index| index.to_string())
        .find(|candidate| !space.tabs.iter().any(|tab| &tab.label == candidate))
        .unwrap_or_else(|| "1".to_string());
    format!("{}{}{}", space.name, separator, suffix)
}

/// Tab to focus when switching to the index-th space (1-based), preferring the
/// tab that space was last left on. Returns a tab id, never a position: the
/// caller resolves it against the current tab list at the moment of the call.
pub(crate) fn resolve_space_switch(
    spaces: &[Space],
    index: usize,
    last_focused: &HashMap<String, usize>,
) -> Option<usize> {
    let space = spaces.get(index.checked_sub(1)?)?;
    let remembered = last_focused
        .get(&space.name)
        .and_then(|tab_id| space.tabs.iter().find(|tab| tab.tab_id == *tab_id));
    remembered
        .or_else(|| space.tabs.first())
        .map(|tab| tab.tab_id)
}

/// The index-th tab of a space (1-based), which is the number the strip shows.
pub(crate) fn resolve_tab_switch(space: &Space, index: usize) -> Option<usize> {
    space.tabs.get(index.checked_sub(1)?).map(|tab| tab.tab_id)
}

/// Neighbouring tab inside a space, wrapping at both ends.
pub(crate) fn resolve_tab_cycle(
    space: &Space,
    active_tab_id: Option<usize>,
    forward: bool,
) -> Option<usize> {
    if space.tabs.is_empty() {
        return None;
    }
    let current = active_tab_id
        .and_then(|tab_id| space.tabs.iter().position(|tab| tab.tab_id == tab_id))
        .or_else(|| space.tabs.iter().position(|tab| tab.active))
        .unwrap_or(0);
    let length = space.tabs.len();
    let next = if forward {
        (current + 1) % length
    } else {
        (current + length - 1) % length
    };
    Some(space.tabs[next].tab_id)
}

/// A keybind pipe reaches every plugin instance of every tab and every client,
/// so exactly one instance must act on it.
///
/// Zellij sends TabUpdate only to the plugins of the focused tab
/// (screen.rs: targeted_plugin_ids), so an instance in a background tab keeps
/// the tab list it had when its own tab lost focus. Acting on that stale list
/// creates tabs in the wrong space, which is why authority follows focus: the
/// instances of the focused tab are the only ones holding current state.
///
/// The focused tab holds one instance per view, so the sidebar acts when the
/// tab has one and the bar acts otherwise.
pub(crate) fn instance_is_in_focused_tab(tabs: &[TabInfo], focused_tab_id: usize) -> bool {
    tabs.iter()
        .any(|tab| tab.active && tab.tab_id == focused_tab_id)
}

pub(crate) fn view_acts(
    panes: &PaneManifest,
    focused_tab_position: usize,
    view_is_vertical: bool,
) -> bool {
    let has_sidebar = panes.panes.get(&focused_tab_position).is_some_and(|panes| {
        panes
            .iter()
            .any(|pane| !pane.is_suppressed && is_vertical_sidebar_plugin(pane))
    });
    if has_sidebar {
        view_is_vertical
    } else {
        !view_is_vertical
    }
}

/// Which tab each space was last left on, shared through a session file because
/// the instance that performs the next switch lives in a different tab and
/// never saw the previous one being focused.
pub(crate) fn format_focus_memory(memory: &HashMap<String, usize>) -> String {
    let mut entries: Vec<(&String, &usize)> = memory.iter().collect();
    entries.sort();
    entries
        .into_iter()
        .filter(|(space, _)| !space.contains('\t') && !space.contains('\n'))
        .map(|(space, tab_id)| format!("{space}\t{tab_id}\n"))
        .collect()
}

pub(crate) fn parse_focus_memory(raw: &str) -> HashMap<String, usize> {
    raw.lines()
        .filter_map(|line| {
            let (space, tab_id) = line.split_once('\t')?;
            let tab_id = tab_id.trim().parse().ok()?;
            (!space.is_empty()).then(|| (space.to_string(), tab_id))
        })
        .collect()
}

/// Record of the instance that last acted on a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Claim {
    pub(crate) plugin_id: u32,
    pub(crate) command: String,
    pub(crate) payload: String,
    pub(crate) millis: u64,
}

pub(crate) fn format_claim(plugin_id: u32, command: &str, payload: &str, millis: u64) -> String {
    format!("{plugin_id}\t{command}\t{payload}\t{millis}\n")
}

pub(crate) fn parse_claim(raw: &str) -> Option<Claim> {
    let line = raw.lines().next()?;
    let mut fields = line.splitn(4, '\t');
    Some(Claim {
        plugin_id: fields.next()?.parse().ok()?,
        command: fields.next()?.to_string(),
        payload: fields.next()?.to_string(),
        millis: fields.next()?.trim().parse().ok()?,
    })
}

/// Focus alone cannot settle who acts: the first instance to act moves the
/// focus, and an instance that reads the host afterwards sees the new focus
/// matching its own frozen tab list and acts a second time. A short claim window
/// closes that gap. A repeat from the same instance is never blocked, so holding
/// a key still cycles.
pub(crate) fn claim_blocks(
    existing: Option<&Claim>,
    plugin_id: u32,
    command: &str,
    payload: &str,
    now_millis: u64,
    window_millis: u64,
) -> bool {
    let Some(claim) = existing else {
        return false;
    };
    claim.plugin_id != plugin_id
        && claim.command == command
        && claim.payload == payload
        && now_millis.saturating_sub(claim.millis) < window_millis
}

#[cfg(test)]
mod tests;
