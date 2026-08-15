use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use super::scrollbar::{render_pane_scrollbar, should_show_scrollbar};
#[cfg(test)]
use super::text::display_width;
use super::text::truncate_end;
use super::widgets::panel_contrast_fg;
use crate::app::state::Palette;
use crate::app::{AppState, Mode};
use crate::layout::PaneInfo;
use crate::popup_size::{resolve_popup_geometry, StackBar, StackBarKind};
use crate::terminal::{TerminalRuntime, TerminalRuntimeRegistry};

pub(crate) fn pane_is_scrolled_back(rt: &TerminalRuntime) -> bool {
    rt.scroll_metrics()
        .is_some_and(|metrics| metrics.offset_from_bottom > 0)
}

/// The name to draw on a pane's own chrome. Falls back to the pane's public
/// number rather than its layer, so a stacked tiled pane is never called
/// "float".
fn pane_label(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    pane_id: crate::layout::PaneId,
) -> String {
    ws.terminal_id(pane_id)
        .and_then(|terminal_id| app.terminals.get(terminal_id))
        .and_then(|terminal| terminal.pane_label(app.show_agent_labels_on_pane_borders))
        .or_else(|| {
            ws.public_pane_numbers
                .get(&pane_id)
                .map(|number| format!("pane {number}"))
        })
        .unwrap_or_default()
}

fn pane_border_title(label: &str, pane_width: u16, _focused: bool) -> Option<String> {
    let label = label.trim();
    if label.is_empty() || pane_width <= 4 {
        return None;
    }
    let max_label_width = pane_width.saturating_sub(4) as usize;
    Some(format!(" {} ", truncate_end(label, max_label_width)))
}

fn stable_terminal_inner_rect(pane_inner: Rect, pane_scrollbars: bool) -> Rect {
    if !pane_scrollbars || pane_inner.width <= 4 {
        return pane_inner;
    }

    Rect::new(
        pane_inner.x,
        pane_inner.y,
        pane_inner.width.saturating_sub(1),
        pane_inner.height,
    )
}

pub(crate) fn pane_inner_rect(area: Rect, borders: Borders) -> Rect {
    if borders.is_empty() {
        area
    } else {
        Block::default().borders(borders).inner(area)
    }
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start.saturating_add(b_len) && b_start < a_start.saturating_add(a_len)
}

fn pane_to_right<'a>(info: &PaneInfo, panes: &'a [PaneInfo]) -> Option<&'a PaneInfo> {
    let right = info.rect.x.saturating_add(info.rect.width);
    panes.iter().find(|other| {
        other.id != info.id
            && other.rect.x == right
            && ranges_overlap(
                info.rect.y,
                info.rect.height,
                other.rect.y,
                other.rect.height,
            )
    })
}

fn pane_below<'a>(info: &PaneInfo, panes: &'a [PaneInfo]) -> Option<&'a PaneInfo> {
    let bottom = info.rect.y.saturating_add(info.rect.height);
    panes.iter().find(|other| {
        other.id != info.id
            && other.rect.y == bottom
            && ranges_overlap(info.rect.x, info.rect.width, other.rect.x, other.rect.width)
    })
}

fn shrink_for_one_cell_gap(size: u16) -> u16 {
    if size > 1 {
        size - 1
    } else {
        size
    }
}

/// A lone pane has no split borders to hang its name on, so it keeps a one-row
/// strip across the top for the title. Both the resize and the render path read
/// this: they must agree, or the PTY is sized to a box the pane never gets.
pub(crate) const LONE_PANE_BORDERS: Borders = Borders::TOP;

pub(crate) fn apply_pane_chrome(
    panes: Vec<PaneInfo>,
    pane_borders: bool,
    pane_gaps: bool,
    pane_outer_borders: bool,
) -> Vec<PaneInfo> {
    let multi_pane = panes.len() > 1;
    let outer_left = panes.iter().map(|info| info.rect.x).min().unwrap_or(0);
    let outer_top = panes.iter().map(|info| info.rect.y).min().unwrap_or(0);
    let outer_right = panes
        .iter()
        .map(|info| info.rect.x.saturating_add(info.rect.width))
        .max()
        .unwrap_or(0);
    let outer_bottom = panes
        .iter()
        .map(|info| info.rect.y.saturating_add(info.rect.height))
        .max()
        .unwrap_or(0);
    panes
        .iter()
        .cloned()
        .map(|mut info| {
            let right_neighbor = multi_pane.then(|| pane_to_right(&info, &panes)).flatten();
            let below_neighbor = multi_pane.then(|| pane_below(&info, &panes)).flatten();

            if multi_pane && pane_gaps && !pane_borders {
                if right_neighbor.is_some() {
                    info.rect.width = shrink_for_one_cell_gap(info.rect.width);
                }
                if below_neighbor.is_some() {
                    info.rect.height = shrink_for_one_cell_gap(info.rect.height);
                }
            }

            info.borders = if !pane_borders {
                Borders::NONE
            } else if !multi_pane {
                LONE_PANE_BORDERS
            } else {
                let mut borders = Borders::ALL;
                if !pane_gaps {
                    if right_neighbor.is_some() {
                        borders.remove(Borders::RIGHT);
                    }
                    if below_neighbor.is_some() {
                        borders.remove(Borders::BOTTOM);
                    }
                }
                if !pane_outer_borders {
                    if info.rect.x == outer_left {
                        borders.remove(Borders::LEFT);
                    }
                    if info.rect.y == outer_top {
                        borders.remove(Borders::TOP);
                    }
                    if info.rect.x.saturating_add(info.rect.width) == outer_right {
                        borders.remove(Borders::RIGHT);
                    }
                    if info.rect.y.saturating_add(info.rect.height) == outer_bottom {
                        borders.remove(Borders::BOTTOM);
                    }
                }
                borders
            };
            info
        })
        .collect()
}

fn runtime_for_tab_pane<'a>(
    terminal_runtimes: &'a TerminalRuntimeRegistry,
    tab: &'a crate::workspace::Tab,
    pane_id: crate::layout::PaneId,
) -> Option<(&'a crate::terminal::TerminalId, &'a TerminalRuntime)> {
    let terminal_id = tab.terminal_id(pane_id)?;
    #[cfg(test)]
    if let Some(runtime) = tab.runtimes.get(&pane_id) {
        return Some((terminal_id, runtime));
    }
    terminal_runtimes
        .get(terminal_id)
        .map(|runtime| (terminal_id, runtime))
}

fn stable_scrollbar_gutter(
    rt: &TerminalRuntime,
    pane_inner: Rect,
    pane_scrollbars: bool,
) -> (Rect, Option<Rect>) {
    let inner_rect = stable_terminal_inner_rect(pane_inner, pane_scrollbars);
    if inner_rect == pane_inner {
        return (inner_rect, None);
    }
    let gutter = Rect::new(
        pane_inner.x + pane_inner.width.saturating_sub(1),
        pane_inner.y,
        1,
        pane_inner.height,
    );
    let scrollbar_rect = rt
        .scroll_metrics()
        .filter(|metrics| should_show_scrollbar(*metrics))
        .map(|_| gutter);

    (inner_rect, scrollbar_rect)
}

