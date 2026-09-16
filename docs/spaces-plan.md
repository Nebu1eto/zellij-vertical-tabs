# Spaces: feature description and implementation plan

Status: implemented and verified in a dedicated session. Target: zellij 0.45.x,
zellij-tile 0.45.0. Sections 5 and 3 were rewritten after runtime testing refuted
the original design; section 12 records what was measured.

## 1. What a space is

A **space** is a named group of Zellij tabs. The hierarchy the user sees is:

```
space  (left sidebar, Cmd+1..9)
└── tab   (top bar, Cmd+Alt+Left/Right)
    └── pane  (Cmd+D / Cmd+Shift+D splits, unchanged)
```

Zellij's own model is `session > tab > pane`. A plugin cannot add a nesting level, so
the space level is synthesised by the plugin from tab names, and the pane level is
Zellij's native one. "A pane that contains tabs" is not part of this design: a plugin can
only draw inside its own pane, so it cannot paint a tab strip onto panes it does not own.
Zellij's native approximation of that idea is a *stacked pane*
(`stack_panes()`, zellij-tile shim.rs:2549); it is out of scope here.

### User-visible behaviour

| Action | Key | Result |
| --- | --- | --- |
| New space | `Super n` | New tab named `<new-space>/1`, focused; sidebar gains a space entry |
| New tab in space | `Super t` | New tab named `<current-space>/<n>`, focused |
| Switch space | `Super 1`..`Super 9` | Focus that space's last-focused tab (or its first tab) |
| Next/previous tab in space | `Super Alt Right/Left` | Cycles within the active space only, wrapping |
| Close | `Super w` | Unchanged (`CloseFocus`); a space disappears when its last tab closes |
| Click a space | mouse | Sidebar space row switches space |
| Click a tab | mouse | Top-bar tab row switches tab (unchanged mechanism) |

The top bar shows only the active space's tabs, with the space prefix stripped and a
per-space index. The sidebar shows a **Spaces** section (all spaces, active one marked)
above the existing tab cards, which are filtered to the active space. The agents section
is unchanged and still lists agents from every space, since that is its purpose.

The feature is **off by default** (`spaces "true"` to enable). With it off, every rendering
path and every keybind falls back to today's behaviour.

## 2. Verified platform facts this plan depends on

Checked against a v0.45.0 checkout and the installed zellij-tile 0.45.0 crate.

1. **Keybind → plugin messaging.** `MessagePlugin` in KDL parses to `Action::KeybindPipe`
   (zellij-utils kdl/mod.rs:2244-2295) and is delivered to `pipe()` as `PipeSource::Keybind`
   (zellij-server route.rs:1702, plugins/mod.rs:983).
2. **Never name the plugin URL in the keybind.** With a URL, routing goes through
   `pipe_to_specific_plugins` → `get_or_load_plugins`, and instance matching requires
   *location and configuration* equality (plugin_map.rs:150-175). The layout loads this
   plugin with ~50 config keys, so a bare keybind matches nothing and **loads a new plugin
   pane** (tiled, because the KDL parser defaults `floating` to false, kdl/mod.rs:2262).
   Omitting the URL yields `plugin: None` (kdl/mod.rs:2246-2251) → `pipe_to_all_plugins`
   (plugins/mod.rs:1331), a broadcast that launches nothing.
3. **Broadcast fan-out is total.** `pipe_to_all_plugins` emits one message per
   `(plugin_id, client_id)` pair (plugins/mod.rs:1332-1348). Plugin instances are per client
   (plugin_loader.rs:313-320, plugin_map.rs:220). `PipeSource::Keybind` carries **no client id**
   (data.rs:3014-3018), so a plugin cannot tell which client pressed the key.
4. **No permission gate on pipe delivery** (pipes.rs:140). The plugin already requests
   `ChangeApplicationState`, so no new permission prompt appears for existing users.
5. **New tabs always append**: `let position = self.tabs.len()` (screen.rs:4720).
   There is no plugin API to create a tab at a position.
6. **Tab ids are `max + 1` and therefore reusable**: `get_new_tab_id` returns
   `self.tabs.keys().last() + 1` (screen.rs:1817-1823). Closing the highest-id tab frees that
   id for the next tab.
7. **`get_focused_pane_info()` is a blocking round-trip to the screen thread**, scoped to the
   calling instance's client, with a 100 ms timeout (zellij_exports.rs:1242-1288).
8. **Filesystem**: `/tmp` maps to `ZELLIJ_TMP_DIR` and `/host` to the plugin's cwd
   (zellij_exports.rs:142-152). The agent sidecar already uses `/tmp` (src/agent.rs:12).
