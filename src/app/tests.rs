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
fn agent_cards_carry_location_session_agent_and_state() {
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
        [
            "1·1 fix coding agent integration",
            "1·2 Claude Code",
            "2·1 Codex",
        ],
        "cards follow tab then pane order"
    );
    assert_eq!(entries[0].state, AgentState::Working);
    assert!(
        entries[0]
            .detail
            .as_deref()
            .unwrap()
            .starts_with("choco-pi · "),
        "the second row names the coding agent before elapsed time"
    );
    assert_eq!(entries[1].state, AgentState::Blocked);
    assert!(entries[1]
        .detail
        .as_deref()
        .unwrap()
        .starts_with("Claude Code · "));
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
        summary: Some("fix the sidebar".to_string()),
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
        output.contains("1·1 fix the sidebar"),
        "the first row shows tab·pane and session name"
    );
    assert!(output.contains("blocked"), "cards show the state text");
    assert!(
        output.contains("choco-pi"),
        "the second row names the coding agent"
    );
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
fn an_agent_card_keeps_its_session_name_while_tools_change() {
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
    assert_eq!(entry.name, "1·1 ship the sidebar");
    assert!(entry.detail.as_deref().unwrap().starts_with("choco-pi · "));
    assert!(!entry.detail.as_deref().unwrap().contains("reading code"));

    // Ending a tool or turn does not change the session label's position.
    assert!(state.apply_agent_event(AgentEvent {
        source: "choco-pi".to_string(),
        event: "Stop".to_string(),
        tool: None,
        summary: None,
        pane_id: 7,
        timestamp: Some(2),
    }));
    let entry = state.agent_entries().remove(0);
    assert_eq!(entry.name, "1·1 ship the sidebar");
    assert!(entry.detail.as_deref().unwrap().starts_with("choco-pi · "));
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
fn configured_initial_width_starts_a_sync_on_the_first_observation() {
    let mut state = State {
        view: View::Vertical,
        plugin_id: Some(11),
        initial_sidebar_width: Some(28),
        tabs: vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }],
        ..State::default()
    };
    state.panes.panes.insert(
        0,
        vec![PaneInfo {
            id: 11,
            is_plugin: true,
            plugin_url: Some("file:/plugins/vertical-sidebar.wasm".to_string()),
            pane_columns: 24,
            ..PaneInfo::default()
        }],
    );

    state.observe_sidebar_width_change();

    assert_eq!(
        state.pending_width_sync,
        Some(PendingWidthSync {
            target_width: 28,
            pane_ids: vec![11],
            last_requested_widths: HashMap::new(),
            attempts_remaining: WIDTH_SYNC_MAX_ATTEMPTS,
        })
    );
}

#[test]
fn pending_initial_width_sync_is_not_retargeted_to_an_intermediate_width() {
    let mut state = State {
        view: View::Vertical,
        plugin_id: Some(11),
        initial_sidebar_width: Some(28),
        tabs: vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }],
        ..State::default()
    };
    state.panes.panes.insert(
        0,
        vec![PaneInfo {
            id: 11,
            is_plugin: true,
            plugin_url: Some("file:/plugins/vertical-sidebar.wasm".to_string()),
            pane_columns: 24,
            ..PaneInfo::default()
        }],
    );
    state.observe_sidebar_width_change();
    state.panes.panes.get_mut(&0).unwrap()[0].pane_columns = 25;

    state.observe_sidebar_width_change();

    assert_eq!(state.pending_width_sync.unwrap().target_width, 28);
}

#[test]
fn a_terminal_resize_preserves_the_sidebar_column_width() {
    let mut state = State {
        view: View::Vertical,
        plugin_id: Some(11),
        tabs: vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }],
        ..State::default()
    };
    state.panes.panes.insert(
        0,
        vec![
            PaneInfo {
                id: 11,
                is_plugin: true,
                plugin_url: Some("file:/plugins/vertical-sidebar.wasm".to_string()),
                pane_columns: 28,
                ..PaneInfo::default()
            },
            PaneInfo {
                id: 7,
                pane_x: 28,
                pane_columns: 72,
                ..PaneInfo::default()
            },
        ],
    );
    state.observe_sidebar_width_change();
    state.panes.panes.get_mut(&0).unwrap()[0].pane_columns = 31;
    state.panes.panes.get_mut(&0).unwrap()[1].pane_x = 31;
    state.panes.panes.get_mut(&0).unwrap()[1].pane_columns = 89;

    state.observe_sidebar_width_change();

    assert_eq!(state.pending_width_sync.unwrap().target_width, 28);
}

#[test]
fn a_manual_resize_adopts_the_new_sidebar_column_width() {
    let mut state = State {
        view: View::Vertical,
        plugin_id: Some(11),
        tabs: vec![TabInfo {
            position: 0,
            active: true,
            ..TabInfo::default()
        }],
        ..State::default()
    };
    state.panes.panes.insert(
        0,
        vec![
            PaneInfo {
                id: 11,
                is_plugin: true,
                plugin_url: Some("file:/plugins/vertical-sidebar.wasm".to_string()),
                pane_columns: 28,
                ..PaneInfo::default()
            },
            PaneInfo {
                id: 7,
                pane_x: 28,
                pane_columns: 72,
                ..PaneInfo::default()
            },
        ],
    );
    state.observe_sidebar_width_change();
    state.panes.panes.get_mut(&0).unwrap()[0].pane_columns = 30;
    state.panes.panes.get_mut(&0).unwrap()[1].pane_x = 30;
    state.panes.panes.get_mut(&0).unwrap()[1].pane_columns = 70;

    state.observe_sidebar_width_change();

    assert_eq!(state.pending_width_sync.unwrap().target_width, 30);
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
