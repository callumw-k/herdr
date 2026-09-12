use super::*;
use crate::protocol::{PaneSurfacePane, SurfaceRect};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

fn surface_pane(pane_id: &str, x: u16, y: u16, width: u16, height: u16) -> PaneSurfacePane {
    PaneSurfacePane {
        pane_id: pane_id.into(),
        content_revision: 0,
        rect: SurfaceRect {
            x,
            y,
            width,
            height,
        },
        inner_rect: SurfaceRect {
            x: x + 1,
            y: y + 1,
            width: width.saturating_sub(2),
            height: height.saturating_sub(2),
        },
        scrollbar_rect: None,
        scroll: None,
        focused: false,
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        alternate_screen_active: false,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// The server lists floats after the tiled panes they cover, so the hit list
/// must be searched top-most first or a click on a float lands underneath it.
#[test]
fn a_click_inside_a_float_focuses_the_float_not_the_pane_beneath() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let mut surface = surface();
    surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(
        &Buffer::with_lines(vec!["x".repeat(60); 20]),
        None,
        &[],
    );
    surface.panes = vec![
        surface_pane("pane_1", 0, 0, 60, 20),
        surface_pane("pane_2", 10, 4, 30, 10),
    ];
    state.set_pane_surface(surface);
    state.compose(106, 28).expect("composed frame");

    let pane_2 = state
        .hits
        .panes
        .iter()
        .find(|h| h.pane_id == "pane_2")
        .expect("pane_2");
    let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: pane_2.inner_rect.x + 5,
        row: pane_2.inner_rect.y + 3,
        modifiers: KeyModifiers::empty(),
    })]);

    assert!(matches!(
        &outcome.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
            )
    ));
}
