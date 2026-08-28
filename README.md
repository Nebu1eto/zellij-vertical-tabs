# zellij-vertical-tabs

A single Zellij plugin for status information, horizontal tabs, two-line vertical tabs, and coding-agent activity.

The horizontal view provides the information used by zjstatus in this profile:

- mode and session
- clickable horizontal tabs and bell indicators
- repository, branch, dirty state, and active command
- configurable UTC offset and clock
- choco-pi, Claude Code, and Codex lifecycle status

The top bar keeps mode/session information on the left and the complete clock on the right. By default, tabs are centered and repository/command context shares the right side. Set `show_tabs "false"` to center repository, branch, dirty state, and active command instead; coding-agent status always takes center priority. The context falls back to the current directory and command outside a repository and is not duplicated on the right when tabs are hidden.

The vertical view renders each visible tab as two rows: tab index/name and the active pane's working directory. Both lines of the focused tab are bold. It keeps the active tab visible, supports click-to-switch behavior, and reserves its final column for a full-height separator by default.

## Build

```sh
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
```

The single artifact is:

```text
target/wasm32-wasip1/release/zellij-vertical-tabs.wasm
```

## Horizontal layout

```kdl
pane size=2 borderless=true {
    plugin location="file:~/.config/zellij/plugins/vertical-tabs.wasm" {
        view "horizontal"
        show_tabs "true"
        timezone_offset_hours "9"
        color_background "#2E3440"
        color_tab_active_fg "#2E3440"
        color_tab_active_bg "#88C0D0"
    }
}
```

## Vertical layout

```kdl
pane size="30%" borderless=true {
    plugin location="file:~/.config/zellij/plugins/vertical-sidebar.wasm" {
        view "vertical"
        home "/Users/example"
        vertical_separator_enabled "true"
        vertical_separator_char "│"
        color_tab_active_bg "#88C0D0"
        color_cwd_active_bg "#5E81AC"
    }
}
```

`show_tabs` defaults to `true`. `vertical_separator_enabled` defaults to `true`, and `vertical_separator_char` defaults to `│`. Disable the separator to make tab title and working-directory content use the full sidebar width. The horizontal `border_enabled` and `border_char` settings remain independent of the vertical separator.

Use a percentage such as `size="30%"` to set the initial sidebar width while keeping it resizable. A numeric value such as `size=30` creates a fixed pane that Zellij cannot resize. Put the plugin pane before or after the terminal pane to place it on the left or right. After granting permissions, the vertical plugin pane remains nonselectable so normal close actions cannot target it. Keep the adjacent terminal focused and use the existing `Alt/Option+Shift+Left/Right` pane-resize bindings to adjust the split. The active sidebar's observed column count becomes the absolute target for every visible plugin whose URL ends in `/vertical-sidebar.wasm`. On each pane-state update, mismatched sidebars grow or shrink toward that target until every reported width is exactly equal; this does not replay the initiating resize delta. Convergence is abandoned after 64 state updates if a pane disappears or cannot reach the target. Once a tab has contained a terminal or non-UI plugin, the vertical sidebar closes that tab when only the layout's top bar, sidebar, and suppressed `zellij:link` helper remain. This synchronization assumes a single attached client because `PaneManifest` exposes pane focus without identifying which client initiated the resize. Both views are nonselectable UI panes, while tab clicks continue to switch tabs.

When one layout loads horizontal and vertical views together, install the same build under two local paths: `vertical-tabs.wasm` for the horizontal bar and `vertical-sidebar.wasm` for the sidebar. Zellij keys running plugins by URL and can conflate two differently configured instances that use the same URL when a layout is added to an existing session. The dotfiles installer creates both files from the same build artifact.

## Colors

Colors use 24-bit ANSI and accept `#RRGGBB` (or `RRGGBB`) values. Invalid values fall back to the built-in Nord defaults. Every foreground/background pair can be configured independently:

