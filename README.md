# zellij-vertical-tabs

English | [한국어](README.ko.md)

A Zellij WebAssembly plugin with two views: a horizontal status bar and a vertical sidebar. It shows tabs, repository, branch, and linked-worktree state, a clock, and coding-agent status for choco-pi, Claude Code, Codex, and detected terminal agents.

![Horizontal status bar and vertical sidebar showing tabs and coding agents](assets/demo.png)

## Install

Download both release artifacts. They contain the same plugin binary under distinct names because
Zellij identifies plugin instances by URL.

```sh
mkdir -p ~/.config/zellij/plugins
curl -fL https://github.com/Nebu1eto/zellij-vertical-tabs/releases/latest/download/vertical-tabs.wasm \
  -o ~/.config/zellij/plugins/vertical-tabs.wasm
curl -fL https://github.com/Nebu1eto/zellij-vertical-tabs/releases/latest/download/vertical-sidebar.wasm \
  -o ~/.config/zellij/plugins/vertical-sidebar.wasm
```

On first launch, focus each plugin pane and press `y` to grant the requested permissions.

## Configure

This layout loads the horizontal and vertical views from distinct paths. Leave the sidebar pane size unset so the plugin can establish and preserve an exact, resizable column width.

```kdl
layout {
    pane size=2 borderless=true {
        plugin location="file:~/.config/zellij/plugins/vertical-tabs.wasm" {
            view "horizontal"
            show_tabs "true"
            timezone_offset_hours "9"
        }
    }
    pane split_direction="vertical" {
        pane
        pane borderless=true {
            plugin location="file:~/.config/zellij/plugins/vertical-sidebar.wasm" {
                view "vertical"
                initial_width "28"
                home "/Users/example"
                vertical_separator_enabled "true"
                vertical_separator_char "│"
            }
        }
    }
}
```

The main configuration keys are:

| Key | Values or purpose |
| --- | --- |
| `view` | `"horizontal"` or `"vertical"` |
| `show_tabs` | Show horizontal tabs; defaults to `"true"` |
| `timezone_offset_hours` | Clock offset from UTC, in hours |
| `home` | Home path used in displayed working directories |
| `initial_width` | Initial vertical-sidebar width in columns; leave the layout pane size unset so it remains resizable |
| `vertical_separator_enabled` | Show the sidebar separator; defaults to `"true"` |
| `vertical_separator_char` | Sidebar separator; defaults to `"│"` |
| `border_enabled` | Enable the horizontal-view border independently of the sidebar separator |
| `border_char` | Horizontal-view border character |
| `right_panel` | Right end of the horizontal bar: `"clock"` (default), `"music"`, or `"both"`; the music options need macOS and fall back to the clock elsewhere |
| `music_format` | Now-playing text; placeholders `{music}` (or `{title}`), `{artist}`, `{album}`, `{albumart}`; defaults to `"{artist} - {music}"` |
| `music_max_width` | Maximum now-playing width in columns before it is cut with `…`; defaults to `"40"` |
| `center_anchor` | Where the centered tabs, context, or agent status of the horizontal bar are balanced: `"bar"` (default) uses the middle of the bar; `"content"` uses the middle of the pane area beside a vertical sidebar in the active tab. Either way the group is pushed aside when it would overlap the left or right segments |

### Now playing

With `right_panel "music"` or `"both"`, the bar shows the track Apple Music is playing or has paused, including playback over AirPlay when the speaker is selected under **Computer** in the AirPlay picker of Music. A HomePod or Apple TV selected under **Home** plays Apple Music by itself while Music only acts as a remote; macOS then reports no player on the Mac and the segment stays hidden. The segment is also hidden while Music is stopped or closed, and the plugin never opens Music. Zellij runs `osascript` against Music on the host, so macOS shows an Automation permission prompt for the Zellij server once; a denied prompt leaves the segment empty. Polling adapts to the track: half the remaining time, bounded to 3–20 seconds, while playing and every 10 seconds otherwise.

`{albumart}` currently renders as `♪`. Zellij 0.45 discards the kitty graphics protocol from plugin panes and Ghostty has no sixel support, so there is no path to draw the artwork yet.

Colors accept `#RRGGBB` or `RRGGBB`. Invalid values use the built-in Nord defaults. Available keys are:

```text
color_background
color_session_fg  color_session_bg
color_mode_normal_fg  color_mode_normal_bg
color_mode_locked_fg  color_mode_locked_bg
color_mode_resize_fg  color_mode_resize_bg
color_mode_pane_fg  color_mode_pane_bg
color_mode_tab_fg  color_mode_tab_bg
color_mode_search_fg  color_mode_search_bg
color_mode_rename_tab_fg  color_mode_rename_tab_bg
color_mode_rename_pane_fg  color_mode_rename_pane_bg
color_mode_move_fg  color_mode_move_bg
color_mode_default_fg  color_mode_default_bg
color_tab_normal_fg  color_tab_normal_bg
color_tab_active_fg  color_tab_active_bg
color_cwd_normal_fg  color_cwd_normal_bg
color_cwd_active_fg  color_cwd_active_bg
color_context_fg  color_context_bg
color_clock_fg  color_clock_bg
color_music_fg  color_music_bg
color_border_fg  color_border_bg
color_agent_fg  color_agent_bg
color_agent_urgent_fg  color_agent_urgent_bg
```

