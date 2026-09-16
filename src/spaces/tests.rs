use super::*;

fn tab(position: usize, tab_id: usize, name: &str, active: bool) -> TabInfo {
    TabInfo {
        position,
        tab_id,
        name: name.to_string(),
        active,
        ..TabInfo::default()
    }
}

fn sidebar_pane(id: u32) -> PaneInfo {
    PaneInfo {
        id,
        is_plugin: true,
        plugin_url: Some("file:/plugins/vertical-sidebar.wasm".to_string()),
        ..PaneInfo::default()
    }
}

#[test]
fn a_name_without_the_separator_belongs_to_the_default_space() {
    assert_eq!(
        split_tab_name("notes", "/", "main"),
        ("main".to_string(), "notes".to_string())
    );
    assert_eq!(
        split_tab_name("/orphan", "/", "main"),
        ("main".to_string(), "/orphan".to_string())
    );
}

#[test]
fn only_the_first_separator_splits_the_name() {
    assert_eq!(
        split_tab_name("work/api/v2", "/", "main"),
        ("work".to_string(), "api/v2".to_string())
    );
}

#[test]
fn spaces_keep_the_order_of_their_first_tab() {
    // Zellij appends new tabs, so a space gaining a tab must not reorder the
    // list under the space shortcuts.
    let tabs = vec![
        tab(0, 0, "work/api", false),
        tab(1, 1, "docs/readme", true),
        tab(2, 2, "work/web", false),
    ];
    let spaces = group_spaces(&tabs, "/", "main");
    assert_eq!(
        spaces
            .iter()
            .map(|space| space.name.as_str())
            .collect::<Vec<_>>(),
        vec!["work", "docs"]
    );
    assert_eq!(spaces[0].tabs.len(), 2);
    assert_eq!(active_space_index(&spaces), Some(1));
}

#[test]
fn an_unnamed_tab_keeps_a_readable_label() {
    let spaces = group_spaces(&[tab(4, 9, "", false)], "/", "main");
    assert_eq!(spaces[0].tabs[0].label, "Tab 5");
}

#[test]
fn generated_names_skip_the_ones_already_in_use() {
    let tabs = vec![tab(0, 0, "space-1/1", false), tab(1, 1, "space-3/1", true)];
    let spaces = group_spaces(&tabs, "/", "main");
    assert_eq!(next_space_name(&spaces), "space-2");
    assert_eq!(next_tab_name(&spaces[0], "/"), "space-1/2");
}

#[test]
fn switching_prefers_the_tab_the_space_was_left_on() {
    let tabs = vec![
        tab(0, 10, "work/api", true),
        tab(1, 11, "docs/a", false),
        tab(2, 12, "docs/b", false),
    ];
    let spaces = group_spaces(&tabs, "/", "main");
    let mut last_focused = HashMap::new();
    last_focused.insert("docs".to_string(), 12);
    assert_eq!(resolve_space_switch(&spaces, 2, &last_focused), Some(12));
    // A remembered tab that no longer exists falls back to the first tab.
    last_focused.insert("docs".to_string(), 99);
    assert_eq!(resolve_space_switch(&spaces, 2, &last_focused), Some(11));
    assert_eq!(resolve_space_switch(&spaces, 3, &last_focused), None);
    assert_eq!(resolve_space_switch(&spaces, 0, &last_focused), None);
}

#[test]
fn cycling_stays_inside_the_space_and_wraps() {
    let tabs = vec![
        tab(0, 10, "work/api", false),
        tab(1, 11, "docs/a", true),
        tab(2, 12, "work/web", false),
    ];
    let spaces = group_spaces(&tabs, "/", "main");
    let work = &spaces[0];
    assert_eq!(resolve_tab_cycle(work, Some(10), true), Some(12));
    assert_eq!(resolve_tab_cycle(work, Some(12), true), Some(10));
    assert_eq!(resolve_tab_cycle(work, Some(10), false), Some(12));
    // An id from another space falls back to that space's own active tab.
    assert_eq!(resolve_tab_cycle(work, Some(11), true), Some(12));
}

#[test]
fn only_an_instance_of_the_focused_tab_has_authority() {
    // A background instance keeps the list it had when its own tab was focused,
    // which still marks its own tab active.
    let stale = vec![tab(0, 10, "work/api", true), tab(1, 11, "docs/a", false)];
    let fresh = vec![tab(0, 10, "work/api", false), tab(1, 11, "docs/a", true)];
    assert!(!instance_is_in_focused_tab(&stale, 11));
    assert!(instance_is_in_focused_tab(&fresh, 11));
    assert!(!instance_is_in_focused_tab(&fresh, 99));
}

#[test]
fn the_sidebar_acts_when_the_focused_tab_has_one() {
    let mut panes = PaneManifest::default();
    panes.panes.insert(1, vec![sidebar_pane(3)]);
    assert!(view_acts(&panes, 1, true));
    assert!(!view_acts(&panes, 1, false));
}

#[test]
fn the_bar_acts_when_the_focused_tab_has_no_sidebar() {
    let mut panes = PaneManifest::default();
    panes.panes.insert(
        1,
        vec![PaneInfo {
            id: 5,
            is_plugin: true,
            plugin_url: Some("file:/plugins/vertical-tabs.wasm".to_string()),
            ..PaneInfo::default()
        }],
    );
    assert!(view_acts(&panes, 1, false));
    assert!(!view_acts(&panes, 1, true));
}

#[test]
fn focus_memory_survives_a_round_trip() {
    let mut memory = HashMap::new();
    memory.insert("work".to_string(), 12);
    memory.insert("docs".to_string(), 3);
    let encoded = format_focus_memory(&memory);
    assert_eq!(parse_focus_memory(&encoded), memory);
    // Damaged lines are dropped instead of poisoning the map.
    let mut damaged = encoded.clone();
    damaged.push_str("broken line\n\tno-space\t7\n");
    assert_eq!(parse_focus_memory(&damaged), memory);
}

#[test]
fn a_claim_blocks_only_another_instance_inside_the_window() {
    let claim = parse_claim(&format_claim(4, "tab-new", "", 1_000)).expect("claim parses");
    assert_eq!(claim.plugin_id, 4);
    // Another instance replaying the same broadcast is blocked.
    assert!(claim_blocks(Some(&claim), 2, "tab-new", "", 1_050, 150));
    // The same instance may repeat, so holding a key keeps cycling.
    assert!(!claim_blocks(Some(&claim), 4, "tab-new", "", 1_050, 150));
    // A later press is a new action.
    assert!(!claim_blocks(Some(&claim), 2, "tab-new", "", 1_200, 150));
    // A different command is unrelated.
    assert!(!claim_blocks(Some(&claim), 2, "tab-prev", "", 1_050, 150));
    assert!(!claim_blocks(None, 2, "tab-new", "", 1_050, 150));
}

#[test]
fn a_damaged_claim_file_does_not_block_anything() {
    assert!(parse_claim("garbage").is_none());
    assert!(parse_claim("").is_none());
}