9. **Tab names survive serialization**: the name is written as a KDL property
   (zellij-utils session_serialization.rs:96-99), so `/` round-trips through resurrection.
10. **`Super` reaches Zellij today** — the user's config already binds `Super n/t/1-9`.
    `KeybindPipe` is an ordinary action, so the modifier path is unchanged.

## 3. Data model

### Encoding

Tab name = `<space><SEP><tab>`, `SEP` configurable (`space_separator`, default `/`).
A tab name without `SEP` belongs to the implicit default space (`default_space_name`,
default `main`). Existing tabs are **never auto-renamed**; adoption is opt-in by renaming.

Renaming a tab is therefore the supported way to **move it between spaces**. There is no
auto-healing rename: re-applying a prefix from the plugin would fight the user's edit and
risks a rename/TabUpdate loop.

### Derived state (no persistence)

Everything is recomputed from `Event::TabUpdate`:

- **Space list and order**: group tabs by prefix, order by each group's *minimum* tab
  position. Since new tabs append, a space gaining a tab never changes its minimum, so the
  `Cmd+1..9` mapping is stable without a persisted order file.
- **Active space**: the space of the tab with `active == true`. Derived, never stored, so it
  cannot drift out of sync.
- **Last-focused tab per space**: a `HashMap<space, tab_id>` mirrored to
  `/tmp/zellij-vtabs-focus-<zellij_pid>`. The original plan kept this in memory only, on the
  assumption that every instance sees the same event stream. That assumption is false:
  Zellij delivers `TabUpdate` only to the plugins of the focused tab
  (screen.rs:6057 `targeted_plugin_ids`), so an instance in a background tab never observes
  a focus change elsewhere and each instance would remember a different tab. Runtime probe:
  leaving a space on its second tab and returning landed on the first tab until the file was
  added.

**Identity rule: address tabs by `TabInfo.tab_id`, never by position.** Positions shift when
any earlier tab closes, so a remembered position can silently target the wrong tab. Resolve
`tab_id → position` only at the moment of the call, and abort if the id is gone (this is also
the correct answer when a space-switch targets a tab that is closing).

## 4. Keybind protocol

All bindings are URL-less broadcasts with `vtabs:`-prefixed names so they cannot collide
with other plugins' pipe names:

```kdl
bind "Super n" { MessagePlugin { name "vtabs:space-new"; }; }
bind "Super t" { MessagePlugin { name "vtabs:tab-new"; }; }
bind "Super 1" { MessagePlugin { name "vtabs:space-switch"; payload "1"; }; }
// ... Super 2..9 likewise
bind "Super Alt Right" { MessagePlugin { name "vtabs:tab-next"; }; }
bind "Super Alt Left"  { MessagePlugin { name "vtabs:tab-prev"; }; }
```

The existing `Super 1-9 → GoToTab` bindings **must** be removed: they address global tab
indices and would jump across spaces. `Super Alt Left/Right` replace
`GoToPreviousTab`/`GoToNextTab`.

When `spaces` is disabled, the handlers fall back to `new_tab`, `go_to_tab`,
`go_to_next_tab`, `go_to_previous_tab`, so the same config works with the feature off.

## 5. Exactly-once execution

A single keypress is delivered to **every** plugin instance: both views, in every tab, for
every client. Without a guard, `Super n` creates one space per instance.

**Rejected: a fixed leader.** The first design elected the lowest sidebar plugin id. It is
deterministic, but because `TabUpdate` reaches only the focused tab's plugins
(screen.rs:6057), a leader sitting in a background tab decides from a tab list frozen at its
last focus. It is not a race that a retry fixes: the staleness is permanent until that tab is
focused again.

**Implemented: authority follows focus, with a claim window.**

1. An instance acts only if `get_focused_pane_info()` names a tab that its own cached list
   marks active (`spaces::instance_is_in_focused_tab`). Only the focused tab's instances
   satisfy this, and they are the only ones holding current state.
2. That tab holds one instance per view, so the sidebar acts and the bar acts only where
   there is no sidebar (`spaces::view_acts`).
3. Focus alone is still not enough: the first actor moves the focus, and an instance that
   reads the host *after* that sees the new focus matching its own frozen list and acts a
   second time. This was observed as two `leader=true` claims on one keypress. A claim file
   `/tmp/zellij-vtabs-claim-<zellij_pid>` records `(plugin_id, command, payload, millis)`;
   a different instance repeating the same command within 150 ms is refused
   (`spaces::claim_blocks`). A repeat from the *same* instance is never refused, so holding
   a key still cycles — verified with four rapid presses producing four switches.