```kdl
color_background "#2E3440"
color_session_fg "#D8DEE9"
color_session_bg "#3B4252"

color_mode_normal_fg "#2E3440"
color_mode_normal_bg "#A3BE8C"
color_mode_locked_fg "#ECEFF4"
color_mode_locked_bg "#BF616A"
color_mode_resize_fg "#2E3440"
color_mode_resize_bg "#EBCB8B"
color_mode_pane_fg "#2E3440"
color_mode_pane_bg "#88C0D0"
color_mode_tab_fg "#2E3440"
color_mode_tab_bg "#B48EAD"
color_mode_search_fg "#2E3440"
color_mode_search_bg "#EBCB8B"
color_mode_rename_tab_fg "#2E3440"
color_mode_rename_tab_bg "#D08770"
color_mode_rename_pane_fg "#2E3440"
color_mode_rename_pane_bg "#D08770"
color_mode_move_fg "#2E3440"
color_mode_move_bg "#B48EAD"
color_mode_default_fg "#D8DEE9"
color_mode_default_bg "#4C566A"

color_tab_normal_fg "#D8DEE9"
color_tab_normal_bg "#3B4252"
color_tab_active_fg "#2E3440"
color_tab_active_bg "#88C0D0"
color_cwd_normal_fg "#81A1C1"
color_cwd_normal_bg "#2E3440"
color_cwd_active_fg "#ECEFF4"
color_cwd_active_bg "#5E81AC"
color_context_fg "#D8DEE9"
color_context_bg "#3B4252"
color_clock_fg "#2E3440"
color_clock_bg "#88C0D0"
color_border_fg "#4C566A"
color_border_bg "#2E3440"
color_agent_fg "#2E3440"
color_agent_bg "#A3BE8C"
color_agent_urgent_fg "#ECEFF4"
color_agent_urgent_bg "#BF616A"
```

## Coding-agent events

Send Claude-compatible lifecycle JSON to the plugin through a small hook bridge:

```sh
zellij pipe --name coding-agent-status -- "$payload"
```

The payload fields used by the plugin are:

```json
{
  "pane_id": 7,
  "hook_event": "PreToolUse",
  "tool_name": "Bash",
  "summary": "fix coding agent integration",
  "source_agent": "codex",
  "ts_ms": 1787833000000
}
```

Supported sources are `choco-pi`, `claude-code`, and `codex`. Supported status events include session, prompt, tool, permission, notification, subagent, completion, failure, and session-end lifecycle events.

The plugin tracks every agent session independently per `pane_id`, so simultaneous agents in different tabs and panes each keep their own state. Both views list agents in tab-then-pane order. The horizontal center shows as many statuses as fit on one line, each prefixed with its `[tab·pane]` location, and appends `+N` when more agents are active than fit. In the vertical view, each tab block (index/name plus working directory) gains one row per agent running in that tab — `p1`, `p2`, … with the full status message such as `⚠ Claude Code permission required · Bash`. Agent rows render neutral-tinted: the text carries the normal or urgent color instead of a solid bar. Each tab title row carries a right-aligned rollup glyph — `⚠` when any agent is blocked, `…` while any works, `✓` when all are done — and agent rows append a dimmed task label from the pipe's `summary` field (kept across later events until a new one arrives) or fall back to the pane's terminal title when it looks like a task name. Clicking an agent row focuses that agent's pane; clicking the tab rows switches tabs.

Two states are sticky instead of expiring: a permission request stays until the agent moves on (its own next event replaces it), and a completed response (`✓`) stays until you focus that pane. Transient states still expire on their per-event lifetime, and a session-end event clears the pane's entry.

Agents that emit no hook events are still detected from their foreground process — `claude`, `codex`, `pi`/`choco-pi`, `opencode`, `gemini`, `cursor`, `aider`, `amp` appear as `◆ <agent> detected`. Hook events for the same pane take over with full lifecycle state, and the detected placeholder disappears when the process exits.

On first launch, grant the requested Zellij permissions. `ReadApplicationState` supplies tabs and working directories, `ChangeApplicationState` enables click-to-switch and notification focus, `RunCommands` supplies Git state, and `ReadCliPipes` receives coding-agent events.
