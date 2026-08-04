> **Temporary file.** Committed at repo root only so it travels with this branch/PR. Delete before merging (normally lives at `.local/prd/`, which is gitignored).

# Floating pane stack preview

## Problem

Herdr renders only the topmost floating pane (`Tab::top_float()`). Every other
pane in `Tab.floats` is fully hidden with no visual trace, so there is no way
to tell how many floats are open or what they are without cycling through
them one at a time. Zellij's stacked panes solve the equivalent problem by
showing a one-line preview above the visible pane for each pane behind it in
the stack.

## Goal

Show a preview bar for each hidden float, stacked directly above the visible
floating popup, and let clicking a bar bring that float to the front and
focus it.

## Non-goals

- No changes to `Tab`, `AppState`, or any server/client protocol. `Tab.floats:
  Vec<PaneId>` (back-to-front, `floats.last()` = topmost/focused) already
  carries everything this feature needs; the stack preview is pure TUI
  presentation state.
- No change to keyboard cycling (`cycle_floats`) or the existing
  `NewFloat`/`ToggleFloat`/`ToggleFloats`/`CycleFloat` bindings.
- No change to the popup's own title bar, which currently falls back to
  `manual_label.unwrap_or("float")` rather than `border_label`. That's
  pre-existing behaviour, not addressed here — flagged so the difference in
  label richness between the popup title and the new stack bars (which do use
  `border_label`) isn't a surprise later.

## Design

### Geometry

`compute_pane_infos` (`src/ui/panes.rs`) already resolves a
`PopupResolvedGeometry` for `top_float()`. Add a strip of 1-row bars directly
above that popup's outer rect, same `x`/`width`, growing upward, one bar per
hidden float.

Number of bars shown:

```
shown = min(floats.len() - 1, space_above_popup, MAX_STACK_PREVIEW_ROWS)
```

- `space_above_popup` = `popup_outer.y - area.y` (rows free between the
  screen top and the popup's top edge).
- `MAX_STACK_PREVIEW_ROWS` is a fixed constant (8) so a screen full of floats
  can't fill the whole terminal with bars.

If `floats.len() - 1 > shown`, the topmost bar row is replaced with a
non-interactive `"+N more"` summary instead of a real pane's bar, where `N`
is the count of floats that don't get their own row.

### Ordering

Bars read top-to-bottom on screen as furthest-back → most-recently-behind,
with the focused popup directly below the bottommost bar. This mirrors
`floats`' back-to-front order and matches zellij's stack layout.

### Labels

Each bar's label is `terminal.border_label(app.show_agent_labels_on_pane_borders)`
(the same source tiled pane borders use, so it includes agent names),
falling back to `"float"`, truncated to fit with the existing `truncate_end`
helper.

### Hit-testing and click-to-switch

Each real (non-summary) bar gets a synthetic `PaneInfo` appended to
`pane_infos`:

```
PaneInfo {
    id: pane_id,
    rect: bar_rect,
    inner_rect: bar_rect,
    scrollbar_rect: None,
    borders: Borders::NONE,
    is_focused: false,
}
```

The existing click path — `pane_at()` → `focus_pane_before_mouse_press()` →
`focus_pane_internal_via_api()` → `focus_pane_in_workspace()` — already
raises a clicked float to the top of `floats` and focuses it. No new
input-handling code is needed; clicking a bar works through this path
unchanged.

The summary row (`"+N more"`) has no `PaneInfo` and is not clickable.

### Avoiding rendering artifacts

`float_rect()` (used to stop tiled-pane split borders drawing through the
float) currently returns the first float `PaneInfo` found — correct today
because there is only ever one. With multiple float `PaneInfo` entries (the
new bars), it must instead return the union bounding box of all `is_float`
entries, so tiled borders are masked out across the whole stack area (from
the topmost bar down to the bottom of the popup), not just around the popup.

### Rendering

In `render_panes`, after the existing top-float block, draw each bar row:

- Vertical `│` at the left and right edges, so the stack reads as one
  continuous frame down into the popup's own border (no visual gap between
  the bars and the popup below them).
- Label text on `panel_bg`.
- Muted `overlay0` styling, matching the existing unfocused-pane border
  color.

The summary row renders the same way with static `"+N more"` text.

## Testing

Follow the existing pattern in `src/ui/panes.rs` (e.g.
`tiled_split_borders_do_not_draw_over_a_float`). Add unit tests for:

- Bar count/capping arithmetic (exact fit, overflow into `"+N more"`, zero
  space above the popup).
- `float_rect()` union bounding box covering bars + popup.
- Click-to-focus on a bar raises the target float to the top of `floats` and
  focuses it (reuse of `focus_pane_in_workspace`, exercised through the bar's
  synthetic `PaneInfo`).

No characterization tests or roundtable needed — this is additive rendering
and hit-testing on top of existing, unchanged `Tab.floats` state; it doesn't
touch pane identity, persisted state, or protocol IDs.