## Spaces

A space is a group of tabs that share a name prefix, so `work/api` and `work/web` belong to the space `work`. The sidebar lists the spaces, the horizontal bar shows only the tabs of the active space, and panes stay ordinary Zellij splits. Renaming a tab moves it to another space.

The feature is off by default. Enable it on both views:

```kdl
plugin location="file:~/.config/zellij/plugins/vertical-sidebar.wasm" {
    view "vertical"
    spaces "true"
}
```

Spaces are driven by keybinds, because a plugin only receives keys while its own pane is focused. Add these to `config.kdl`, and remove any `GoToTab` bindings on the same keys, which address global tab indices and would jump across spaces:

```kdl
keybinds {
    normal {
        bind "Super n" { MessagePlugin { name "vtabs:space-new"; }; }
        bind "Super t" { MessagePlugin { name "vtabs:tab-new"; }; }
        bind "Super 1" { MessagePlugin { name "vtabs:space-switch"; payload "1"; }; }
        bind "Super 2" { MessagePlugin { name "vtabs:space-switch"; payload "2"; }; }
        bind "Super Alt Right" { MessagePlugin { name "vtabs:tab-next"; }; }
        bind "Alt 1" { MessagePlugin { name "vtabs:tab-switch"; payload "1"; }; }
        bind "Super Alt Left" { MessagePlugin { name "vtabs:tab-prev"; }; }
    }
}
```

Write `MessagePlugin` without a plugin URL. With a URL, Zellij matches the running instance by location *and* configuration, so a binding that does not repeat every key of the layout opens a second plugin pane instead of reaching the sidebar.

| Command | Effect |
| --- | --- |
| `vtabs:space-new` | New space with its first tab |
| `vtabs:tab-new` | New tab in the active space |
| `vtabs:space-switch` | Switch to the space in `payload`, on the tab it was last left on |
| `vtabs:tab-switch` | Switch to the tab numbered in `payload` inside the active space |
| `vtabs:tab-next`, `vtabs:tab-prev` | Cycle tabs inside the active space |

With `spaces` off, the same bindings keep their plain Zellij meaning: a new tab, a tab by index, and the next or previous tab.

| Key | Values or purpose |
| --- | --- |
| `spaces` | Group tabs into spaces; defaults to `"false"` |
| `space_separator` | Separator between space and tab name; defaults to `"/"` |
| `default_space_name` | Space for tabs without a separator; defaults to `"main"` |
| `auto_tab_names` | Name generated tabs after their repository or directory; defaults to `"true"` |
| `auto_space_names` | Show a generated space under its project name; defaults to `"true"` |

### Tab bar

With spaces on, put the tab strip in its own one-line pane above the space's content, so the status bar keeps its middle for agents and music:

```kdl
pane split_direction="horizontal" {
    pane size=1 borderless=true {
        plugin location="file:~/.config/zellij/plugins/vertical-tabs.wasm" {
            view "tabs"
            spaces "true"
        }
    }
    children
}
```

The strip lists the active space's tabs from the left edge and puts a `+` button directly after the last tab; clicking a tab switches to it and clicking `+` adds a tab to the space. Set `show_tabs "false"` on the horizontal status bar so the tabs are not drawn twice.

A tab you have not named shows the repository it sits in, or its directory, and follows the pane as you change directory. Naming a tab yourself always wins.

## Coding-agent status

The plugin detects supported terminal agent processes without hooks. Hooks add lifecycle and task details by sending JSON through the named pipe:

```sh
zellij pipe --name coding-agent-status -- "$payload"
```

The JSON may identify the pane, event, tool, task summary, source agent, and timestamp. Hook events from choco-pi, Claude Code, and Codex update the matching pane's status.

## Build

Install the WASI target and build the release artifact:

```sh
rustup target add wasm32-wasip1
cargo build --release
```

The build writes the artifact to:

```text
target/wasm32-wasip1/release/zellij-vertical-tabs.wasm
```

## Release

Continuous integration checks formatting, Clippy, tests, and the release WASM build. It uploads one
artifact containing `vertical-tabs.wasm` and `vertical-sidebar.wasm`. To publish a release:

1. Set `version` in `Cargo.toml` to `MAJOR.MINOR.PATCH`.
2. Commit and push that change.
3. Create and push the matching tag:

   ```sh
   git tag vMAJOR.MINOR.PATCH
   git push origin vMAJOR.MINOR.PATCH
   ```

The release workflow accepts only a strict `vMAJOR.MINOR.PATCH` tag whose version matches
`Cargo.toml`. It then creates a public GitHub Release and uploads the original build plus
`vertical-tabs.wasm` and `vertical-sidebar.wasm`.

## License

[MIT](LICENSE)