4. On `Err` from `get_focused_pane_info()` the instance does nothing: a lost keypress is
   cheaper than several instances acting on stale lists.

**Known residual defect:** with two clients attached to one session, each client has its own
instance set and its own leader, so both leaders act and the action happens twice. The pipe
carries no client id, so this cannot be fixed inside the plugin. v1 documents the
single-client assumption. If it matters later, an `O_EXCL` token file under `/tmp`
(= `ZELLIJ_TMP_DIR`, shared by all clients of the session) makes it exactly-once; whether
`create_new` works over WASI must be probed first.

## 6. Code structure

New module `src/spaces.rs`, pure functions only:

```rust
pub struct SpaceRef { pub name: String, pub tabs: Vec<TabRef> } // TabRef { tab_id, position, label }
pub enum SpaceAction {           // what to do, decided without touching the host
    NewTab { name: String, cwd: Option<PathBuf> },
    FocusTab { tab_id: u64 },
    Fallback(FallbackAction),    // feature disabled
    None,
}

pub fn split_tab_name(name: &str, sep: &str, default_space: &str) -> (String, String);
pub fn group_spaces(tabs: &[TabInfo], sep: &str, default_space: &str) -> Vec<SpaceRef>;
pub fn active_space(spaces: &[SpaceRef], tabs: &[TabInfo]) -> Option<usize>;
pub fn next_space_name(spaces: &[SpaceRef]) -> String;
pub fn next_tab_name(space: &SpaceRef, sep: &str) -> String;
pub fn resolve_space_switch(spaces: &[SpaceRef], index: usize, last_focused: &HashMap<String, u64>) -> SpaceAction;
pub fn resolve_tab_cycle(spaces: &[SpaceRef], active: usize, active_tab_id: u64, forward: bool) -> SpaceAction;
pub fn is_leader(panes: &PaneManifest, plugin_id: u32) -> bool;
```

`app.rs` keeps a thin executor: decide with a pure function, then perform `new_tab`,
`switch_tab_to`, `focus_terminal_pane`. Every branch above is testable in `src/app/tests.rs`
in the existing style, with no Zellij runtime.

## 7. Rendering changes

### Horizontal bar

- Build a filtered `Vec<&TabInfo>` for the active space; pass the **active index within the
  filtered vector** to `horizontal_visible_indices`. The current
  `tabs.iter().position(|t| t.active).unwrap_or(0)` would otherwise highlight the wrong tab
  whenever the active tab is transiently outside the filter.
- Labels: strip the prefix and renumber per space. `tab_label` currently prints the global
  `tab.position + 1`, which would contradict `Super 1..9` now meaning spaces.
- `TabHitbox.position` keeps the **global** position so the existing click path still works.
- Do **not** strip the prefix while `self.mode == InputMode::RenameTab`: the user is editing
  the full name and must see what they are typing.

### Vertical sidebar

- New **Spaces** section at the top: one row per space, `▸` marker for the active one, name
  plus tab count; rows register hitboxes that switch space.
- The existing tab cards stay but are filtered to the active space, keeping per-tab cwd,
  repo/branch and the agent addressing that depends on them. Replacing them entirely with
  space cards would be a regression.
- `tab_totals_label` becomes space-scoped so the counts match the filtered list.
- Row budget: spaces section is capped (e.g. `min(spaces, rows/4)`) so tabs and agents keep
  their rows on short terminals.

## 8. Known hazards and how they are handled

| Hazard | Evidence | Handling |
| --- | --- | --- |
| Multi-client double execution | data.rs:3014-3018, plugins/mod.rs:1332 | Documented limit in v1; `/tmp` `O_EXCL` token later |
| Stale cached active tab on fast keys | TabUpdate is async to the pipe | Leader queries `get_focused_pane_info()` once |
| Position drift on close | positions are recomputed by the server | Address by `tab_id`, re-resolve at call time |
| Pane event precedes tab name | `PaneUpdate` and `TabUpdate` are separate events | Hold the previous active space for a frame; seed from the `tab_id` returned by `new_tab` |
| `close_empty_own_tab_if_needed` closing a new tab | `tabs_with_user_content` is never pruned, and tab ids are reused (screen.rs:1817) | Bounded today because each instance only evaluates its own tab and dies with it; still prune the set against `self.tabs` on `TabUpdate`, and exempt a tab this instance just created until it has been seen with user content |
| User renames a tab and loses the prefix | Rename mode edits the raw name | Treated as the documented way to move a tab between spaces |
| Tabs of one space interleaved in the real tab order | New tabs append (screen.rs:4720) | All space logic is filter-based, never range-based; no `MoveTab` re-grouping |
| Prefixed names leak into session-manager, `zellij action go-to-tab-name` | those surfaces read raw names | Accepted; the full name is the real name |
| Duplicate tab names across spaces | unverified whether zellij dedupes | Never use `go_to_tab_name`; always use ids |

