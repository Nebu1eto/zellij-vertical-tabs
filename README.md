# zellij-vertical-tabs

A compact Zellij sidebar that renders every visible tab as two rows:

1. tab index and name
2. the active pane's current working directory

The active tab remains visible when the sidebar is too short to display every tab. Clicking either row switches to that tab.

## Build

```sh
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
```

The plugin artifact is written to:

```text
target/wasm32-wasip1/release/zellij-vertical-tabs.wasm
```

## Layout

Use the plugin as a fixed-width pane beside the tab's normal content:

```kdl
pane split_direction="vertical" {
    pane size=30 borderless=true {
        plugin location="file:~/.config/zellij/plugins/vertical-tabs.wasm"
    }
    pane
}
```

Change `size=30` to customize the width. The plugin pane appears on the left because it comes before the terminal pane; move it after the terminal pane to place the sidebar on the right.

On first launch, Zellij asks for `ReadApplicationState` and `ChangeApplicationState` permissions. The first reads tabs and pane directories; the second makes mouse clicks switch tabs.
