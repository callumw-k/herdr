use ratatui::{layout::Rect, Frame};

use crate::app::state::{AppState, NavigatorRow, NavigatorTarget};
use crate::layout::PaneId;
use crate::terminal::TerminalRuntimeRegistry;
use crate::ui::widgets::render_panel_shell;

/// Draw the selected row's pane into `area`. The pane keeps its real size and
/// is corner-cropped by `TerminalRuntime::render`, which stops at the rect's
/// width and height. `rows` is the list the caller already built for this
/// frame, so the selection lookup doesn't rebuild it a second time.
pub(super) fn render_preview(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    rows: &[NavigatorRow],
    frame: &mut Frame,
    area: Rect,
) {
    // `surface_dim`, not the overlay's `accent`, so the preview reads as nested
    // inside the navigator frame rather than competing with it.
    let Some(inner) =
        render_panel_shell(frame, area, app.palette.surface_dim, app.palette.panel_bg)
    else {
        return;
    };

    let Some(target) = rows
        .get(app.navigator.selected)
        .map(|row| row.target.clone())
    else {
        return;
    };
    let Some((ws_idx, pane_id)) = preview_target(app, target) else {
        return;
    };
    let Some(runtime) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
    else {
        return;
    };
    runtime.render(frame, inner, false);
}

/// A pane row previews itself; tab and workspace rows preview their focused
/// pane.
fn preview_target(app: &AppState, target: NavigatorTarget) -> Option<(usize, PaneId)> {
    match target {
        NavigatorTarget::Pane {
            ws_idx, pane_id, ..
        } => Some((ws_idx, pane_id)),
        NavigatorTarget::Tab { ws_idx, tab_idx } => {
            let tab = app.workspaces.get(ws_idx)?.tabs.get(tab_idx)?;
            Some((ws_idx, tab.focused_pane()))
        }
        NavigatorTarget::Workspace { ws_idx } => {
            let ws = app.workspaces.get(ws_idx)?;
            Some((ws_idx, ws.focused_pane()))
        }
    }
}