## 9. Phased delivery

Each phase is shippable and independently testable.

- **Phase 0 — probes (no code shipped).** Settle the assumptions listed in §10 in a scratch
  session with a throwaway layout. Do not touch the user's live config.
- **Phase 1 — `src/spaces.rs` pure functions + unit tests.** No behaviour change.
- **Phase 2 — rendering only, behind `spaces "true"`.** Filtered bar, spaces section,
  labels. Validated by renaming tabs by hand; no pipes, no mutation.
- **Phase 3 — pipe plumbing with a no-op handler.** Handlers parse and elect a leader, then
  append a line to a debug file instead of acting. Proves exactly-once before anything mutates.
- **Phase 4 — switching.** `space-switch`, `tab-next`, `tab-prev`, sidebar clicks.
- **Phase 5 — creation.** `space-new`, `tab-new`, after the `tabs_with_user_content` prune
  and the creation exemption are in place.
- **Phase 6 (optional).** `/tmp` `O_EXCL` token for multi-client exactly-once; space rename;
  moving a tab between spaces from the sidebar.

Documentation (README.md and README.ko.md) lands with Phase 4, including the required
keybind block, since the feature is unusable without config changes.

## 10. Probes to run before relying on an assumption

| Assumption | Probe | Cost |
| --- | --- | --- |
| URL-less `MessagePlugin` broadcasts and launches no pane | temporary `Super p` → `vtabs:probe`; log `plugin_id`, view, tab; count lines, watch for a new pane | 1 keypress |
| Fan-out count equals instance count | same probe with 3 tabs open | 1 keypress |
| Cached active tab can lag | fast `Super 2` then `Super t`; compare cached active tab with `get_focused_pane_info()` | 2 keypresses |
| Plugin-created tabs get the `default_tab_template` | `new_tab(Some(name), Some(cwd))`; check the sidebar appears; if not, `new_tabs_with_layout_info` is required | 1 run |
| Tab id reuse hits the auto-close path | add `tab_id` to `debug_snapshot()`, `touch /host/.zellij-vtabs-debug`, create/close/create | 1 minute |
| `/` in tab names, duplicates | `zellij action rename-tab 'work/api'` twice, then `go-to-tab-name` | 1 minute |
| `O_EXCL` works over WASI | `OpenOptions::new().create_new(true)` twice from `Event::Timer` | Phase 6 only |

## 11. Explicitly out of scope

- Panes containing tabs (not representable; stacked panes are the native alternative).
- Persisted sidecar state for ordering or last-focused tab (derivable).
- Auto-healing renames via `rename_tab_with_id` (fights the user, risks loops).
- Tab reordering via `MoveTabByTabId` to make spaces contiguous (focus churn, no benefit
  once all logic is filter-based).
- Spaces as Zellij sessions (option B, rejected: every switch is a client detach/reattach).
- Per-space colours, icons, and drag-and-drop reordering in the sidebar.

## 12. Runtime verification

Run in an isolated session: separate `HOME`, `ZELLIJ_SOCKET_DIR`, and `TMPDIR`, with teardown
by exact session name. Isolating only `HOME` is not enough — the socket directory is derived
from `TMPDIR`, so `zellij ls` otherwise lists every session on the machine.

| Check | Result |
| --- | --- |
| URL-less `MessagePlugin` reaches running instances | Passed; every instance received it and no pane was opened |
| Exactly one instance acts | Passed after the claim window; one `leader=true` per keypress across 7 presses |
| Key repeat is not swallowed | Passed; 4 rapid presses produced 4 switches |
| `space-new`, `tab-new` | Passed; created `space-1/1`, then `space-1/2` in the same space |
| `space-switch` returns to the last tab | Passed after the shared focus file |
| `tab-next`/`tab-prev` cycle and wrap inside the space | Passed |
| Rename moves a tab between spaces | Passed; `space-1/2` renamed to `main/moved` regrouped live |
| Closing a space's last tab removes the space | Passed |
| Sidebar click switches space | Passed, driven by a synthetic SGR mouse click |
| `spaces` off keeps plain Zellij behaviour | Passed; new tab, tab by index, next tab |

Not verified: multi-client duplication, which remains a documented limitation, and
behaviour under session resurrection.
