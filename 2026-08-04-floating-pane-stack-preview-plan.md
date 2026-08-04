# Floating Pane Stack Preview Implementation Plan

> **Temporary file.** Committed at repo root only so it travels with this branch/PR. Delete before merging (normally lives at `.local/prd/`, which is gitignored).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a one-line preview bar for every floating pane hidden behind the visible popup, stacked above it, and let clicking a bar bring that float to the front.

**Architecture:** A pure geometry function (`stack_bar_rects`) turns the hidden-float list into a list of `StackBar`s (rect + label source), capped to the space available and a fixed row limit. That list is computed alongside the existing pane-hit-test pass, threaded through the existing `TabSurfaceLayout`/`TabSurfaceView`/`ViewState` pipeline unchanged in kind, and consumed by two places: the existing mouse-hit-test path (via synthetic `PaneInfo` entries, no new input code) and a new render pass that draws each bar.

**Tech Stack:** Rust, ratatui. No new dependencies.

## Global Constraints

- No `unwrap()` in production code (`src/`, outside `#[cfg(test)]`).
- `#[allow(...)]` only with a comment explaining why.
- Follow "Render is pure": geometry/data computation stays in `compute_*` functions that take `&AppState`; `render_*` functions only draw and take `&AppState`.
- No new dependencies.
- Match this repo's existing patterns exactly: `pane_border_title` for label truncation, `Workspace`'s `Deref<Target = Tab>` for `floats`/`top_float()`/`is_float()`, `ws.pane_state(id).and_then(...).and_then(|t| t.border_label(...))` for labels.
- Commit messages: lowercase conventional commits, one subject line, no AI co-author line (per this repo's commit style).

---

### Task 1: Stack bar geometry and capping

**Files:**
- Modify: `src/popup_size.rs`
- Test: `src/popup_size.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub enum StackBarKind { Pane(PaneId), Summary { count: usize } }`
  - `pub struct StackBar { pub rect: Rect, pub kind: StackBarKind }`
  - `pub(crate) fn stack_bar_rects(hidden: &[PaneId], popup_outer: Rect, area: Rect) -> Vec<StackBar>`
  - `hidden` is back-to-front (oldest first) — the same order as `Tab.floats[..floats.len() - 1]`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/popup_size.rs` (after the existing `use super::PopupSize;` line, add a second `use` line, then these tests alongside the existing ones):

```rust
    use super::{stack_bar_rects, StackBar, StackBarKind, MAX_STACK_PREVIEW_ROWS};

    #[test]
    fn stack_bar_rects_is_empty_when_no_hidden_floats() {
        let popup = super::super::Rect::new(5, 10, 20, 6);
        let area = super::super::Rect::new(0, 0, 80, 24);
        assert!(stack_bar_rects(&[], popup, area).is_empty());
    }

    #[test]
    fn stack_bar_rects_renders_one_bar_per_hidden_float_when_space_allows() {
        let hidden = [
            crate::layout::PaneId::from_raw(1),
            crate::layout::PaneId::from_raw(2),
        ];
        let popup = super::super::Rect::new(5, 10, 20, 6);
        let area = super::super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].rect, super::super::Rect::new(5, 8, 20, 1));
        assert!(matches!(bars[0].kind, StackBarKind::Pane(id) if id == hidden[0]));
        assert_eq!(bars[1].rect, super::super::Rect::new(5, 9, 20, 1));
        assert!(matches!(bars[1].kind, StackBarKind::Pane(id) if id == hidden[1]));
    }

    #[test]
    fn stack_bar_rects_folds_overflow_into_a_summary_row_when_space_is_tight() {
        let hidden: Vec<_> = (1..=5).map(crate::layout::PaneId::from_raw).collect();
        let popup = super::super::Rect::new(5, 2, 20, 6);
        let area = super::super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        // space_above = popup.y(2) - area.y(0) = 2, so only 2 rows fit:
        // 1 summary row + 1 real bar for the most-recently-hidden float.
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].rect, super::super::Rect::new(5, 0, 20, 1));
        assert!(matches!(bars[0].kind, StackBarKind::Summary { count: 4 }));
        assert_eq!(bars[1].rect, super::super::Rect::new(5, 1, 20, 1));
        assert!(matches!(bars[1].kind, StackBarKind::Pane(id) if id == hidden[4]));
    }

    #[test]
    fn stack_bar_rects_caps_at_max_stack_preview_rows_even_with_room_to_spare() {
        let hidden: Vec<_> = (1..=20).map(crate::layout::PaneId::from_raw).collect();
        let popup = super::super::Rect::new(5, 15, 20, 6);
        let area = super::super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        assert_eq!(bars.len(), MAX_STACK_PREVIEW_ROWS as usize);
        assert!(matches!(bars[0].kind, StackBarKind::Summary { count: 13 }));
        assert!(matches!(bars[1].kind, StackBarKind::Pane(id) if id == hidden[13]));
        assert!(matches!(bars[7].kind, StackBarKind::Pane(id) if id == hidden[19]));
    }

    #[test]
    fn stack_bar_rects_is_empty_when_popup_touches_the_top_edge() {
        let hidden = [crate::layout::PaneId::from_raw(1)];
        let popup = super::super::Rect::new(5, 0, 20, 6);
        let area = super::super::Rect::new(0, 0, 80, 24);
        assert!(stack_bar_rects(&hidden, popup, area).is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo nextest run --locked popup_size::tests::stack_bar_rects -v`
Expected: FAIL — `stack_bar_rects`, `StackBar`, `StackBarKind`, `MAX_STACK_PREVIEW_ROWS` don't exist yet.

- [ ] **Step 3: Implement the geometry function**

In `src/popup_size.rs`, add `use crate::layout::PaneId;` to the imports at the top (after `use std::borrow::Cow;`):

```rust
use std::borrow::Cow;

use crate::layout::PaneId;
use ratatui::layout::Rect;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
```

Then, immediately after the closing brace of `resolve_popup_geometry` (after the line `}` that ends that function, before `impl Serialize for PopupSize {`), add:

```rust
/// Cap on preview rows drawn above a floating pane's stack, regardless of
/// how many floats are hidden or how much vertical space is free.
const MAX_STACK_PREVIEW_ROWS: u16 = 8;

/// What a single stack preview row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackBarKind {
    /// A hidden float; clicking this row's rect brings it to the front.
    Pane(PaneId),
    /// Folds `count` further hidden floats that didn't fit as individual rows.
    Summary { count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackBar {
    pub rect: Rect,
    pub kind: StackBarKind,
}

/// One preview row per entry in `hidden`, stacked directly above
/// `popup_outer`, capped to the space between `area`'s top edge and the
/// popup and to `MAX_STACK_PREVIEW_ROWS`. `hidden` is ordered back-to-front
/// (oldest first); when not everything fits, the oldest floats are folded
/// into a single summary row at the top of the stack.
pub(crate) fn stack_bar_rects(hidden: &[PaneId], popup_outer: Rect, area: Rect) -> Vec<StackBar> {
    if hidden.is_empty() {
        return Vec::new();
    }
    let space_above = popup_outer.y.saturating_sub(area.y);
    let max_bars = space_above.min(MAX_STACK_PREVIEW_ROWS) as usize;
    if max_bars == 0 {
        return Vec::new();
    }

    let kinds: Vec<StackBarKind> = if hidden.len() <= max_bars {
        hidden.iter().map(|id| StackBarKind::Pane(*id)).collect()
    } else {
        let real_bars = max_bars - 1;
        let folded = hidden.len() - real_bars;
        let mut kinds = vec![StackBarKind::Summary { count: folded }];
        kinds.extend(
            hidden[hidden.len() - real_bars..]
                .iter()
                .map(|id| StackBarKind::Pane(*id)),
        );
        kinds
    };

    let total = kinds.len() as u16;
    kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| StackBar {
            rect: Rect::new(
                popup_outer.x,
                popup_outer.y.saturating_sub(total - i as u16),
                popup_outer.width,
                1,
            ),
            kind,
        })
        .collect()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked popup_size::tests::stack_bar_rects -v`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add src/popup_size.rs
git commit -m "feat: compute stacked floating pane preview bar geometry"
```

---

### Task 2: Wire stack bars through the view pipeline and render them

**Files:**
- Modify: `src/ui/tab_surface.rs`
- Modify: `src/app/state.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/ui.rs`
- Modify: `src/ui/panes.rs`
- Test: `src/ui/tab_surface.rs` (existing `#[cfg(test)] mod tests`), `src/ui/panes.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `StackBar`, `StackBarKind` from Task 1 (`crate::popup_size`); `resolve_popup_geometry` (existing, `crate::popup_size`); `Tab::top_float()`, `Tab::floats` (existing, via `Workspace`'s `Deref<Target = Tab>`); `Workspace::pane_state(PaneId) -> Option<&PaneState>` (existing); `TerminalState::border_label(bool) -> Option<String>` (existing); `pane_border_title(&str, u16, bool) -> Option<String>` (existing, `src/ui/panes.rs`).
- Produces:
  - `TabSurfaceLayout.stack_bars: Vec<StackBar>`, `TabSurfaceView.stack_bars: &[StackBar]`, `ViewState.stack_bars: Vec<StackBar>` — the full preview row list (including summary rows), for rendering.
  - `render_panes` gains a `stack_bars: &[StackBar]` parameter (5th positional param, after `split_borders`).
  - Every real (non-summary) stack bar also gets a synthetic `PaneInfo` appended to `TabSurfaceLayout.pane_infos` / `ViewState.pane_infos`, so the existing `pane_at()` → `focus_pane_before_mouse_press()` click path finds it.

- [ ] **Step 1: Write the failing tests**

In `src/ui/tab_surface.rs`, inside the existing `#[cfg(test)] mod tests` block (after the last test, before the closing `}` of the module), add:

```rust
    #[tokio::test]
    async fn compute_tab_surface_adds_click_targets_and_stack_bars_for_hidden_floats() {
        let mut workspace = Workspace::test_new("stack");
        let root_pane = workspace.tabs[0].root_pane;
        let float_a = crate::layout::PaneId::from_raw(301);
        let float_b = crate::layout::PaneId::from_raw(302);
        let float_c = crate::layout::PaneId::from_raw(303);
        for id in [float_a, float_b, float_c] {
            workspace.tabs[0].push_float(
                id,
                crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
            );
        }

        let mut app = AppState::test_new();
        app.workspaces = vec![workspace];
        app.active = Some(0);
        app.mode = Mode::Terminal;

        let area = Rect::new(0, 0, 100, 40);
        let surface = compute_tab_surface(
            &app,
            &TerminalRuntimeRegistry::new(),
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );

        assert_eq!(surface.stack_bars.len(), 2);
        assert!(matches!(
            surface.stack_bars[0].kind,
            crate::popup_size::StackBarKind::Pane(id) if id == float_a
        ));
        assert!(matches!(
            surface.stack_bars[1].kind,
            crate::popup_size::StackBarKind::Pane(id) if id == float_b
        ));

        let pane_ids: Vec<_> = surface.pane_infos.iter().map(|info| info.id).collect();
        assert!(pane_ids.contains(&root_pane));
        assert!(pane_ids.contains(&float_c));
        assert!(pane_ids.contains(&float_a));
        assert!(pane_ids.contains(&float_b));
        assert_eq!(pane_ids.len(), 4);

        let bar_a = surface
            .pane_infos
            .iter()
            .find(|info| info.id == float_a)
            .expect("bar hit-test entry for float_a");
        assert_eq!(bar_a.rect, surface.stack_bars[0].rect);
        assert_eq!(bar_a.borders, ratatui::widgets::Borders::NONE);
    }
```

In `src/ui/panes.rs`, inside the existing `#[cfg(test)] mod tests` block, add (this can go right after `pane_scrollbar_gutter_is_reserved_before_scrollback_exists` or any other existing test):

```rust
    #[test]
    fn render_panes_draws_a_stack_bar_with_its_pane_label_above_the_popup() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 30, 10);

        let mut ws = Workspace::test_new("test");
        let hidden_id = PaneId::from_raw(50);
        let top_id = PaneId::from_raw(51);
        let hidden_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            hidden_id,
            crate::pane::PaneState::new(hidden_terminal_id.clone()),
        );
        ws.tabs[0].push_float(
            top_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );

        let mut terminal_state = TerminalState::new(hidden_terminal_id.clone(), "/tmp".into());
        terminal_state.set_manual_label("claude".into());
        app.terminals.insert(hidden_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let bar = crate::popup_size::StackBar {
            rect: Rect::new(5, 3, 20, 1),
            kind: crate::popup_size::StackBarKind::Pane(hidden_id),
        };

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 10)).unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    &[],
                    &[],
                    std::slice::from_ref(&bar),
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (5..25).map(|x| buffer[(x, 3)].symbol()).collect();
        assert!(row.contains("claude"), "bar row: {row:?}");
        assert_eq!(buffer[(5, 3)].symbol(), "│");
        assert_eq!(buffer[(24, 3)].symbol(), "│");
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo nextest run --locked -E 'test(compute_tab_surface_adds_click_targets) + test(render_panes_draws_a_stack_bar)' -v`
Expected: FAIL — `TabSurfaceLayout`/`TabSurfaceView` have no `stack_bars` field, `render_panes` takes 4 positional args not 5.

- [ ] **Step 3: Thread `StackBar` through `TabSurfaceLayout`/`TabSurfaceView` and add click-hit-test entries**

In `src/ui/tab_surface.rs`, change the top import line:

```rust
use ratatui::{layout::Rect, widgets::Borders, Frame};

use super::panes::{compute_pane_infos, render_panes, resize_tab_panes};
use crate::app::state::ViewState;
use crate::app::{AppState, Mode};
use crate::layout::{PaneInfo, SplitBorder};
use crate::popup_size::{resolve_popup_geometry, stack_bar_rects, StackBar, StackBarKind};
use crate::protocol::CursorState;
use crate::terminal::TerminalRuntimeRegistry;
```

Change `TabSurfaceLayout` and `TabSurfaceView`:

```rust
pub(crate) struct TabSurfaceLayout {
    pub(crate) pane_infos: Vec<PaneInfo>,
    pub(crate) split_borders: Vec<SplitBorder>,
    pub(crate) stack_bars: Vec<StackBar>,
}

#[derive(Clone, Copy)]
pub(crate) struct TabSurfaceView<'a> {
    pub(crate) pane_infos: &'a [PaneInfo],
    pub(crate) split_borders: &'a [SplitBorder],
    pub(crate) stack_bars: &'a [StackBar],
}

impl ViewState {
    pub(crate) fn tab_surface(&self) -> TabSurfaceView<'_> {
        TabSurfaceView {
            pane_infos: &self.pane_infos,
            split_borders: &self.split_borders,
            stack_bars: &self.stack_bars,
        }
    }
}
```

Change `compute_tab_surface` to also compute and attach the bars:

```rust
pub(crate) fn compute_tab_surface(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> TabSurfaceLayout {
    let split_borders = app
        .active
        .and_then(|i| app.workspaces.get(i))
        .map(|ws| {
            if ws.zoomed {
                Vec::new()
            } else {
                ws.layout.splits(area)
            }
        })
        .unwrap_or_default();
    let mut pane_infos = compute_pane_infos(app, terminal_runtimes, area, resize_panes, cell_size);
    let stack_bars = compute_stack_bars(app, area);
    for bar in &stack_bars {
        if let StackBarKind::Pane(pane_id) = bar.kind {
            pane_infos.push(PaneInfo {
                id: pane_id,
                rect: bar.rect,
                inner_rect: bar.rect,
                scrollbar_rect: None,
                borders: Borders::NONE,
                is_focused: false,
            });
        }
    }

    TabSurfaceLayout {
        pane_infos,
        split_borders,
        stack_bars,
    }
}

/// Preview rows for the floats hidden behind the active tab's visible
/// floating popup, back-to-front (oldest first). Empty when there is no
/// floating popup showing or nothing is hidden behind it.
fn compute_stack_bars(app: &AppState, area: Rect) -> Vec<StackBar> {
    let Some(ws_idx) = app.active else {
        return Vec::new();
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return Vec::new();
    };
    if ws.top_float().is_none() {
        return Vec::new();
    }
    let Some(geometry) =
        resolve_popup_geometry(app.floating_pane_width, app.floating_pane_height, area)
    else {
        return Vec::new();
    };
    let hidden = &ws.floats[..ws.floats.len() - 1];
    stack_bar_rects(hidden, geometry.outer, area)
}
```

Change `render_tab_surface` to pass the bars through:

```rust
pub(crate) fn render_tab_surface(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    surface: TabSurfaceView<'_>,
    frame: &mut Frame,
) {
    render_panes(
        app,
        terminal_runtimes,
        frame,
        surface.pane_infos,
        surface.split_borders,
        surface.stack_bars,
    );
}
```

In the same file's test module, update the manually-constructed `TabSurfaceView` literal in `explicit_surface_layout_drives_render_cursor_and_hyperlinks`:

```rust
        let surface_view = TabSurfaceView {
            pane_infos: &surface.pane_infos,
            split_borders: &surface.split_borders,
            stack_bars: &surface.stack_bars,
        };
```

- [ ] **Step 4: Add the `stack_bars` field to `ViewState` and its constructors**

In `src/app/state.rs`, find the `use crate::layout::{PaneId, PaneInfo, SplitBorder};` import line and add a new import line after it:

```rust
use crate::popup_size::StackBar;
```

In the `ViewState` struct definition, add the field after `pub split_borders: Vec<SplitBorder>,`:

```rust
    pub split_borders: Vec<SplitBorder>,
    pub stack_bars: Vec<StackBar>,
}
```

At `src/app/state.rs:1808` (the `view: ViewState { ... }` literal inside `AppState::test_new()` or equivalent constructor), add `stack_bars: Vec::new(),` after `split_borders: Vec::new(),`:

```rust
                pane_infos: Vec::new(),
                split_borders: Vec::new(),
                stack_bars: Vec::new(),
            },
```

In `src/app/mod.rs` at the `view: state::ViewState { ... }` literal (around line 575), make the same change:

```rust
                pane_infos: Vec::new(),
                split_borders: Vec::new(),
                stack_bars: Vec::new(),
            },
```

- [ ] **Step 5: Update the two `compute_view` call sites in `src/ui.rs`**

Around line 279 (desktop view), change the destructure and the `ViewState { ... }` literal:

```rust
    let TabSurfaceLayout {
        pane_infos,
        split_borders,
        stack_bars,
    } = compute_tab_surface(
        app,
        terminal_runtimes,
        terminal_area,
        resize_panes,
        cell_size,
    );
```

```rust
    app.view = crate::app::ViewState {
        layout: ViewLayout::Desktop,
        sidebar_rect: sidebar_area,
        workspace_card_areas,
        tab_bar_rect,
        tab_hit_areas: tab_bar_view.tab_hit_areas,
        tab_scroll_left_hit_area: tab_bar_view.scroll_left_hit_area,
        tab_scroll_right_hit_area: tab_bar_view.scroll_right_hit_area,
        new_tab_hit_area: tab_bar_view.new_tab_hit_area,
        terminal_area,
        mobile_header_rect: Rect::default(),
        mobile_menu_hit_area: Rect::default(),
        toast_hit_area,
        pane_infos,
        split_borders,
        stack_bars,
    };
```

Around line 348 (mobile view), make the matching change to both the destructure and the `ViewState { ... }` literal:

```rust
    let TabSurfaceLayout {
        pane_infos,
        split_borders,
        stack_bars,
    } = compute_tab_surface(
        app,
        terminal_runtimes,
        terminal_area,
        resize_panes,
        cell_size,
    );
```

```rust
    app.view = crate::app::ViewState {
        layout: ViewLayout::Mobile,
        sidebar_rect: Rect::default(),
        workspace_card_areas: Vec::new(),
        tab_bar_rect: Rect::default(),
        tab_hit_areas: Vec::new(),
        tab_scroll_left_hit_area: Rect::default(),
        tab_scroll_right_hit_area: Rect::default(),
        new_tab_hit_area: Rect::default(),
        terminal_area,
        mobile_header_rect: header_rect,
        mobile_menu_hit_area: header_hits.menu,
        toast_hit_area,
        pane_infos,
        split_borders,
        stack_bars,
    };
```

- [ ] **Step 6: Render the bars in `render_panes`**

In `src/ui/panes.rs`, change the import at line 17:

```rust
use crate::popup_size::{resolve_popup_geometry, StackBar, StackBarKind};
```

Change the `render_panes` signature (around line 337) to accept the bars:

```rust
pub(super) fn render_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    pane_infos: &[PaneInfo],
    split_borders: &[crate::layout::SplitBorder],
    stack_bars: &[StackBar],
) {
```

Immediately after the existing `if let Some(float_id) = ws.top_float() { ... }` block (the one that draws the popup's own border and content — its closing `}` is directly before the trailing `render_pane_borders(app, ws, pane_infos, split_borders, frame);` call), add the bar-drawing loop:

```rust
    for bar in stack_bars {
        render_stack_bar(app, ws, frame, bar);
    }

    render_pane_borders(app, ws, pane_infos, split_borders, frame);
}
```

Add the new helper function right after `render_panes`'s closing brace (before `pub(crate) fn popup_pane_rects`):

```rust
fn render_stack_bar(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    frame: &mut Frame,
    bar: &StackBar,
) {
    let label = match bar.kind {
        StackBarKind::Pane(pane_id) => ws
            .pane_state(pane_id)
            .and_then(|pane| app.terminals.get(&pane.attached_terminal_id))
            .and_then(|terminal| terminal.border_label(app.show_agent_labels_on_pane_borders))
            .unwrap_or_else(|| "float".to_string()),
        StackBarKind::Summary { count } => format!("+{count} more"),
    };
    let text = pane_border_title(&label, bar.rect.width, false).unwrap_or_default();
    let style = Style::default()
        .fg(app.palette.overlay0)
        .bg(app.palette.panel_bg);
    frame.render_widget(Clear, bar.rect);
    frame.render_widget(
        Paragraph::new(Line::from(text)).style(style).block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(style),
        ),
        bar.rect,
    );
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo nextest run --locked -E 'test(compute_tab_surface_adds_click_targets) + test(render_panes_draws_a_stack_bar) + test(explicit_surface_layout_drives_render_cursor_and_hyperlinks)' -v`
Expected: PASS (3 tests)

- [ ] **Step 8: Run the full test suite to catch any other call site**

Run: `cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never`
Expected: PASS. If anything else fails to compile, it's another `TabSurfaceLayout`/`TabSurfaceView`/`ViewState` literal that step 3-5 missed — search with `rg -n "TabSurfaceView \{|TabSurfaceLayout \{|ViewState \{"` and add `stack_bars`/`stack_bars: &[]` there too.

- [ ] **Step 9: Commit**

```bash
git add src/ui/tab_surface.rs src/app/state.rs src/app/mod.rs src/ui.rs src/ui/panes.rs
git commit -m "feat: render floating pane stack preview bars and wire click-to-focus"
```

---

### Task 3: Mask tiled borders under the whole stack, not just the popup

**Files:**
- Modify: `src/ui/panes.rs`
- Test: `src/ui/panes.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing new — this fixes `float_rect()`, an existing private helper in `src/ui/panes.rs`, which after Task 2 sees multiple `is_float` `PaneInfo` entries (the popup plus each bar) instead of just one.
- Produces: `float_rect()` keeps its existing signature and callers (`render_pane_borders`, `render_pane_border_titles`); its return value changes from "first float's rect" to "union of every float's rect".

- [ ] **Step 1: Write the failing test**

In `src/ui/panes.rs`'s `#[cfg(test)] mod tests`, add a test right after `tiled_split_borders_do_not_draw_over_a_float`:

```rust
    #[test]
    fn float_rect_returns_the_union_of_every_float_pane_info() {
        let bar_id = PaneId::from_raw(1);
        let float_id = PaneId::from_raw(2);
        let pane_infos = vec![
            PaneInfo {
                id: bar_id,
                rect: Rect::new(5, 3, 20, 1),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::NONE,
                is_focused: false,
            },
            PaneInfo {
                id: float_id,
                rect: Rect::new(5, 4, 20, 6),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
        ];
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].push_float(
            bar_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );

        assert_eq!(float_rect(&ws, &pane_infos), Some(Rect::new(5, 3, 20, 7)));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --locked float_rect_returns_the_union -v`
Expected: FAIL — current `float_rect` returns `Some(Rect::new(5, 3, 20, 1))` (the first match, `bar_id`'s rect), not the union.

- [ ] **Step 3: Fix `float_rect` to compute the union**

In `src/ui/panes.rs`, replace the `float_rect` function:

```rust
fn float_rect(ws: &crate::workspace::Workspace, pane_infos: &[PaneInfo]) -> Option<Rect> {
    pane_infos
        .iter()
        .filter(|info| ws.is_float(info.id))
        .map(|info| info.rect)
        .reduce(union_rect)
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let bottom = a.y.saturating_add(a.height).max(b.y.saturating_add(b.height));
    Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked -E 'test(float_rect_returns_the_union) + test(tiled_split_borders_do_not_draw_over_a_float)' -v`
Expected: PASS (2 tests) — the new union test, and the pre-existing single-float test (which still passes because a union of one rect is that rect).

- [ ] **Step 5: Commit**

```bash
git add src/ui/panes.rs
git commit -m "fix: mask tiled borders under the full floating pane stack"
```

---

### Task 4: Prove clicking a stack bar raises and focuses that float

**Files:**
- Modify: `src/app/input/terminal.rs`

**Interfaces:**
- Consumes: `App::handle_mouse` (existing), `Tab::push_float`, `Tab::top_float`, `Tab.float_focused` (existing) — no new production code in this task. This is a regression test proving the claim in the spec ("no new input-handling code is needed") holds, using the synthetic `PaneInfo` a real bar produces (per Task 2).

- [ ] **Step 1: Write the test**

In `src/app/input/terminal.rs`'s `#[cfg(test)] mod tests`, add:

```rust
    #[tokio::test]
    async fn clicking_a_stack_bar_raises_that_float_and_focuses_it() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let hidden_float = crate::layout::PaneId::from_raw(201);
        let top_float = crate::layout::PaneId::from_raw(202);
        ws.tabs[0].push_float(
            hidden_float,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );
        ws.tabs[0].push_float(
            top_float,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );

        let bar_rect = Rect::new(30, 4, 40, 1);
        app.state.view.pane_infos = vec![crate::layout::PaneInfo {
            id: hidden_float,
            rect: bar_rect,
            inner_rect: bar_rect,
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::NONE,
            is_focused: false,
        }];

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 35, 4));

        let ws = &app.state.workspaces[0];
        assert_eq!(ws.tabs[0].top_float(), Some(hidden_float));
        assert!(ws.tabs[0].float_focused);
    }
```

- [ ] **Step 2: Run the test to verify it passes**

Run: `cargo nextest run --locked clicking_a_stack_bar_raises_that_float -v`
Expected: PASS — `app.handle_mouse` routes the click through `focus_pane_before_mouse_press` → `pane_at()` (matches `bar_rect`) → `focus_pane_internal_via_api` → `AppState::focus_pane_in_workspace`, which raises `hidden_float` to the top of `Tab.floats` and sets `float_focused = true`. This is all pre-existing code; the test should pass on the first run without any further implementation changes. If it fails, do not add new production code to force it green — investigate why the existing focus path doesn't reach a bar's synthetic `PaneInfo` and report back before changing anything.

- [ ] **Step 3: Run the full test suite**

Run: `cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/app/input/terminal.rs
git commit -m "test: cover click-to-focus for a stacked floating pane bar"
```

---

## Final Verification

- [ ] Run `just check` from the repo root and confirm it passes (formatting, clippy with `-D warnings` on both the native and Windows target, the full nextest suite, and maintenance script tests).
- [ ] Manually verify with `cargo run -- ...` (per this repo's dev-server instructions) by opening 3+ floating panes in a small terminal and confirming: preview bars appear above the popup, labels match each pane's agent/title, clicking a bar brings it to front, and opening enough floats to exceed 8 collapses the rest into a `"+N more"` row.
