use super::*;
use crate::api::schema::AgentStatus;
use crate::protocol::{ClientShellPane, ClientShellTab};

fn two_workspace_snapshot() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    let mut second = snapshot.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    second.active_tab_id = "tab_2".into();
    second.number = 2;
    second.label = "alpha".into();
    second.focused = false;
    snapshot.workspaces[0].label = "beta".into();
    snapshot.workspaces.push(second);
    snapshot.tabs.push(ClientShellTab {
        tab_id: "tab_2".into(),
        workspace_id: "ws_2".into(),
        number: 1,
        label: "1".into(),
        custom_label: false,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
    });
    snapshot.panes[0].label = Some("alpha-runner".into());
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_2".into(),
        tab_id: "tab_2".into(),
        label: Some("zeta".into()),
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
    });
    snapshot
}

fn rows_for(state: &mut ClientShellState, query: &str) -> Vec<ClientNavigatorRow> {
    state.open_navigator_overlay();
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
        panic!("navigator open");
    };
    navigator.query = query.to_owned();
    render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator)
}

#[test]
fn the_navigator_opens_with_search_focused() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.open_navigator_overlay();
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(ClientNavigatorOverlay {
            search_focused: true,
            ..
        }))
    ));
}

#[test]
fn a_query_flattens_to_pane_rows_and_ranks_own_matches_first() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_workspace_snapshot()));

    let rows = rows_for(&mut state, "alpha");
    assert!(rows
        .iter()
        .all(|row| matches!(row.target, ClientNavigatorTarget::Pane { .. })));
    assert_eq!(
        rows[0].label, "alpha-runner",
        "a pane's own match outranks a breadcrumb match"
    );
    assert_eq!(rows[1].label, "zeta");
    assert!(
        rows[1].meta.contains("alpha"),
        "breadcrumb shows why the row matched"
    );
}

#[test]
fn a_query_for_a_workspace_name_returns_only_its_panes() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_workspace_snapshot()));

    let rows = rows_for(&mut state, "beta");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "alpha-runner");
}