/// Resize every visible runtime in a tab to the geometry it would receive if the tab were selected.
pub(super) fn resize_tab_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    tab: &crate::workspace::Tab,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    let multi_pane = tab.layout.pane_count() > 1;

    if tab.zoomed {
        let focused_id = tab.layout.focused();
        if let Some((terminal_id, rt)) = runtime_for_tab_pane(terminal_runtimes, tab, focused_id) {
            let borders = if multi_pane && app.pane_borders && app.pane_outer_borders {
                Borders::ALL
            } else if !multi_pane && app.pane_borders {
                LONE_PANE_BORDERS
            } else {
                Borders::NONE
            };
            let pane_inner = pane_inner_rect(area, borders);
            let inner_rect = stable_terminal_inner_rect(pane_inner, app.pane_scrollbars);
            if !app.direct_attach_resize_locks.contains(terminal_id) {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        return;
    }

    for info in apply_pane_chrome(
        tab.layout.panes(area),
        app.pane_borders,
        app.pane_gaps,
        app.pane_outer_borders,
    ) {
        let pane_inner = pane_inner_rect(info.rect, info.borders);

        if let Some((terminal_id, rt)) = runtime_for_tab_pane(terminal_runtimes, tab, info.id) {
            let inner_rect = stable_terminal_inner_rect(pane_inner, app.pane_scrollbars);
            if !app.direct_attach_resize_locks.contains(terminal_id) {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
    }
}

/// Compute pane layout info and optionally resize pane runtimes to match.
pub(super) fn compute_pane_infos(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Vec<PaneInfo> {
    let Some(ws_idx) = app.active else {
        return Vec::new();
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return Vec::new();
    };

    let multi_pane = ws.layout.pane_count() > 1;

    let mut pane_infos = if ws.zoomed {
        let focused_id = ws.layout.focused();
        let borders = if multi_pane && app.pane_borders && app.pane_outer_borders {
            Borders::ALL
        } else if !multi_pane && app.pane_borders {
            LONE_PANE_BORDERS
        } else {
            Borders::NONE
        };
        let pane_inner = pane_inner_rect(area, borders);
        let mut inner_rect = pane_inner;
        let mut scrollbar_rect = None;
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, focused_id) {
            (inner_rect, scrollbar_rect) =
                stable_scrollbar_gutter(rt, pane_inner, app.pane_scrollbars);
            if resize_panes
                && ws.terminal_id(focused_id).is_some_and(|terminal_id| {
                    !app.direct_attach_resize_locks.contains(terminal_id)
                })
            {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        vec![PaneInfo {
            id: focused_id,
            rect: area,
            inner_rect,
            scrollbar_rect,
            borders,
            is_focused: !ws.float_focused,
        }]
    } else {
        let mut pane_infos = apply_pane_chrome(
            ws.layout.panes(area),
            app.pane_borders,
            app.pane_gaps,
            app.pane_outer_borders,
        );

        for info in &mut pane_infos {
            let pane_inner = pane_inner_rect(info.rect, info.borders);

            let mut inner_rect = pane_inner;
            let mut scrollbar_rect = None;
            if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)
            {
                (inner_rect, scrollbar_rect) =
                    stable_scrollbar_gutter(rt, pane_inner, app.pane_scrollbars);
                if resize_panes
                    && ws.terminal_id(info.id).is_some_and(|terminal_id| {
                        !app.direct_attach_resize_locks.contains(terminal_id)
                    })
                {
                    rt.resize(
                        inner_rect.height,
                        inner_rect.width,
                        cell_size.width_px,
                        cell_size.height_px,
                    );
                }
            }

            info.inner_rect = inner_rect;
            info.scrollbar_rect = scrollbar_rect;
            info.is_focused = !ws.float_focused && info.is_focused;
        }

        pane_infos
    };

    // Floats are appended last so hit-tests that iterate in reverse find them
    // before any tiled pane underneath. Every member gets a `PaneInfo` — even
    // a collapsed or folded one — the same way a tiled `Node::Stack` does, so
    // the render pass can reuse that stack's bar/fold handling unchanged.
    //
    // A hidden layer emits nothing at all. Drawing, PTY resizing, mouse
    // hit-testing, hyperlink scanning and graphics all key off this list, so
    // this one gate is what `floats_hidden` means everywhere downstream.
    if let Some(layout) = ws.float_layout.as_ref().filter(|_| !ws.floats_hidden) {
        if let Some(geometry) =
            resolve_popup_geometry(app.floating_pane_width, app.floating_pane_height, area)
        {
            let focused_float = ws.focused_float();
            for mut info in layout.panes(geometry.outer) {
                info.borders = Borders::ALL;
                // Floats get no scrollbar lane, matching the old popup pane;
                // `layout.panes` already leaves `scrollbar_rect: None`.
                info.inner_rect = pane_inner_rect(info.rect, info.borders);
                info.is_focused = ws.float_focused && Some(info.id) == focused_float;
                // Only the expanded member has content to display; resizing a
                // collapsed or folded float's PTY to its near-zero box would
                // reflow it for nothing.
                if resize_panes && info.rect.height > 1 {
                    if let Some(rt) =
                        app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)
                    {
                        rt.resize(
                            info.inner_rect.height,
                            info.inner_rect.width,
                            cell_size.width_px,
                            cell_size.height_px,
                        );
                    }
                }
                pane_infos.push(info);
            }
        }
    }

    pane_infos
}

pub(super) fn render_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    pane_infos: &[PaneInfo],
    split_borders: &[crate::layout::SplitBorder],
    stack_bars: &[StackBar],
) {
    let Some(ws_idx) = app.active else {
        return;
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return;
    };

    let multi_pane = ws.layout.pane_count() > 1;
    let terminal_active = app.mode == Mode::Terminal;

    for info in pane_infos {
        // Tiled stack members with height 0 or 1 have no content on screen —
        // `stack_rects` collapsed them to a bar or folded them out. They get
        // drawn as bars below instead.
        if ws.is_float(info.id) || info.rect.height <= 1 {
            continue;
        }
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id) {
            let show_cursor = info.is_focused
                && terminal_active
                && !pane_is_scrolled_back(rt)
                && app.pane_exposes_host_cursor(ws_idx, info.id);
            rt.render(frame, info.inner_rect, show_cursor);
            render_pane_scrollbar(app, frame, info, rt);

            let should_dim = !info.is_focused && multi_pane && !terminal_active;
            if should_dim {
                let inner = info.inner_rect;
                let buf = frame.buffer_mut();
                for y in inner.y..inner.y + inner.height {
                    for x in inner.x..inner.x + inner.width {
                        let cell = &mut buf[(x, y)];
                        cell.set_style(cell.style().add_modifier(Modifier::DIM));
                    }
                }
            }

            let (copy_search_top, copy_search_bottom, copy_search_matches) =
                validated_copy_mode_search_matches(app, info, rt);
            render_copy_mode_search_highlights(
                app,
                frame,
                info,
                copy_search_top,
                copy_search_bottom,
                &copy_search_matches,
                false,
            );
            render_selection_highlight(
                &app.selection,
                frame,
                info.id,
                info.inner_rect,
                rt.scroll_metrics(),
                &app.palette,
                app.host_terminal_theme,
            );
            render_copy_mode_search_highlights(
                app,
                frame,
                info,
                copy_search_top,
                copy_search_bottom,
                &copy_search_matches,
                true,
            );
            render_copy_mode_cursor(app, frame, info);
        }
    }

    for info in pane_infos
        .iter()
        .filter(|info| ws.is_float(info.id) && info.rect.height > 1)
    {
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id) {
            let title = pane_label(app, ws, info.id);
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(if info.is_focused {
                    app.palette.accent
                } else {
                    app.palette.overlay1
                }))
                .title(
                    pane_border_title(&title, info.rect.width, info.is_focused).unwrap_or_default(),
                )
                .style(Style::default().bg(app.palette.panel_bg));
            render_float_shadow(frame, info.rect, app.palette.surface_dim);
            frame.render_widget(Clear, info.rect);
            frame.render_widget(block, info.rect);
            let show_cursor = info.is_focused
                && terminal_active
                && !pane_is_scrolled_back(rt)
                && app.pane_exposes_host_cursor(ws_idx, info.id);
            rt.render(frame, info.inner_rect, show_cursor);
        }
    }

    let tiled_bars = tiled_stack_bars(ws, pane_infos);
    let float_bars = stack_bars_for(pane_infos.iter().filter(|info| ws.is_float(info.id)));
    for bar in stack_bars
        .iter()
        .chain(tiled_bars.iter())
        .chain(float_bars.iter())
    {
        render_stack_bar(app, ws, frame, bar);
    }

    render_pane_borders(app, ws, pane_infos, split_borders, stack_bars, frame);
}

/// A one-cell band down the right edge and along the bottom of a float, so it
/// reads as lifted off the tiled panes behind it rather than blending into
/// them. Drawn before the float's own `Clear`, over content already on screen.
fn render_float_shadow(frame: &mut Frame, rect: Rect, shadow: Color) {
    let area = frame.area();
    let strips = [
        Rect::new(rect.right(), rect.y.saturating_add(1), 1, rect.height),
        Rect::new(rect.x.saturating_add(1), rect.bottom(), rect.width, 1),
    ];
    let buf = frame.buffer_mut();
    for strip in strips {
        let strip = strip.intersection(area);
        for y in strip.top()..strip.bottom() {
            for x in strip.left()..strip.right() {
                buf[(x, y)].set_bg(shadow);
            }
        }
    }
}

/// Where to find an already-valid, currently-drawn row to repurpose as a
/// fold's `+N more` indicator, since a fold's own rect is never one (see
/// `close_fold_run`).
#[derive(Clone, Copy)]
enum FoldAnchor {
    /// Index into `bars` of an already-pushed collapsed-bar entry.
    Bar(usize),
    /// The stack's active member's own rect.
    Active(Rect),
}

struct ZeroRun {
    rect: Rect,
    count: usize,
    predecessor: Option<FoldAnchor>,
}

/// Derives collapsed (height 1) and folded (height 0) rows into `StackBar`s
/// from a set of already-laid-out stack members. `stack_rects` lays out a
/// tiled `Node::Stack` and the floating layer's stacked arrangement
/// identically, so this fold-detection is shared between them: a run of
/// consecutive height-0 entries sharing a rect position is one fold; a run
/// interrupted by the active member (a different rect) is a second, separate
/// fold, since they are genuinely different screen locations.
fn stack_bars_for<'a>(infos: impl Iterator<Item = &'a PaneInfo>) -> Vec<StackBar> {
    let mut bars: Vec<StackBar> = Vec::new();
    // The most recent non-folded (height >= 1) entry seen, and its x — a
    // fold's predecessor candidate, valid only while still inside the same
    // stack's column (stack members all share the same x and width).
    let mut last_real: Option<(u16, FoldAnchor)> = None;
    let mut zero_run: Option<ZeroRun> = None;

    for info in infos {
        if info.rect.height == 0 {
            let continues = zero_run
                .as_ref()
                .is_some_and(|run| run.rect.x == info.rect.x && run.rect.y == info.rect.y);
            if continues {
                if let Some(run) = zero_run.as_mut() {
                    run.count += 1;
                }
            } else {
                if let Some(run) = zero_run.take() {
                    close_fold_run(&mut bars, run, None);
                }
                let predecessor = last_real
                    .filter(|(x, _)| *x == info.rect.x)
                    .map(|(_, anchor)| anchor);
                zero_run = Some(ZeroRun {
                    rect: Rect::new(info.rect.x, info.rect.y, info.rect.width, 0),
                    count: 1,
                    predecessor,
                });
            }
            continue;
        }

        if let Some(run) = zero_run.take() {
            close_fold_run(&mut bars, run, Some(info.rect));
        }

        if info.rect.height == 1 {
            bars.push(StackBar {
                rect: info.rect,
                kind: StackBarKind::Pane(info.id),
            });
            last_real = Some((info.rect.x, FoldAnchor::Bar(bars.len() - 1)));
        } else {
            last_real = Some((info.rect.x, FoldAnchor::Active(info.rect)));
        }
    }
    if let Some(run) = zero_run.take() {
        close_fold_run(&mut bars, run, None);
    }
    bars
}

/// Bars for tiled `Node::Stack` members whose rect signals a collapsed or
/// folded row.
fn tiled_stack_bars(ws: &crate::workspace::Workspace, pane_infos: &[PaneInfo]) -> Vec<StackBar> {
    stack_bars_for(pane_infos.iter().filter(|info| !ws.is_float(info.id)))
}

/// `stack_rects` always consumes the whole stack area once any folding
/// happens, so a fold's own rect is never a real, drawable row: a fold
/// before the active member lands exactly on the active member's own first
/// row, and a fold after it lands one row past the end of the area — both
/// invalid to draw at directly. Borrow an already-valid row instead: the
/// last collapsed bar right before the fold if one exists (that pane's row
/// now reads as part of the fold too, so it joins the count); otherwise the
/// active member's near edge (its pane stays visible in its own rect, so
/// the count is unaffected) — a fold with no bar before it is always
/// immediately followed by the active member, since `stack_rects` never
/// folds the active member itself.
fn close_fold_run(bars: &mut Vec<StackBar>, run: ZeroRun, successor: Option<Rect>) {
    match run.predecessor {
        Some(FoldAnchor::Bar(index)) => {
            bars[index].kind = StackBarKind::Summary {
                count: run.count + 1,
            };
        }
        Some(FoldAnchor::Active(rect)) => {
            bars.push(StackBar {
                rect: active_edge_row(rect, false),
                kind: StackBarKind::Summary { count: run.count },
            });
        }
        None => {
            if let Some(active_rect) = successor {
                bars.push(StackBar {
                    rect: active_edge_row(active_rect, true),
                    kind: StackBarKind::Summary { count: run.count },
                });
            }
            // No predecessor and no successor is geometrically unreachable —
            // `stack_rects` always keeps the active member present — but
            // skip rather than draw an invalid rect if that ever changes.
        }
    }
}

