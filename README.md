# zellij-vertical-tabs

English | [한국어](README.ko.md)

A Zellij WebAssembly plugin with two views: a horizontal status bar and a vertical sidebar. It shows tabs, repository, branch, and linked-worktree state, a clock, and coding-agent status for choco-pi, Claude Code, Codex, and detected terminal agents.

![Horizontal status bar and vertical sidebar showing tabs and coding agents](assets/demo.png)

## Install

Download one release artifact under two names. Zellij identifies plugin instances by URL, so layouts that use both views must give each view a distinct URL or file path.

```sh
mkdir -p ~/.config/zellij/plugins
curl -fL https://github.com/Nebu1eto/zellij-vertical-tabs/releases/latest/download/zellij-vertical-tabs.wasm \
  -o ~/.config/zellij/plugins/vertical-tabs.wasm
cp ~/.config/zellij/plugins/vertical-tabs.wasm \
  ~/.config/zellij/plugins/vertical-sidebar.wasm
```

On first launch, focus each plugin pane and press `y` to grant the requested permissions.

## Configure

This layout loads the horizontal and vertical views from distinct paths. The percentage size sets the sidebar's initial width while keeping it resizable.

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
        pane size="30%" borderless=true {
            plugin location="file:~/.config/zellij/plugins/vertical-sidebar.wasm" {
                view "vertical"
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
| `vertical_separator_enabled` | Show the sidebar separator; defaults to `"true"` |
| `vertical_separator_char` | Sidebar separator; defaults to `"│"` |
| `border_enabled` | Enable the horizontal-view border independently of the sidebar separator |
| `border_char` | Horizontal-view border character |

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
color_border_fg  color_border_bg
color_agent_fg  color_agent_bg
color_agent_urgent_fg  color_agent_urgent_bg
```

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

Continuous integration checks formatting, Clippy, tests, and the release WASM build. To publish a release:

1. Set `version` in `Cargo.toml` to `MAJOR.MINOR.PATCH`.
2. Commit and push that change.
3. Create and push the matching tag:

   ```sh
   git tag vMAJOR.MINOR.PATCH
   git push origin vMAJOR.MINOR.PATCH
   ```

The release workflow accepts only a strict `vMAJOR.MINOR.PATCH` tag whose version matches `Cargo.toml`. It then creates a public GitHub Release and uploads the artifact as `zellij-vertical-tabs.wasm`.

## License

[MIT](LICENSE)
