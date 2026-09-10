use super::*;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn key(code: KeyCode, modifiers: KeyModifiers) -> RawInputEvent {
    RawInputEvent::Key(crate::input::TerminalKey::new(code, modifiers))
}

fn endpoint_methods(actions: &[ClientShellAction]) -> Vec<&crate::api::schema::Method> {
    actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(&request.method),
            _ => None,
        })
        .collect()
}

#[test]
fn opening_the_workspace_editor_looks_up_the_pinned_path() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let mut outcome = ClientShellInput::default();

    state.open_rename_workspace_overlay_for("ws_1".into(), ClientRenameField::Path, &mut outcome);

    let methods = endpoint_methods(&outcome.actions);
    assert!(matches!(
        methods[..],
        [crate::api::schema::Method::WorkspaceGet(target)] if target.workspace_id == "ws_1"
    ));
    let request_id = match &outcome.actions[0] {
        ClientShellAction::Endpoint { request, .. } => request.id.clone(),
        _ => unreachable!(),
    };
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::WorkspaceInfo {
            workspace: crate::api::schema::WorkspaceInfo {
                path: Some("/repos/herdr".into()),
                ..workspace_info("ws_1")
            },
        }),
    );
    let Some(ClientShellOverlay::Rename(rename)) = state.overlay.as_ref() else {
        panic!("editor open");
    };
    assert_eq!(rename.path_input, "/repos/herdr");
    assert_eq!(rename.field, ClientRenameField::Path);
}

#[test]
fn saving_sends_rename_and_set_path_only_for_changed_fields() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let mut outcome = ClientShellInput::default();
    state.open_rename_workspace_overlay_for("ws_1".into(), ClientRenameField::Name, &mut outcome);
    if let Some(ClientShellOverlay::Rename(rename)) = state.overlay.as_mut() {
        rename.path_loaded = true;
        rename.original_path = String::new();
    }

    state.handle_raw_events(vec![key(KeyCode::Tab, KeyModifiers::NONE)]);
    for character in "/tmp/x".chars() {
        state.handle_raw_events(vec![key(KeyCode::Char(character), KeyModifiers::NONE)]);
    }
    let saved = state.handle_raw_events(vec![key(KeyCode::Enter, KeyModifiers::NONE)]);

    let methods = endpoint_methods(&saved.actions);
    assert!(
        matches!(
            methods[..],
            [crate::api::schema::Method::WorkspaceSetPath(params)]
                if params.workspace_id == "ws_1" && params.path.as_deref() == Some("/tmp/x")
        ),
        "the name was untouched, so only set_path is sent"
    );
}

#[test]
fn clicking_the_path_field_focuses_it_instead_of_cancelling() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut outcome = ClientShellInput::default();
    state.open_rename_workspace_overlay_for("ws_1".into(), ClientRenameField::Name, &mut outcome);
    state.compose(106, 28).expect("composed");
    let path_rect = state
        .hits
        .rename_fields
        .iter()
        .find(|(_, field)| *field == ClientRenameField::Path)
        .map(|(rect, _)| *rect)
        .expect("path field hit");

    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: path_rect.x + 1,
        row: path_rect.y,
        modifiers: KeyModifiers::NONE,
    })]);

    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            field: ClientRenameField::Path,
            ..
        }))
    ));
}

#[test]
fn navigator_p_opens_the_path_editor_for_the_selected_workspace() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.open_navigator_overlay();
    state.handle_raw_events(vec![key(KeyCode::Esc, KeyModifiers::NONE)]);
    let opened = state.handle_raw_events(vec![key(KeyCode::Char('p'), KeyModifiers::NONE)]);

    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            field: ClientRenameField::Path,
            target: ClientRenameTarget::Workspace { ref workspace_id },
            ..
        })) if workspace_id == "ws_1"
    ));
    assert!(matches!(
        endpoint_methods(&opened.actions)[..],
        [crate::api::schema::Method::WorkspaceGet(_)]
    ));
}

#[test]
fn navigator_ctrl_o_opens_the_new_workspace_dialog() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.open_navigator_overlay();
    state.handle_raw_events(vec![key(KeyCode::Esc, KeyModifiers::NONE)]);
    state.handle_raw_events(vec![key(KeyCode::Char('o'), KeyModifiers::CONTROL)]);

    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            target: ClientRenameTarget::NewWorkspace { .. },
            ..
        }))
    ));
}

#[test]
fn a_new_workspace_with_a_path_pins_it_after_creation() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.open_new_workspace_overlay();
    state.handle_raw_events(vec![key(KeyCode::Tab, KeyModifiers::NONE)]);
    for character in "/tmp/new".chars() {
        state.handle_raw_events(vec![key(KeyCode::Char(character), KeyModifiers::NONE)]);
    }
    let created = state.handle_raw_events(vec![key(KeyCode::Enter, KeyModifiers::NONE)]);
    let request_id = match &created.actions[0] {
        ClientShellAction::Endpoint { request, .. } => request.id.clone(),
        _ => panic!("create request"),
    };

    let (_, follow_up) = state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::WorkspaceCreated {
            workspace: workspace_info("ws_9"),
            tab: tab_info("ws_9", "ws_9:t1"),
            root_pane: pane_info("ws_9", "ws_9:t1", "ws_9:p1"),
        }),
    );
    assert!(matches!(
        endpoint_methods(&follow_up)[..],
        [crate::api::schema::Method::WorkspaceSetPath(params)]
            if params.workspace_id == "ws_9" && params.path.as_deref() == Some("/tmp/new")
    ));
}