fn active_edge_row(rect: Rect, top: bool) -> Rect {
    let y = if top {
        rect.y
    } else {
        rect.y.saturating_add(rect.height).saturating_sub(1)
    };
    Rect::new(rect.x, y, rect.width, 1)
}

fn render_stack_bar(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    frame: &mut Frame,
    bar: &StackBar,
) {
    let label = match bar.kind {
        StackBarKind::Pane(pane_id) => pane_label(app, ws, pane_id),
        StackBarKind::Summary { count } => format!("+{count} more"),
    };
    let text = pane_border_title(&label, bar.rect.width, false).unwrap_or_default();
    let style = Style::default()
        .fg(app.palette.subtext0)
        .bg(app.palette.surface0)
        .add_modifier(Modifier::BOLD);
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

pub(crate) fn popup_pane_rects(app: &AppState, area: Rect) -> Option<(Rect, Rect)> {
    let popup = app.popup_pane.as_ref()?;
    resolve_popup_geometry(popup.width, popup.height, area)
        .map(|geometry| (geometry.outer, geometry.inner))
}

pub(super) fn resize_popup_pane(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    let Some(popup) = app.popup_pane.as_ref() else {
        return;
    };
    let Some((_outer, inner)) = popup_pane_rects(app, area) else {
        return;
    };
    if app.direct_attach_resize_locks.contains(&popup.terminal_id) {
        return;
    }
    if let Some(rt) = terminal_runtimes.get(&popup.terminal_id) {
        rt.resize(
            inner.height,
            inner.width,
            cell_size.width_px,
            cell_size.height_px,
        );
    }
}

pub(super) fn render_popup_pane(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let Some(popup) = app.popup_pane.as_ref() else {
        return;
    };
    let Some((outer, inner)) = popup_pane_rects(app, area) else {
        return;
    };
    let Some(rt) = terminal_runtimes.get(&popup.terminal_id) else {
        return;
    };
    let title = app
        .terminals
        .get(&popup.terminal_id)
        .and_then(|terminal| terminal.manual_label.as_deref())
        .unwrap_or("popup");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.palette.accent))
        .title(pane_border_title(title, outer.width, true).unwrap_or_default())
        .style(Style::default().bg(app.palette.panel_bg));
    frame.render_widget(Clear, outer);
    frame.render_widget(block, outer);
    rt.render(frame, inner, !pane_is_scrolled_back(rt));
}

#[derive(Clone, Copy, Default)]
struct LineCell {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

/// The bounding box of the floating layer — every float's own rect, expanded
/// or collapsed, plus any externally supplied `stack_bars`. Tiled-pane border
/// decorations must avoid drawing into this area so they don't punch through
/// the floats, which are drawn earlier in `render_panes`. Folded members have
/// a zero-height `PaneInfo` of their own, so a `+N more` summary row's rect
/// (borrowed from a neighbour, see `close_fold_run`) always lands inside the
/// union already.
fn float_rect(
    ws: &crate::workspace::Workspace,
    pane_infos: &[PaneInfo],
    stack_bars: &[StackBar],
) -> Option<Rect> {
    pane_infos
        .iter()
        .filter(|info| ws.is_float(info.id))
        .map(|info| info.rect)
        .chain(stack_bars.iter().map(|bar| bar.rect))
        .reduce(union_rect)
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let bottom =
        a.y.saturating_add(a.height)
            .max(b.y.saturating_add(b.height));
    Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
}

fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn render_pane_borders(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    pane_infos: &[PaneInfo],
    split_borders: &[crate::layout::SplitBorder],
    stack_bars: &[StackBar],
    frame: &mut Frame,
) {
    if !app.pane_borders || pane_infos.iter().all(|info| info.borders.is_empty()) {
        return;
    }

    let tiled_bars = tiled_stack_bars(ws, pane_infos);

    let mut cells = std::collections::HashMap::<(u16, u16), LineCell>::new();
    for info in pane_infos {
        // The float draws its own Block border; feeding it into the line-join
        // merge would corrupt the tiled joins underneath it. A collapsed or
        // folded stack member draws as a bar instead, with its own left/right
        // border chars — same reasoning applies.
        if ws.is_float(info.id) || info.rect.height <= 1 {
            continue;
        }
        add_pane_border_cells(&mut cells, info);
    }
    add_split_border_cells(app.pane_gaps, split_borders, &mut cells);
    let float_rect = float_rect(ws, pane_infos, stack_bars);

    let buf = frame.buffer_mut();
    let area = buf.area;
    for ((x, y), line) in cells {
        if x < area.x
            || x >= area.x.saturating_add(area.width)
            || y < area.y
            || y >= area.y.saturating_add(area.height)
        {
            continue;
        }
        if float_rect.is_some_and(|rect| rect_contains(rect, x, y)) {
            continue;
        }
        // Stack bars can sit anywhere in the tab, so each is checked on its
        // own rect rather than unioned like the float — a union could wrongly
        // swallow real dividers between unrelated stacks elsewhere on screen.
        if tiled_bars.iter().any(|bar| rect_contains(bar.rect, x, y)) {
            continue;
        }
        let focused = pane_infos.iter().any(|info| {
            !ws.is_float(info.id) && info.is_focused && line_touches_pane(x, y, info, app.pane_gaps)
        });
        let symbol = line_cell_symbol(line);
        if symbol.is_empty() {
            continue;
        }
        let cell = &mut buf[(x, y)];
        cell.set_symbol(symbol);
        let color = if focused {
            app.palette.accent
        } else {
            app.palette.overlay0
        };
        cell.set_style(Style::default().fg(color));
    }

    render_pane_border_titles(app, ws, pane_infos, stack_bars, &tiled_bars, frame);
}

fn add_split_border_cells(
    pane_gaps: bool,
    split_borders: &[crate::layout::SplitBorder],
    cells: &mut std::collections::HashMap<(u16, u16), LineCell>,
) {
    if pane_gaps {
        return;
    }

    for split in split_borders {
        match split.direction {
            ratatui::layout::Direction::Horizontal => {
                let x = split.pos;
                let end = split.area.y.saturating_add(split.area.height);
                for y in split.area.y..=end {
                    if !cells.contains_key(&(x, y)) {
                        continue;
                    }
                    let left = x
                        .checked_sub(1)
                        .and_then(|left_x| cells.get(&(left_x, y)))
                        .is_some_and(|cell| cell.left || cell.right);
                    let right = cells
                        .get(&(x.saturating_add(1), y))
                        .is_some_and(|cell| cell.left || cell.right);
                    let cell = cells.entry((x, y)).or_default();
                    cell.up |= y > split.area.y;
                    cell.down |= y + 1 < end;
                    cell.left |= left;
                    cell.right |= right;
                }
            }
            ratatui::layout::Direction::Vertical => {
                let y = split.pos;
                let end = split.area.x.saturating_add(split.area.width);
                for x in split.area.x..=end {
                    if !cells.contains_key(&(x, y)) {
                        continue;
                    }
                    let up = y
                        .checked_sub(1)
                        .and_then(|up_y| cells.get(&(x, up_y)))
                        .is_some_and(|cell| cell.up || cell.down);
                    let down = cells
                        .get(&(x, y.saturating_add(1)))
                        .is_some_and(|cell| cell.up || cell.down);
                    let cell = cells.entry((x, y)).or_default();
                    cell.left |= x > split.area.x;
                    cell.right |= x + 1 < end;
                    cell.up |= up;
                    cell.down |= down;
                }
            }
        }
    }
}

fn add_pane_border_cells(
    cells: &mut std::collections::HashMap<(u16, u16), LineCell>,
    info: &PaneInfo,
) {
    let rect = info.rect;
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let right = rect.x.saturating_add(rect.width).saturating_sub(1);
    let bottom = rect.y.saturating_add(rect.height).saturating_sub(1);

    if info.borders.contains(Borders::TOP) {
        for x in rect.x..=right {
            let cell = cells.entry((x, rect.y)).or_default();
            cell.left |= x > rect.x;
            cell.right |= x < right;
        }
    }
    if info.borders.contains(Borders::BOTTOM) {
        for x in rect.x..=right {
            let cell = cells.entry((x, bottom)).or_default();
            cell.left |= x > rect.x;
            cell.right |= x < right;
        }
    }
    if info.borders.contains(Borders::LEFT) {
        for y in rect.y..=bottom {
            let cell = cells.entry((rect.x, y)).or_default();
            cell.up |= y > rect.y;
            cell.down |= y < bottom;
        }
    }
    if info.borders.contains(Borders::RIGHT) {
        for y in rect.y..=bottom {
            let cell = cells.entry((right, y)).or_default();
            cell.up |= y > rect.y;
            cell.down |= y < bottom;
        }
    }
}

fn line_touches_pane(x: u16, y: u16, info: &PaneInfo, pane_gaps: bool) -> bool {
    let rect = info.rect;
    if rect.width == 0 || rect.height == 0 {
        return false;
    }
    let right = rect.x.saturating_add(rect.width).saturating_sub(1);
    let bottom = rect.y.saturating_add(rect.height).saturating_sub(1);
    let in_rows = y >= rect.y && y <= bottom;
    let in_cols = x >= rect.x && x <= right;
    let own_border =
        (in_rows && (x == rect.x || x == right)) || (in_cols && (y == rect.y || y == bottom));

    if pane_gaps {
        return own_border;
    }

    let shared_right = rect.x.saturating_add(rect.width);
    let shared_bottom = rect.y.saturating_add(rect.height);
    own_border
        || (in_rows && x == shared_right)
        || (in_cols && y == shared_bottom)
        || (x == shared_right && y == shared_bottom)
}

fn render_pane_border_titles(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    pane_infos: &[PaneInfo],
    stack_bars: &[StackBar],
    tiled_bars: &[StackBar],
    frame: &mut Frame,
) {
    let buf = frame.buffer_mut();
    let area = buf.area;
    let float_rect = float_rect(ws, pane_infos, stack_bars);
    for info in pane_infos {
        // A collapsed or folded stack member draws its label via
        // `render_stack_bar` instead, styled for a 1-row bar rather than a
        // pane's top border.
        if !info.borders.contains(Borders::TOP) || info.rect.width <= 4 || info.rect.height <= 1 {
            continue;
        }
        let Some(title) = ws
            .pane_state(info.id)
            .and_then(|pane| app.terminals.get(&pane.attached_terminal_id))
            .and_then(|terminal| terminal.pane_label(app.show_agent_labels_on_pane_borders))
            .and_then(|label| pane_border_title(&label, info.rect.width, info.is_focused))
        else {
            continue;
        };
        let y = info.rect.y;
        if y < area.y || y >= area.y.saturating_add(area.height) {
            continue;
        }
        let start_x = info.rect.x.saturating_add(1);
        let end_x = info
            .rect
            .x
            .saturating_add(info.rect.width)
            .saturating_sub(1)
            .min(area.x.saturating_add(area.width));
        if start_x >= end_x {
            continue;
        }
        if !ws.is_float(info.id)
            && float_rect.is_some_and(|rect| {
                y >= rect.y
                    && y < rect.y.saturating_add(rect.height)
                    && start_x < rect.x.saturating_add(rect.width)
                    && end_x > rect.x
            })
        {
            continue;
        }
        // A fold with no collapsed bar to repurpose borrows the active
        // member's own top row instead (see `close_fold_run`) — that row's
        // title must give way to the fold's own label.
        if tiled_bars.iter().any(|bar| {
            y >= bar.rect.y
                && y < bar.rect.y.saturating_add(bar.rect.height)
                && start_x < bar.rect.x.saturating_add(bar.rect.width)
                && end_x > bar.rect.x
        }) {
            continue;
        }
        let color = if info.is_focused {
            app.palette.accent
        } else {
            app.palette.overlay0
        };
        let mut style = Style::default().fg(color);
        if info.is_focused {
            style = style.add_modifier(Modifier::BOLD);
        }
        buf.set_stringn(
            start_x,
            y,
            title,
            end_x.saturating_sub(start_x) as usize,
            style,
        );
    }
}

fn line_cell_symbol(line: LineCell) -> &'static str {
    match (line.up, line.down, line.left, line.right) {
        (true, true, true, true) => "┼",
        (true, true, true, false) => "┤",
        (true, true, false, true) => "├",
        (true, false, true, true) => "┴",
        (false, true, true, true) => "┬",
        (true, true, false, false) | (true, false, false, false) | (false, true, false, false) => {
            "│"
        }
        (false, false, true, true) | (false, false, true, false) | (false, false, false, true) => {
            "─"
        }
        (false, true, false, true) => "┌",
        (false, true, true, false) => "┐",
        (true, false, false, true) => "└",
        (true, false, true, false) => "┘",
        _ => "",
    }
}

fn render_copy_mode_cursor(app: &AppState, frame: &mut Frame, info: &PaneInfo) {
    if app.mode != Mode::Copy {
        return;
    }
    let Some(copy_mode) = app.copy_mode.as_ref() else {
        return;
    };
    if copy_mode.pane_id != info.id
        || copy_mode.cursor_row >= info.inner_rect.height
        || copy_mode.cursor_col >= info.inner_rect.width
    {
        return;
    }

    let x = info.inner_rect.x + copy_mode.cursor_col;
    let y = info.inner_rect.y + copy_mode.cursor_row;
    let cell = &mut frame.buffer_mut()[(x, y)];
    cell.set_style(
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
}

fn validated_copy_mode_search_matches(
    app: &AppState,
    info: &PaneInfo,
    rt: &crate::terminal::TerminalRuntime,
) -> (u32, u32, Vec<(usize, crate::pane::TerminalTextMatch)>) {
    let Some(copy_mode) = app.copy_mode.as_ref() else {
        return (0, 0, Vec::new());
    };
    if copy_mode.pane_id != info.id {
        return (0, 0, Vec::new());
    }
    let Some(metrics) = rt.scroll_metrics() else {
        return (0, 0, Vec::new());
    };
    let top = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom)
        .min(u32::MAX as usize) as u32;
    let bottom = top.saturating_add(u32::from(info.inner_rect.height.saturating_sub(1)));
    let first_visible = copy_mode
        .search
        .matches
        .partition_point(|text_match| text_match.end.row < top);
    let visible = &copy_mode.search.matches[first_visible..];
    let visible_len = visible.partition_point(|text_match| text_match.start.row <= bottom);
    let candidates = visible[..visible_len].to_vec();
    let validity = rt.text_matches_are_current(&candidates);

    let matches = candidates
        .into_iter()
        .zip(validity)
        .enumerate()
        .filter_map(|(offset, (text_match, is_current))| {
            is_current.then_some((first_visible + offset, text_match))
        })
        .collect();
    (top, bottom, matches)
}

fn render_copy_mode_search_highlights(
    app: &AppState,
    frame: &mut Frame,
    info: &PaneInfo,
    top: u32,
    bottom: u32,
    matches: &[(usize, crate::pane::TerminalTextMatch)],
    current_only: bool,
) {
    let Some(copy_mode) = app.copy_mode.as_ref() else {
        return;
    };
    let current = copy_mode.search.current;
    let style = if current_only {
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(app.palette.text)
            .bg(app.palette.surface1)
    };

    for &(index, text_match) in matches {
        if (current == Some(index)) != current_only {
            continue;
        }
        let start_row = text_match.start.row.max(top);
        let end_row = text_match.end.row.min(bottom);
        for absolute_row in start_row..=end_row {
            let viewport_row = absolute_row.saturating_sub(top) as u16;
            let start_col = if absolute_row == text_match.start.row {
                text_match.start.col
            } else {
                0
            };
            let end_col = if absolute_row == text_match.end.row {
                text_match.end.col
            } else {
                info.inner_rect.width.saturating_sub(1)
            };
            for col in start_col..=end_col.min(info.inner_rect.width.saturating_sub(1)) {
                let x = info.inner_rect.x.saturating_add(col);
                let y = info.inner_rect.y.saturating_add(viewport_row);
                frame.buffer_mut()[(x, y)].set_style(style);
            }
        }
    }
}

fn render_selection_highlight(
    selection: &Option<crate::selection::Selection>,
    frame: &mut Frame,
    pane_id: crate::layout::PaneId,
    inner: Rect,
    scroll_metrics: Option<crate::pane::ScrollMetrics>,
    p: &Palette,
    host_theme: crate::terminal_theme::TerminalTheme,
) {
    if let Some(sel) = selection {
        if sel.is_visible() && sel.pane_id == pane_id {
            let buf = frame.buffer_mut();
            let style = automatic_selection_style(p, host_theme);
            for y in 0..inner.height {
                for x in 0..inner.width {
                    if sel.contains(y, x, scroll_metrics) {
                        let cell = &mut buf[(inner.x + x, inner.y + y)];
                        cell.set_style(style);
                    }
                }
            }
        }
    }
}

type Rgb = (u8, u8, u8);

fn automatic_selection_style(
    p: &Palette,
    host_theme: crate::terminal_theme::TerminalTheme,
) -> Style {
    let bg = automatic_selection_bg(p, host_theme);
    Style::reset().fg(selection_fg_for_bg(bg, p)).bg(bg)
}

fn automatic_selection_bg(p: &Palette, host_theme: crate::terminal_theme::TerminalTheme) -> Color {
    let Some(background) = host_theme.background.map(terminal_theme_to_rgb) else {
        return selection_palette_background(p);
    };

    let target = if relative_luminance(background) < 0.5 {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };
    let selected = mix_rgb(background, target, 0.28);
    Color::Rgb(selected.0, selected.1, selected.2)
}

fn selection_palette_background(p: &Palette) -> Color {
    if p.panel_bg == Color::Reset {
        p.surface_dim
    } else {
        p.panel_bg
    }
}

fn terminal_theme_to_rgb(color: crate::terminal_theme::RgbColor) -> Rgb {
    (color.r, color.g, color.b)
}

fn selection_fg_for_bg(bg: Color, p: &Palette) -> Color {
    color_to_rgb(bg)
        .map(|bg| {
            if relative_luminance(bg) < 0.5 {
                Color::White
            } else {
                Color::Black
            }
        })
        .unwrap_or_else(|| panel_contrast_fg(p))
}

fn mix_rgb(base: Rgb, target: Rgb, amount: f32) -> Rgb {
    fn channel(base: u8, target: u8, amount: f32) -> u8 {
        (f32::from(base) + (f32::from(target) - f32::from(base)) * amount).round() as u8
    }
    (
        channel(base.0, target.0, amount),
        channel(base.1, target.1, amount),
        channel(base.2, target.2, amount),
    )
}

fn relative_luminance(color: Rgb) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
}

fn color_to_rgb(color: Color) -> Option<Rgb> {
    match color {
        Color::Reset => None,
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((128, 0, 0)),
        Color::Green => Some((0, 128, 0)),
        Color::Yellow => Some((128, 128, 0)),
        Color::Blue => Some((0, 0, 128)),
        Color::Magenta => Some((128, 0, 128)),
        Color::Cyan => Some((0, 128, 128)),
        Color::Gray => Some((192, 192, 192)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((0, 0, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(_) => None,
    }
}

pub(super) fn render_empty(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;
    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "  No workspaces yet",
            Style::default().fg(p.overlay0),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  A workspace is one project context.",
            Style::default().fg(p.overlay1),
        )),
        Line::from(Span::styled(
            "  Its root pane (top-left) sets the default repo or folder name.",
            Style::default().fg(p.overlay1),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Press ", Style::default().fg(p.overlay0)),
            Span::styled(
                app.keybinds
                    .new_workspace
                    .label()
                    .unwrap_or_else(|| "unset".to_string()),
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to create one", Style::default().fg(p.overlay0)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(p.surface_dim)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PaneId;
    use crate::selection::Selection;
    use crate::terminal::TerminalRuntime;
    use crate::terminal::TerminalState;
    use crate::workspace::Workspace;

    fn render_view_pane_borders(app: &AppState, ws: &Workspace, frame: &mut Frame) {
        render_pane_borders(
            app,
            ws,
            &app.view.pane_infos,
            &app.view.split_borders,
            &app.view.stack_bars,
            frame,
        );
    }

    #[test]
    fn pane_border_title_trims_and_truncates() {
        assert_eq!(
            pane_border_title(" claude ", 20, false).as_deref(),
            Some(" claude ")
        );
        assert_eq!(
            pane_border_title(" claude ", 20, true).as_deref(),
            Some(" claude ")
        );
        assert_eq!(pane_border_title("", 20, false), None);
        assert_eq!(
            pane_border_title("abcdef", 8, false).as_deref(),
            Some(" abc… ")
        );
        assert_eq!(
            pane_border_title("abcdef", 8, true).as_deref(),
            Some(" abc… ")
        );
        assert_eq!(pane_border_title("abcdef", 4, false), None);
    }

    #[test]
    fn pane_border_title_truncates_cjk_by_display_width() {
        let title = pane_border_title("1 模块组织（已定）", 12, false).unwrap();

        assert_eq!(title, " 1 模块… ");
        assert!(display_width(title.as_str()) <= 10);
    }

    #[test]
    fn pane_border_renderer_places_adjacent_cjk_by_display_width() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 12, 3);
        let ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        app.view.pane_infos = vec![PaneInfo {
            id: pane_id,
            rect: Rect::new(0, 0, 12, 3),
            inner_rect: Rect::default(),
            scrollbar_rect: None,
            borders: Borders::ALL,
            is_focused: false,
        }];

        let terminal_id = ws.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        let mut terminal_state = TerminalState::new(terminal_id.clone(), "/tmp".into());
        terminal_state.set_manual_label("1 模块组织（已定）".into());
        app.terminals.insert(terminal_id, terminal_state);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(12, 3)).unwrap();
        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(4, 0)].symbol(), "模");
        assert_eq!(buffer[(5, 0)].symbol(), " ");
        assert_eq!(buffer[(6, 0)].symbol(), "块");
    }

    #[test]
    fn default_horizontal_split_uses_one_shared_divider_column() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            true,
            false,
            true,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.rect.x + left.rect.width, right.rect.x);
        assert!(!left.borders.contains(Borders::RIGHT));
        assert!(right.borders.contains(Borders::LEFT));
    }

    #[test]
    fn default_vertical_split_uses_one_shared_divider_row() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let bottom = workspace.test_split(ratatui::layout::Direction::Vertical);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            true,
            false,
            true,
        );
        let top = infos.iter().find(|info| info.id == root).unwrap();
        let bottom = infos.iter().find(|info| info.id == bottom).unwrap();

        assert_eq!(top.rect.y + top.rect.height, bottom.rect.y);
        assert!(!top.borders.contains(Borders::BOTTOM));
        assert!(bottom.borders.contains(Borders::TOP));
    }

    #[test]
    fn disabled_outer_borders_keep_only_shared_pane_dividers() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            true,
            false,
            false,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.borders, Borders::NONE);
        assert_eq!(right.borders, Borders::LEFT);
    }

    #[test]
    fn pane_gaps_keep_independent_bordered_panes() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            true,
            true,
            true,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.rect.x + left.rect.width, right.rect.x);
        assert_eq!(left.borders, Borders::ALL);
        assert_eq!(right.borders, Borders::ALL);
    }

    #[test]
    fn borderless_pane_gaps_add_one_empty_cell_between_panes() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            false,
            true,
            true,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.rect, Rect::new(0, 0, 49, 20));
        assert_eq!(right.rect, Rect::new(50, 0, 50, 20));
        assert!(left.borders.is_empty());
        assert!(right.borders.is_empty());
    }

    #[test]
    fn disabled_pane_borders_make_inner_rect_equal_visual_rect() {
        let mut workspace = Workspace::test_new("test");
        workspace.test_split(ratatui::layout::Direction::Horizontal);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            false,
            false,
            true,
        );

        for info in infos {
            assert!(info.borders.is_empty());
            assert_eq!(pane_inner_rect(info.rect, info.borders), info.rect);
        }
    }

    #[test]
    fn global_pane_border_renderer_composes_junctions_and_focus_style() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 4, 4);
        app.view.pane_infos = vec![
            PaneInfo {
                id: PaneId::from_raw(1),
                rect: Rect::new(0, 0, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT,
                is_focused: true,
            },
            PaneInfo {
                id: PaneId::from_raw(2),
                rect: Rect::new(2, 0, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::RIGHT,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(3),
                rect: Rect::new(0, 2, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::BOTTOM,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(4),
                rect: Rect::new(2, 2, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
        ];
        app.view.split_borders = vec![
            crate::layout::SplitBorder {
                pos: 2,
                direction: ratatui::layout::Direction::Horizontal,
                ratio: 0.5,
                area: Rect::new(0, 0, 4, 4),
                path: vec![],
            },
            crate::layout::SplitBorder {
                pos: 2,
                direction: ratatui::layout::Direction::Vertical,
                ratio: 0.5,
                area: Rect::new(0, 0, 4, 4),
                path: vec![false],
            },
        ];
        let ws = Workspace::test_new("test");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(4, 4)).unwrap();

        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 2)].symbol(), "┼");
        assert_eq!(buffer[(2, 2)].style().fg, Some(app.palette.accent));
        assert_eq!(buffer[(2, 1)].symbol(), "│");
        assert_eq!(buffer[(2, 1)].style().fg, Some(app.palette.accent));
    }

    #[test]
    fn tiled_split_borders_do_not_draw_over_a_float() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 4, 4);
        let float_id = PaneId::from_raw(99);
        app.view.pane_infos = vec![
            PaneInfo {
                id: PaneId::from_raw(1),
                rect: Rect::new(0, 0, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(2),
                rect: Rect::new(2, 0, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::RIGHT,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(3),
                rect: Rect::new(0, 2, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::BOTTOM,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(4),
                rect: Rect::new(2, 2, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
            // The float covers the cross-junction the tiled split would
            // otherwise draw at (2, 2) / (2, 1).
            PaneInfo {
                id: float_id,
                rect: Rect::new(1, 1, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
        ];
        app.view.split_borders = vec![
            crate::layout::SplitBorder {
                pos: 2,
                direction: ratatui::layout::Direction::Horizontal,
                ratio: 0.5,
                area: Rect::new(0, 0, 4, 4),
                path: vec![],
            },
            crate::layout::SplitBorder {
                pos: 2,
                direction: ratatui::layout::Direction::Vertical,
                ratio: 0.5,
                area: Rect::new(0, 0, 4, 4),
                path: vec![false],
            },
        ];
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(4, 4)).unwrap();

        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 2)].symbol(), " ", "junction hidden under float");
        assert_eq!(buffer[(2, 1)].symbol(), " ", "divider hidden under float");
        // Outside the float's rect, the tiled join still draws normally.
        assert_eq!(buffer[(2, 0)].symbol(), "┬");
    }

    #[test]
    fn float_rect_unions_the_popup_with_every_stack_bar_including_the_summary_row() {
        let hidden_id = PaneId::from_raw(1);
        let float_id = PaneId::from_raw(2);
        let pane_infos = vec![PaneInfo {
            id: float_id,
            rect: Rect::new(5, 4, 20, 6),
            inner_rect: Rect::default(),
            scrollbar_rect: None,
            borders: Borders::ALL,
            is_focused: true,
        }];
        let stack_bars = vec![
            crate::popup_size::StackBar {
                rect: Rect::new(5, 2, 20, 1),
                kind: crate::popup_size::StackBarKind::Summary { count: 2 },
            },
            crate::popup_size::StackBar {
                rect: Rect::new(5, 3, 20, 1),
                kind: crate::popup_size::StackBarKind::Pane(hidden_id),
            },
        ];
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].push_float(
            hidden_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );

        assert_eq!(
            float_rect(&ws, &pane_infos, &stack_bars),
            Some(Rect::new(5, 2, 20, 8))
        );
    }

    #[test]
    fn gapped_pane_focus_does_not_color_neighbor_border() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.pane_gaps = true;
        app.view.terminal_area = Rect::new(0, 0, 4, 3);
        app.view.pane_infos = vec![
            PaneInfo {
                id: PaneId::from_raw(1),
                rect: Rect::new(0, 0, 2, 3),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
            PaneInfo {
                id: PaneId::from_raw(2),
                rect: Rect::new(2, 0, 2, 3),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
        ];
        let ws = Workspace::test_new("test");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(4, 3)).unwrap();

        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 1)].style().fg, Some(app.palette.accent));
        assert_eq!(buffer[(2, 1)].style().fg, Some(app.palette.overlay0));
    }

    #[tokio::test]
    async fn pane_scrollbar_gutter_is_reserved_before_scrollback_exists() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        // One row goes to the lone pane's title strip.
        assert_eq!(info.inner_rect, Rect::new(10, 4, 39, 7));
    }

    #[tokio::test]
    async fn zoomed_pane_scrollbar_gutter_is_reserved_before_scrollback_exists() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        workspace.zoomed = true;
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        // One row goes to the lone pane's title strip.
        assert_eq!(info.inner_rect, Rect::new(10, 4, 39, 7));
    }

    #[tokio::test]
    async fn zoomed_multi_pane_keeps_border_space() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let focused_pane = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.zoomed = true;
        workspace.tabs[0].runtimes.insert(
            focused_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.id, focused_pane);
        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(11, 4, 37, 6));
    }

    #[tokio::test]
    async fn tiny_pane_does_not_reserve_scrollbar_gutter() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(4, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 4, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, pane_inner_rect(area, LONE_PANE_BORDERS));
    }

    #[tokio::test]
    async fn pane_scrollbar_setting_controls_reserved_column() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(
                40,
                8,
                1024,
                b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
            ),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, Some(Rect::new(49, 4, 1, 7)));
        assert_eq!(info.inner_rect, Rect::new(10, 4, 39, 7));

        app.pane_scrollbars = false;
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, pane_inner_rect(area, LONE_PANE_BORDERS));
    }

    #[tokio::test]
    async fn a_lone_pane_keeps_a_title_strip_naming_its_agent_task() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        let area = Rect::new(0, 0, 40, 12);
        app.view.terminal_area = area;

        let mut ws = Workspace::test_new("test");
        let root_pane = ws.tabs[0].root_pane;
        ws.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 11, 1024, b""),
        );
        let terminal_id = ws.terminal_id(root_pane).cloned().expect("terminal id");
        let mut terminal_state = TerminalState::new(terminal_id.clone(), "/home/user/herdr".into());
        terminal_state.set_detected_state(
            Some(crate::detect::Agent::Claude),
            crate::detect::AgentState::Working,
        );
        terminal_state.set_terminal_title(Some("✳ Wiring the title strip".into()));
        app.terminals.insert(terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &pane_infos[0];
        assert_eq!(info.borders, LONE_PANE_BORDERS);
        assert_eq!(info.inner_rect.y, area.y + 1);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| render_panes(&app, &terminal_runtimes, frame, &pane_infos, &[], &[]))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (area.x..area.x + area.width)
            .map(|x| buffer[(x, area.y)].symbol())
            .collect();
        assert!(row.contains("Wiring the title strip"), "top row: {row:?}");
    }

    #[tokio::test]
    async fn render_panes_falls_back_to_cwd_basename_for_an_unlabeled_float() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 40, 12);

        let mut ws = Workspace::test_new("test");
        let float_id = PaneId::from_raw(60);
        let float_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(float_terminal_id.clone()),
        );
        ws.tabs[0].runtimes.insert(
            float_id,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b""),
        );

        let terminal_state =
            TerminalState::new(float_terminal_id.clone(), "/home/user/zellij".into());
        app.terminals.insert(float_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let area = app.view.terminal_area;
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let float_rect = pane_infos
            .iter()
            .find(|info| info.id == float_id)
            .expect("float pane info")
            .rect;

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| render_panes(&app, &terminal_runtimes, frame, &pane_infos, &[], &[]))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (float_rect.x..float_rect.x + float_rect.width)
            .map(|x| buffer[(x, float_rect.y)].symbol())
            .collect();
        assert!(row.contains("zellij"), "float border row: {row:?}");
    }

    #[tokio::test]
    async fn render_panes_prefers_foreground_process_name_over_cwd_basename_for_an_unlabeled_float()
    {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 40, 12);

        let mut ws = Workspace::test_new("test");
        let float_id = PaneId::from_raw(61);
        let float_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(float_terminal_id.clone()),
        );
        ws.tabs[0].runtimes.insert(
            float_id,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b""),
        );

        let mut terminal_state =
            TerminalState::new(float_terminal_id.clone(), "/home/user/herdr".into());
        terminal_state.foreground_process_name = Some("nvim".to_string());
        app.terminals.insert(float_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let area = app.view.terminal_area;
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let float_rect = pane_infos
            .iter()
            .find(|info| info.id == float_id)
            .expect("float pane info")
            .rect;

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| render_panes(&app, &terminal_runtimes, frame, &pane_infos, &[], &[]))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (float_rect.x..float_rect.x + float_rect.width)
            .map(|x| buffer[(x, float_rect.y)].symbol())
            .collect();
        assert!(row.contains("nvim"), "float border row: {row:?}");
        assert!(!row.contains("herdr"), "float border row: {row:?}");
    }

    #[tokio::test]
    async fn render_panes_prefers_osc_title_over_cwd_basename_for_an_unlabeled_float() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 40, 12);

        let mut ws = Workspace::test_new("test");
        let float_id = PaneId::from_raw(62);
        let float_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(float_terminal_id.clone()),
        );
        ws.tabs[0].runtimes.insert(
            float_id,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b""),
        );

        let mut terminal_state =
            TerminalState::new(float_terminal_id.clone(), "/home/user/herdr".into());
        terminal_state.set_terminal_title(Some("ssh remote-host".to_string()));
        app.terminals.insert(float_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let area = app.view.terminal_area;
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let float_rect = pane_infos
            .iter()
            .find(|info| info.id == float_id)
            .expect("float pane info")
            .rect;

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| render_panes(&app, &terminal_runtimes, frame, &pane_infos, &[], &[]))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (float_rect.x..float_rect.x + float_rect.width)
            .map(|x| buffer[(x, float_rect.y)].symbol())
            .collect();
        assert!(row.contains("ssh remote-host"), "float border row: {row:?}");
        assert!(!row.contains("herdr"), "float border row: {row:?}");
    }

    #[tokio::test]
    async fn render_panes_prefers_foreground_process_name_over_a_shell_set_osc_title() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 40, 12);

        let mut ws = Workspace::test_new("test");
        let float_id = PaneId::from_raw(63);
        let float_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(float_terminal_id.clone()),
        );
        ws.tabs[0].runtimes.insert(
            float_id,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b""),
        );

        // A shell prompt commonly sets the OSC title on every command (to the
        // command it just ran, here an alias) even while something else is
        // the actual foreground process — the process name must still win.
        let mut terminal_state =
            TerminalState::new(float_terminal_id.clone(), "/home/user/herdr".into());
        terminal_state.set_terminal_title(Some("lg ~/herdr".to_string()));
        terminal_state.foreground_process_name = Some("lazygit".to_string());
        app.terminals.insert(float_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let area = app.view.terminal_area;
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let float_rect = pane_infos
            .iter()
            .find(|info| info.id == float_id)
            .expect("float pane info")
            .rect;

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| render_panes(&app, &terminal_runtimes, frame, &pane_infos, &[], &[]))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (float_rect.x..float_rect.x + float_rect.width)
            .map(|x| buffer[(x, float_rect.y)].symbol())
            .collect();
        assert!(row.contains("lazygit"), "float border row: {row:?}");
        assert!(!row.contains("lg "), "float border row: {row:?}");
    }

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

    #[test]
    fn tiled_split_borders_do_not_draw_over_stack_bars_or_the_summary_row() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 20, 12);

        let left_id = PaneId::from_raw(60);
        let right_id = PaneId::from_raw(61);
        let hidden_id = PaneId::from_raw(62);
        let top_id = PaneId::from_raw(63);

        // A 50/50 vertical divider running down column 10, straight through
        // the columns the centred popup and its stack bars occupy.
        let pane_infos = vec![
            PaneInfo {
                id: left_id,
                rect: Rect::new(0, 0, 10, 12),
                inner_rect: Rect::new(1, 1, 8, 10),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::BOTTOM,
                is_focused: false,
            },
            PaneInfo {
                id: right_id,
                rect: Rect::new(10, 0, 10, 12),
                inner_rect: Rect::new(11, 1, 8, 10),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
            PaneInfo {
                id: top_id,
                rect: Rect::new(2, 4, 16, 6),
                inner_rect: Rect::new(3, 5, 14, 4),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
        ];
        let split_borders = vec![crate::layout::SplitBorder {
            pos: 10,
            direction: ratatui::layout::Direction::Horizontal,
            ratio: 0.5,
            area: Rect::new(0, 0, 20, 12),
            path: vec![],
        }];
        let stack_bars = vec![
            crate::popup_size::StackBar {
                rect: Rect::new(2, 2, 16, 1),
                kind: crate::popup_size::StackBarKind::Summary { count: 2 },
            },
            crate::popup_size::StackBar {
                rect: Rect::new(2, 3, 16, 1),
                kind: crate::popup_size::StackBarKind::Pane(hidden_id),
            },
        ];

        let mut ws = Workspace::test_new("test");
        for (offset, id) in [hidden_id, top_id].into_iter().enumerate() {
            ws.tabs[0].push_float(
                id,
                crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
            );
            ws.register_new_pane_with_number(id, 2 + offset);
        }
        app.workspaces = vec![ws];
        app.active = Some(0);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 12)).unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    &pane_infos,
                    &split_borders,
                    &stack_bars,
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // The divider is real: it draws above the stack and below the popup.
        assert_eq!(buffer[(10, 1)].symbol(), "│");
        assert_eq!(buffer[(10, 10)].symbol(), "│");
        // ...but never inside it, on the summary row or on a real bar row.
        for y in 2..=3 {
            assert_ne!(
                buffer[(10, y)].symbol(),
                "│",
                "tiled divider punched through stack row {y}"
            );
        }
        let summary_row: String = (2..18).map(|x| buffer[(x, 2)].symbol()).collect();
        assert!(summary_row.contains("+2 more"), "summary: {summary_row:?}");
        let bar_row: String = (2..18).map(|x| buffer[(x, 3)].symbol()).collect();
        assert!(bar_row.contains("pane 2"), "bar: {bar_row:?}");
    }

    #[test]
    fn render_panes_draws_a_summary_bar_with_the_folded_count() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 30, 10);
        app.workspaces = vec![Workspace::test_new("test")];
        app.active = Some(0);

        let bar = crate::popup_size::StackBar {
            rect: Rect::new(5, 3, 20, 1),
            kind: crate::popup_size::StackBarKind::Summary { count: 3 },
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
        assert!(row.contains("+3 more"), "bar row: {row:?}");
    }

    #[test]
    fn selection_highlight_uses_one_uniform_style() {
        let palette = Palette::catppuccin();
        let host_theme = crate::terminal_theme::TerminalTheme {
            foreground: None,
            background: Some(crate::terminal_theme::RgbColor {
                r: 12,
                g: 14,
                b: 16,
            }),
            ..Default::default()
        };
        let expected_style = automatic_selection_style(&palette, host_theme);
        let selection = Some(Selection::range(PaneId::from_raw(1), 0, 0, 2, None));
        let backend = ratatui::backend::TestBackend::new(4, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let buf = frame.buffer_mut();
                buf[(0, 0)].set_style(
                    Style::default()
                        .fg(Color::Rgb(10, 220, 120))
                        .bg(Color::Black),
                );
                buf[(1, 0)].set_style(
                    Style::default()
                        .fg(Color::Rgb(220, 180, 40))
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
                buf[(2, 0)].set_style(Style::default().fg(Color::Blue).bg(Color::Reset));
                render_selection_highlight(
                    &selection,
                    frame,
                    PaneId::from_raw(1),
                    Rect::new(0, 0, 4, 1),
                    None,
                    &palette,
                    host_theme,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let first = buffer[(0, 0)].style();
        let second = buffer[(1, 0)].style();
        let third = buffer[(2, 0)].style();

        assert_eq!(first.fg, expected_style.fg);
        assert_eq!(second.fg, expected_style.fg);
        assert_eq!(third.fg, expected_style.fg);
        assert_eq!(first.bg, expected_style.bg);
        assert_eq!(second.bg, expected_style.bg);
        assert_eq!(third.bg, expected_style.bg);
        assert_eq!(first.add_modifier, expected_style.add_modifier);
        assert_eq!(second.add_modifier, expected_style.add_modifier);
        assert_eq!(third.add_modifier, expected_style.add_modifier);
        assert!(!second.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn automatic_selection_background_uses_host_background() {
        let bg = automatic_selection_bg(
            &Palette::terminal(),
            crate::terminal_theme::TerminalTheme {
                foreground: Some(crate::terminal_theme::RgbColor {
                    r: 230,
                    g: 230,
                    b: 230,
                }),
                background: Some(crate::terminal_theme::RgbColor {
                    r: 12,
                    g: 14,
                    b: 16,
                }),
                ..Default::default()
            },
        );

        let Color::Rgb(r, g, b) = bg else {
            panic!("selection background should resolve to rgb");
        };
        assert!(relative_luminance((r, g, b)) > relative_luminance((12, 14, 16)));
    }

    // Task 2's invariant: cycling arrangements must preserve pane order and
    // focus. These two are geometry-only regression anchors for that
    // invariant as it applies to Stacked; they exercise Task 5-8 code, not
    // this task's renderer, and are expected to already pass.
    #[test]
    fn collapsed_stack_members_render_as_single_row_bars() {
        let area = Rect::new(0, 0, 80, 10);
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        let first = tab.layout.focused();
        let second = tab
            .layout
            .split_focused(ratatui::layout::Direction::Horizontal);
        tab.arrangement = crate::layout::Arrangement::Stacked;
        tab.needs_reflow = true;
        tab.reflow(area, None);

        let infos = tab.layout.panes(area);
        let bars: Vec<_> = infos.iter().filter(|p| p.rect.height == 1).collect();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].id, first);
        let expanded: Vec<_> = infos.iter().filter(|p| p.rect.height > 1).collect();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].id, second);
    }

    #[test]
    fn zooming_a_stack_member_shows_only_that_pane() {
        let area = Rect::new(0, 0, 80, 10);
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        let second = tab
            .layout
            .split_focused(ratatui::layout::Direction::Horizontal);
        tab.arrangement = crate::layout::Arrangement::Stacked;
        tab.needs_reflow = true;
        tab.reflow(area, None);
        tab.zoomed = true;

        // Zoom already renders only the focused pane, and the focused pane is
        // always the stack's active member, so no stack-specific handling is
        // needed. This test exists to catch a regression if that changes.
        assert_eq!(tab.layout.focused(), second);
        let infos = tab.layout.panes(area);
        let focused = infos.iter().find(|p| p.is_focused).expect("a focused pane");
        assert_eq!(focused.id, second);
    }

    #[tokio::test]
    async fn render_panes_draws_terminal_content_for_the_active_member_and_a_bar_for_the_collapsed_one(
    ) {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 30, 10);

        let active_id = PaneId::from_raw(70);
        let collapsed_id = PaneId::from_raw(71);
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].panes.insert(
            active_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );
        let collapsed_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].panes.insert(
            collapsed_id,
            crate::pane::PaneState::new(collapsed_terminal_id.clone()),
        );
        ws.tabs[0].runtimes.insert(
            active_id,
            TerminalRuntime::test_with_screen_bytes(28, 7, b"ACTIVE"),
        );

        let mut terminal_state = TerminalState::new(collapsed_terminal_id.clone(), "/tmp".into());
        terminal_state.set_manual_label("collapsed".into());
        app.terminals.insert(collapsed_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let pane_infos = vec![
            PaneInfo {
                id: active_id,
                rect: Rect::new(0, 0, 30, 9),
                inner_rect: Rect::new(1, 1, 28, 7),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
            PaneInfo {
                id: collapsed_id,
                rect: Rect::new(0, 9, 30, 1),
                inner_rect: Rect::new(0, 9, 30, 1),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
        ];

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 10)).unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    &pane_infos,
                    &[],
                    &[],
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let collapsed_row: String = (0..30).map(|x| buffer[(x, 9)].symbol()).collect();
        assert!(
            collapsed_row.contains("collapsed"),
            "bar row: {collapsed_row:?}"
        );
        // Distinguishes an actual bar draw from a leftover generic border
        // title: a plain bordered pane on a 1-row rect draws "─" at its left
        // edge from the junction table, not the "│" render_stack_bar uses.
        assert_eq!(buffer[(0, 9)].symbol(), "│", "bar row: {collapsed_row:?}");

        let content: String = (0..9)
            .flat_map(|y| (0..30).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol())
            .collect();
        assert!(
            content.contains("ACTIVE"),
            "active content missing: {content:?}"
        );
    }

    #[test]
    fn every_float_gets_a_rect_inside_the_region() {
        let region = Rect::new(10, 5, 40, 10);
        let mut workspace = Workspace::test_new("floats");
        let tab = workspace.tabs.get_mut(0).expect("a tab");
        let ids: Vec<_> = (0..3).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            tab.push_float(
                *id,
                crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
            );
        }
        tab.float_arrangement = crate::layout::Arrangement::Stacked;
        tab.needs_reflow = true;
        tab.reflow(Rect::new(0, 0, 80, 20), Some(region));

        let infos = tab
            .float_layout
            .as_ref()
            .expect("a float layout")
            .panes(region);
        assert_eq!(
            infos.len(),
            3,
            "every float is laid out, not just the top one"
        );
        for info in &infos {
            assert!(info.rect.y >= region.y);
            assert!(info.rect.y + info.rect.height <= region.y + region.height);
        }
        let expanded: Vec<_> = infos.iter().filter(|i| i.rect.height > 1).collect();
        assert_eq!(
            expanded.len(),
            1,
            "stacked shows exactly one expanded member"
        );
    }

    #[test]
    fn compute_pane_infos_appends_every_float_after_the_tiled_panes_with_its_own_rect() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        let area = Rect::new(0, 0, 60, 20);
        app.view.terminal_area = area;

        let mut ws = Workspace::test_new("test");
        let root_pane = ws.tabs[0].root_pane;
        let float_ids: Vec<_> = (0..3).map(|_| PaneId::alloc()).collect();
        for id in &float_ids {
            ws.tabs[0].push_float(
                *id,
                crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
            );
        }
        ws.tabs[0].float_arrangement = crate::layout::Arrangement::Grid;
        let float_region =
            resolve_popup_geometry(app.floating_pane_width, app.floating_pane_height, area)
                .map(|geometry| geometry.outer);
        ws.tabs[0].needs_reflow = true;
        ws.tabs[0].reflow(area, float_region);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );

        // The tiled root pane comes first; every float comes after it, so a
        // reverse-order hit test finds any float before the tiled pane it covers.
        assert_eq!(pane_infos[0].id, root_pane);
        let float_infos: Vec<_> = pane_infos[1..].iter().collect();
        assert_eq!(float_infos.len(), 3);
        let ids: std::collections::HashSet<_> = float_infos.iter().map(|info| info.id).collect();
        assert_eq!(ids, float_ids.iter().copied().collect());

        // Grid keeps every float expanded at once, each with a real, distinct box.
        for info in &float_infos {
            assert!(
                info.rect.height > 1,
                "float {:?} should be expanded",
                info.id
            );
        }
        let rects: std::collections::HashSet<_> = float_infos
            .iter()
            .map(|info| (info.rect.x, info.rect.y, info.rect.width, info.rect.height))
            .collect();
        assert_eq!(
            rects.len(),
            3,
            "every float gets its own rect, not a shared one"
        );
    }

    #[tokio::test]
    async fn compute_pane_infos_resizes_every_visible_floats_runtime_to_its_own_box() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        let area = Rect::new(0, 0, 60, 20);
        app.view.terminal_area = area;

        let mut ws = Workspace::test_new("test");
        let float_ids: Vec<_> = (0..2).map(|_| PaneId::alloc()).collect();
        for id in &float_ids {
            ws.tabs[0].push_float(
                *id,
                crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
            );
        }
        ws.tabs[0].float_arrangement = crate::layout::Arrangement::Grid;
        let float_region =
            resolve_popup_geometry(app.floating_pane_width, app.floating_pane_height, area)
                .map(|geometry| geometry.outer);
        ws.tabs[0].needs_reflow = true;
        ws.tabs[0].reflow(area, float_region);

        // Seed both runtimes at a row count no real box in this layout could
        // produce, so a per-float resize is the only way their viewport ends
        // up matching the layout.
        for id in &float_ids {
            ws.tabs[0]
                .runtimes
                .insert(*id, TerminalRuntime::test_with_screen_bytes(5, 1, b""));
        }
        app.workspaces = vec![ws];
        app.active = Some(0);

        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let cell_size = crate::kitty_graphics::HostCellSize::default();
        let pane_infos = compute_pane_infos(&app, &terminal_runtimes, area, true, cell_size);

        let float_infos: Vec<_> = pane_infos
            .iter()
            .filter(|info| float_ids.contains(&info.id))
            .collect();
        assert_eq!(
            float_infos.len(),
            2,
            "grid keeps both floats expanded at once"
        );

        for info in float_infos {
            assert!(
                info.rect.height > 1,
                "float {:?} should be expanded",
                info.id
            );
            let rt = app
                .runtime_for_pane_in_workspace(&terminal_runtimes, 0, info.id)
                .expect("runtime");
            let metrics = rt.scroll_metrics().expect("scroll metrics");
            assert_eq!(
                metrics.viewport_rows, info.inner_rect.height as usize,
                "float {:?} was not resized to its own box",
                info.id
            );
        }
    }

    #[tokio::test]
    async fn render_panes_draws_every_floats_content_not_just_one() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 40, 10);

        let left_id = PaneId::from_raw(700);
        let right_id = PaneId::from_raw(701);
        let mut ws = Workspace::test_new("test");
        for id in [left_id, right_id] {
            ws.tabs[0].push_float(
                id,
                crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
            );
        }
        ws.tabs[0].runtimes.insert(
            left_id,
            TerminalRuntime::test_with_screen_bytes(18, 8, b"LEFTFLOAT"),
        );
        ws.tabs[0].runtimes.insert(
            right_id,
            TerminalRuntime::test_with_screen_bytes(18, 8, b"RIGHTFLOAT"),
        );
        app.workspaces = vec![ws];
        app.active = Some(0);

        // Two members that would come from a Stacked layout with only the
        // bar collapsed away — both above height 1, so both must draw.
        let pane_infos = vec![
            PaneInfo {
                id: left_id,
                rect: Rect::new(0, 0, 20, 10),
                inner_rect: Rect::new(1, 1, 18, 8),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
            PaneInfo {
                id: right_id,
                rect: Rect::new(20, 0, 20, 10),
                inner_rect: Rect::new(21, 1, 18, 8),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
        ];

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    &pane_infos,
                    &[],
                    &[],
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = (0..10)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol())
            .collect();
        assert!(
            content.contains("LEFTFLOAT"),
            "left float missing: {content:?}"
        );
        assert!(
            content.contains("RIGHTFLOAT"),
            "right float missing: {content:?}"
        );
    }

    #[tokio::test]
    async fn a_hidden_float_layer_is_neither_laid_out_nor_drawn() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        let area = Rect::new(0, 0, 40, 12);
        app.view.terminal_area = area;

        let float_id = PaneId::from_raw(720);
        let mut ws = Workspace::test_new("test");
        let root_pane = ws.tabs[0].root_pane;
        let float_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            float_id,
            crate::pane::PaneState::new(float_terminal_id.clone()),
        );
        ws.tabs[0].runtimes.insert(
            float_id,
            TerminalRuntime::test_with_screen_bytes(18, 8, b"HIDDENFLOAT"),
        );
        let mut float_terminal = TerminalState::new(float_terminal_id.clone(), "/tmp".into());
        float_terminal.set_manual_label("hiddenlabel".into());
        app.terminals.insert(float_terminal_id, float_terminal);

        // `assert_invariants_for_test` permits a hidden layer that still holds
        // its layout, so this state is legal and must render nothing.
        ws.tabs[0].set_floats_hidden(true);
        let float_region =
            resolve_popup_geometry(app.floating_pane_width, app.floating_pane_height, area)
                .map(|geometry| geometry.outer);
        ws.tabs[0].reflow(area, float_region);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let pane_infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            true,
            crate::kitty_graphics::HostCellSize::default(),
        );
        assert_eq!(
            pane_infos.iter().map(|info| info.id).collect::<Vec<_>>(),
            vec![root_pane],
            "a hidden float layer contributes no pane info"
        );

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| render_panes(&app, &terminal_runtimes, frame, &pane_infos, &[], &[]))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = (0..12)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol())
            .collect();
        assert!(
            !content.contains("HIDDENFLOAT"),
            "hidden float content drawn: {content:?}"
        );
        assert!(
            !content.contains("hiddenlabel"),
            "hidden float title drawn: {content:?}"
        );
        // The lone tiled pane draws no border, so any box corner on screen can
        // only have come from the float's block.
        assert!(
            !content.contains('┌'),
            "hidden float border drawn: {content:?}"
        );
    }

    #[test]
    fn render_panes_draws_a_bar_for_a_collapsed_float_without_an_external_stack_bar() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 30, 10);

        let mut ws = Workspace::test_new("test");
        let collapsed_id = PaneId::from_raw(710);
        let active_id = PaneId::from_raw(711);
        let collapsed_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].push_float(
            collapsed_id,
            crate::pane::PaneState::new(collapsed_terminal_id.clone()),
        );
        ws.tabs[0].push_float(
            active_id,
            crate::pane::PaneState::new(crate::terminal::TerminalId::alloc()),
        );
        let mut terminal_state = TerminalState::new(collapsed_terminal_id.clone(), "/tmp".into());
        terminal_state.set_manual_label("hidden-float".into());
        app.terminals.insert(collapsed_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        // Hand-built as `stack_rects` would lay out a two-member stack: one
        // expanded, one collapsed to a single row. Passing `&[]` for the
        // external `stack_bars` proves the bar is derived from `pane_infos`
        // at render time instead.
        let pane_infos = vec![
            PaneInfo {
                id: collapsed_id,
                rect: Rect::new(5, 3, 20, 1),
                inner_rect: Rect::new(5, 3, 20, 1),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
            PaneInfo {
                id: active_id,
                rect: Rect::new(5, 4, 20, 6),
                inner_rect: Rect::new(6, 5, 18, 4),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
        ];

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 10)).unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    &pane_infos,
                    &[],
                    &[],
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (5..25).map(|x| buffer[(x, 3)].symbol()).collect();
        assert!(row.contains("hidden-float"), "bar row: {row:?}");
    }

    /// Builds a real `count`-member stack, focuses the member that ends up
    /// at `active_index` in `TileLayout::pane_ids()` order, reflows it under
    /// `area`, and returns the resulting `PaneInfo`s straight from
    /// `stack_rects` — geometry a hand-built fixture can't be trusted to
    /// reproduce, since `stack_rects` always consumes the whole area once
    /// any folding happens (a spare row a fixture might leave never exists).
    fn stacked_pane_infos(
        ws: &mut Workspace,
        count: usize,
        active_index: usize,
        area: Rect,
    ) -> Vec<PaneInfo> {
        let tab = &mut ws.tabs[0];
        for _ in 1..count {
            tab.layout
                .split_focused(ratatui::layout::Direction::Horizontal);
        }
        let ids = tab.layout.pane_ids();
        assert_eq!(ids.len(), count);
        tab.layout.focus_pane(ids[active_index]);
        tab.arrangement = crate::layout::Arrangement::Stacked;
        tab.needs_reflow = true;
        tab.reflow(area, None);
        tab.layout.panes(area)
    }

    fn render_stacked(pane_infos: &[PaneInfo], app: &AppState, area: Rect) -> Vec<String> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                .unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    pane_infos,
                    &[],
                    &[],
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn folding_with_no_room_for_any_bar_still_shows_a_truthful_summary() {
        // count=5, active=0, height=3: MIN_ACTIVE_STACK_HEIGHT alone consumes
        // the whole area, so every other member folds to height 0 with no
        // collapsed bar anywhere to repurpose. The single fold must borrow
        // the active member's own row rather than panic or draw nothing.
        let area = Rect::new(0, 0, 20, 3);
        let mut ws = Workspace::test_new("test");
        let pane_infos = stacked_pane_infos(&mut ws, 5, 0, area);
        assert_eq!(
            pane_infos.iter().filter(|p| p.rect.height == 0).count(),
            4,
            "every non-active member should have folded"
        );

        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.workspaces = vec![ws];
        app.active = Some(0);

        let rows = render_stacked(&pane_infos, &app, area);
        assert!(
            rows.iter().any(|row| row.contains("+4 more")),
            "no truthful summary shown: {rows:?}"
        );
    }

    #[test]
    fn folding_around_the_active_member_produces_two_separate_truthful_summaries() {
        // count=20, active=15, height=10: enough real bars exist before the
        // active member to leave a genuine leading fold (repurposes its last
        // collapsed bar, so that pane's own row joins the count) and a
        // trailing fold with no bar left to repurpose (borrows the active
        // member's own row instead, so its count stays unchanged).
        let area = Rect::new(0, 0, 20, 10);
        let mut ws = Workspace::test_new("test");
        let pane_infos = stacked_pane_infos(&mut ws, 20, 15, area);
        assert_eq!(
            pane_infos.iter().filter(|p| p.rect.height == 0).count(),
            12,
            "8 leading + 4 trailing folded members"
        );

        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.workspaces = vec![ws];
        app.active = Some(0);

        let rows = render_stacked(&pane_infos, &app, area);
        assert!(
            rows.iter().any(|row| row.contains("+9 more")),
            "leading fold's repurposed bar missing: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("+4 more")),
            "trailing fold's borrowed active row missing: {rows:?}"
        );
    }

    #[test]
    fn render_pane_borders_does_not_draw_over_a_collapsed_stack_bar() {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.view.terminal_area = Rect::new(0, 0, 20, 10);

        let active_id = PaneId::from_raw(90);
        let collapsed_id = PaneId::from_raw(91);

        let pane_infos = vec![
            PaneInfo {
                id: active_id,
                rect: Rect::new(0, 0, 20, 9),
                inner_rect: Rect::new(1, 1, 18, 7),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::RIGHT,
                is_focused: true,
            },
            PaneInfo {
                id: collapsed_id,
                rect: Rect::new(0, 9, 20, 1),
                inner_rect: Rect::new(0, 9, 20, 1),
                scrollbar_rect: None,
                // Real chrome (`apply_pane_chrome`) hands a collapsed bar the
                // same full border set as any bordered pane; the renderer
                // must ignore it rather than let it fight the bar's own
                // border characters.
                borders: Borders::ALL,
                is_focused: false,
            },
        ];

        let mut ws = Workspace::test_new("test");
        let collapsed_terminal_id = crate::terminal::TerminalId::alloc();
        ws.tabs[0].panes.insert(
            collapsed_id,
            crate::pane::PaneState::new(collapsed_terminal_id.clone()),
        );
        let mut terminal_state = TerminalState::new(collapsed_terminal_id.clone(), "/tmp".into());
        terminal_state.set_manual_label("collapsed".into());
        app.terminals.insert(collapsed_terminal_id, terminal_state);
        app.workspaces = vec![ws];
        app.active = Some(0);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).unwrap();
        terminal
            .draw(|frame| {
                render_panes(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    &pane_infos,
                    &[],
                    &[],
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let bar_row: String = (0..20).map(|x| buffer[(x, 9)].symbol()).collect();
        assert!(bar_row.contains("collapsed"), "bar row: {bar_row:?}");
        // The generic per-pane junction table turns a lone TOP+BOTTOM+LEFT
        // border on a 1-row rect into "─" at the left edge, not the "│"
        // render_stack_bar draws there — a leftover generic draw would show
        // up as this row's leftmost cell reverting to a horizontal dash.
        assert_eq!(
            buffer[(0, 9)].symbol(),
            "│",
            "generic border drew over the bar's own left edge"
        );
    }
}
