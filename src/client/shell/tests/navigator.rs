use super::*;
use crate::api::schema::AgentStatus;
use crate::client::endpoint::{
    ClientEndpointId, ClientEndpointStatus, ProfileId, SavedSshEndpoint,
};
use crate::protocol::{ClientShellAgent, ClientShellPane, ClientShellTab};

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
        parent_tab_id: None,
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

fn two_pane_tab_snapshot() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.tabs[0].label = "Gap between panes".into();
    snapshot.panes[0].label = Some("Shell".into());
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        label: Some("cargo watch".into()),
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
    });
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    snapshot
}

#[test]
fn a_tab_with_one_visible_pane_collapses_to_a_row_targeting_the_pane() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let rows = rows_for(&mut state, "");
    assert_eq!(rows.len(), 2);
    assert!(matches!(
        rows[0].target,
        ClientNavigatorTarget::Workspace { .. }
    ));
    assert_eq!(rows[1].label, "1");
    assert_eq!(rows[1].title, "1");
    assert!(
        matches!(&rows[1].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );
    assert!(rows[1].status.is_some());
    assert!(rows[1].current);
    assert_eq!(rows[1].meta, "");
}

#[test]
fn a_status_filter_keeps_a_tab_whose_own_status_matches_with_no_visible_panes() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.open_navigator_overlay();
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
        panic!("navigator open")
    };
    navigator.agents_only = false;
    navigator.filter = Some(ClientNavigatorFilter::Idle);
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    assert!(rows
        .iter()
        .all(|row| !matches!(row.target, ClientNavigatorTarget::Pane { .. })));
    let tab = rows
        .iter()
        .find(|row| matches!(row.target, ClientNavigatorTarget::Tab { .. }))
        .expect("tab kept by its own status");
    assert_eq!(tab.meta, "");
}

#[test]
fn a_tab_with_two_visible_panes_lists_them_under_a_tab_row() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_pane_tab_snapshot()));
    let rows = rows_with(&mut state, "", false);
    assert!(matches!(rows[1].target, ClientNavigatorTarget::Tab { .. }));
    assert_eq!(rows[1].meta, "2 panes");
    assert_eq!(rows[2].label, "Shell");
    assert_eq!(rows[2].title, "Gap between panes › Shell");
    assert_eq!(rows[3].label, "cargo watch");
    assert!(rows[2].depth > rows[1].depth);
}

#[test]
fn agents_only_hiding_one_of_two_panes_collapses_the_tab() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_pane_tab_snapshot()));
    let rows = rows_with(&mut state, "", true);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].label, "Gap between panes");
    assert!(
        matches!(&rows[1].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );
}

#[test]
fn opening_selects_the_collapsed_row_of_the_current_pane() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot()));
    state.open_navigator_overlay();
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator open")
    };
    assert!(
        matches!(&navigator.selected, Some(ClientNavigatorTarget::Pane { pane_id, .. }) if pane_id == "pane_1")
    );
}

fn rows_for(state: &mut ClientShellState, query: &str) -> Vec<ClientNavigatorRow> {
    rows_with(state, query, false)
}

fn rows_with(
    state: &mut ClientShellState,
    query: &str,
    agents_only: bool,
) -> Vec<ClientNavigatorRow> {
    state.open_navigator_overlay();
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
        panic!("navigator open");
    };
    navigator.query = TextEditor::from(query);
    navigator.agents_only = agents_only;
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
fn a_query_ranks_primary_field_matches_above_context_matches() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_workspace_snapshot()));

    let rows = rows_for(&mut state, "alpha");
    assert!(rows
        .iter()
        .all(|row| matches!(row.target, ClientNavigatorTarget::Pane { .. })));
    assert!(
        matches!(&rows[0].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1"),
        "a pane's own match outranks a workspace-name match"
    );
    assert_eq!(rows[1].meta, "alpha");
}

#[test]
fn a_query_for_a_workspace_name_returns_only_its_panes() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_workspace_snapshot()));

    let rows = rows_for(&mut state, "beta");
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );
}

#[test]
fn search_rows_lead_with_the_tab_name_and_add_the_pane_only_for_multi_pane_tabs() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_pane_tab_snapshot()));
    let rows = rows_with(&mut state, "gap", false);
    let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Gap between panes › Shell",
            "Gap between panes › cargo watch"
        ]
    );
    let rows = rows_with(&mut state, "gap", true);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "Gap between panes");
    assert_eq!(rows[0].meta, "client-shell");
}

#[test]
fn each_query_term_can_match_a_different_field() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_pane_tab_snapshot()));
    let rows = rows_with(&mut state, "client gap", true);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label_hits, vec![0, 1, 2]);
    assert_eq!(rows[0].meta_hits, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn an_unlabelled_pane_in_a_multi_pane_tab_falls_back_to_its_number() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_pane_tab_snapshot();
    snapshot.panes[1].label = None;
    state.set_snapshot(Box::new(snapshot));
    let rows = rows_with(&mut state, "gap", false);
    assert_eq!(rows[1].label, "Gap between panes › pane 2");
}

#[test]
fn a_primary_match_outranks_a_higher_scoring_secondary_match() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot.tabs[0].label = "wxxoxxrxxkxxexxr".into();
    snapshot.workspaces[1].label = "worker".into();
    assert!(
        crate::fuzzy::fuzzy_score("worker", "wxxoxxrxxkxxexxr")
            < crate::fuzzy::fuzzy_score("worker", "worker")
    );
    state.set_snapshot(Box::new(snapshot));
    let rows = rows_for(&mut state, "worker");
    assert_eq!(rows.len(), 2);
    assert!(
        matches!(&rows[0].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );
    assert_eq!(rows[1].meta, "worker");
}

#[test]
fn a_term_matching_only_a_hidden_field_leaves_no_highlights() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let rows = rows_for(&mut state, "main");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].label_hits.is_empty());
    assert!(rows[0].meta_hits.is_empty());
}

#[test]
fn a_tab_name_match_ranks_above_a_cwd_only_match() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot.tabs[1].label = "repo cleanup".into();
    state.set_snapshot(Box::new(snapshot));
    let rows = rows_for(&mut state, "repo");
    assert_eq!(rows[0].label, "repo cleanup");
}

pub(super) fn agent(
    workspace_id: &str,
    tab_id: &str,
    pane_id: &str,
    kind: Option<&str>,
    title: Option<&str>,
) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: workspace_id.into(),
        tab_id: tab_id.into(),
        name: None,
        display_agent: None,
        agent: kind.map(str::to_owned),
        title: None,
        terminal_title: title.map(|title| format!("◑ {title}")),
        terminal_title_stripped: title.map(str::to_owned),
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
    }
}

fn pane_rows(rows: &[ClientNavigatorRow]) -> Vec<&ClientNavigatorRow> {
    rows.iter()
        .filter(|row| matches!(row.target, ClientNavigatorTarget::Pane { .. }))
        .collect()
}

#[test]
fn a_pane_without_a_label_shows_its_stripped_terminal_title() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    snap.agents.push(agent(
        "ws_1",
        "tab_1",
        "pane_1",
        Some("claude"),
        Some("Code review"),
    ));
    state.set_snapshot(Box::new(snap));

    let rows = rows_for(&mut state, "");
    assert_eq!(pane_rows(&rows)[0].label, "1");

    let rows = rows_for(&mut state, "review");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "1", "a search row leads with the tab name");
    assert!(
        matches!(&rows[0].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );
}

#[test]
fn a_user_label_beats_the_terminal_title_and_a_bare_pane_falls_back_to_its_number() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    snap.panes[0].label = Some("renamed".into());
    snap.agents.push(agent(
        "ws_1",
        "tab_1",
        "pane_1",
        Some("claude"),
        Some("Code review"),
    ));
    state.set_snapshot(Box::new(snap));
    let rows = rows_for(&mut state, "renamed");
    assert_eq!(rows[0].label, "1");
    assert!(
        matches!(&rows[0].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let rows = rows_for(&mut state, "pane");
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0].target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1")
    );
}

#[test]
fn agents_only_seeds_from_config_and_prefers_the_saved_preference() {
    let state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    assert!(state.navigator_agents_only);
    assert!(!state.navigator_agents_only_manual);

    let mut config = ClientShellConfig::from_config(&Config::default());
    config.preferences.navigator_agents_only = Some(false);
    let state = ClientShellState::new(config);
    assert!(!state.navigator_agents_only);
    assert!(state.navigator_agents_only_manual);
}

fn labels(rows: &[ClientNavigatorRow]) -> Vec<&str> {
    rows.iter().map(|row| row.label.as_str()).collect()
}

#[test]
fn agents_only_hides_panes_tabs_and_workspaces_without_agents() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    state.set_snapshot(Box::new(snapshot));

    let rows = rows_with(&mut state, "", true);
    assert_eq!(labels(&rows), vec!["beta", "1"]);

    let rows = rows_with(&mut state, "", false);
    assert_eq!(labels(&rows), vec!["beta", "1", "alpha", "1"]);
}

#[test]
fn agents_only_applies_to_flat_query_rows() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    state.set_snapshot(Box::new(snapshot));

    assert!(rows_with(&mut state, "zeta", true).is_empty());
    assert_eq!(rows_with(&mut state, "zeta", false).len(), 1);
}

#[test]
fn the_navigator_opens_with_the_state_agents_only_setting() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.navigator_agents_only = false;
    state.open_navigator_overlay();
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(ClientNavigatorOverlay {
            agents_only: false,
            ..
        }))
    ));
}

#[test]
fn agents_only_keeps_the_active_machine_row_and_drops_empty_remote_machines() {
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").expect("profile id"),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    state.set_snapshot(Box::new(snapshot()));
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.workspaces[0].label = "remote-workspace".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote.clone()));

    let rows = rows_with(&mut state, "", true);
    let machines: Vec<_> = rows
        .iter()
        .filter(|row| matches!(row.target, ClientNavigatorTarget::Machine { .. }))
        .collect();
    assert_eq!(machines.len(), 1);
    assert!(matches!(
        &machines[0].target,
        ClientNavigatorTarget::Machine { endpoint_id } if *endpoint_id == ClientEndpointId::Local
    ));

    remote
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let rows = rows_with(&mut state, "", true);
    let machine_count = rows
        .iter()
        .filter(|row| matches!(row.target, ClientNavigatorTarget::Machine { .. }))
        .count();
    assert_eq!(machine_count, 2);
}

use crossterm::event::{KeyCode, KeyModifiers};

fn press(state: &mut ClientShellState, code: KeyCode, modifiers: KeyModifiers) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code, modifiers,
    ))])
}

fn open(state: &mut ClientShellState) {
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::OpenNavigator),
        &mut outcome,
    );
}

fn navigator(state: &ClientShellState) -> &ClientNavigatorOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::Navigator(navigator)) => navigator,
        _ => panic!("navigator open"),
    }
}

#[test]
fn esc_closes_the_navigator_from_search_and_tree_mode() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.overlay.is_none());

    open(&mut state);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    assert!(!navigator(&state).search_focused);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.overlay.is_none());
}

#[test]
fn esc_clears_a_query_before_it_closes() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"repo");
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(navigator(&state).query.as_str(), "");
    assert!(navigator(&state).search_focused);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.overlay.is_none());

    open(&mut state);
    state.handle_input_bytes(b"repo");
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(navigator(&state).query.as_str(), "");
    assert!(!navigator(&state).search_focused);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.overlay.is_none());
}

#[test]
fn tab_switches_modes_and_keeps_the_query() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"repo");
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    assert!(!navigator(&state).search_focused);
    assert_eq!(navigator(&state).query.as_str(), "repo");
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    assert!(navigator(&state).search_focused);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(navigator(&state).search_focused);
}

#[test]
fn the_agents_only_toggle_works_in_both_modes_and_is_marked_manual() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    assert!(navigator(&state).agents_only);
    press(&mut state, KeyCode::Char('a'), KeyModifiers::ALT);
    assert!(!navigator(&state).agents_only);
    assert!(!state.navigator_agents_only);
    assert!(state.navigator_agents_only_manual);

    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(navigator(&state).agents_only);
    assert!(state.navigator_agents_only);
}

#[test]
fn status_filters_set_in_search_mode_and_clear_on_repeat() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    press(&mut state, KeyCode::Char('b'), KeyModifiers::ALT);
    assert_eq!(
        navigator(&state).filter,
        Some(ClientNavigatorFilter::Blocked)
    );
    press(&mut state, KeyCode::Char('b'), KeyModifiers::ALT);
    assert_eq!(navigator(&state).filter, None);

    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('w'), KeyModifiers::NONE);
    assert_eq!(
        navigator(&state).filter,
        Some(ClientNavigatorFilter::Working)
    );
    press(&mut state, KeyCode::Char('w'), KeyModifiers::NONE);
    assert_eq!(navigator(&state).filter, None);

    press(&mut state, KeyCode::Char('i'), KeyModifiers::NONE);
    assert_eq!(navigator(&state).filter, Some(ClientNavigatorFilter::Idle));
    assert!(!navigator(&state).search_focused);
    press(&mut state, KeyCode::Char('i'), KeyModifiers::NONE);
    assert_eq!(navigator(&state).filter, None);
}

#[test]
fn the_open_navigator_binding_toggles_the_overlay() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
    open(&mut state);
    assert!(state.overlay.is_none());
}

#[test]
fn prefix_g_closes_the_navigator_from_the_keyboard() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    press(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(state.mode, ClientShellMode::Prefix);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
    press(&mut state, KeyCode::Char('g'), KeyModifiers::NONE);
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);

    open(&mut state);
    press(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(navigator(&state).query.as_str(), "");
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn a_direct_goto_binding_toggles_the_navigator_closed() {
    let mut config = Config::default();
    config.keys.goto =
        crate::config::BindingConfig::Many(vec!["prefix+g".to_owned(), "alt+p".to_owned()]);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));

    press(&mut state, KeyCode::Char('p'), KeyModifiers::ALT);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));

    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('p'), KeyModifiers::ALT);
    assert!(state.overlay.is_none());

    press(&mut state, KeyCode::Char('p'), KeyModifiers::ALT);
    press(&mut state, KeyCode::Char('x'), KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('p'), KeyModifiers::ALT);
    assert!(state.overlay.is_none());
}

#[test]
fn other_direct_bindings_still_reach_the_navigator_query() {
    let mut config = Config::default();
    config.keys.goto = crate::config::BindingConfig::one("alt+p");
    config.keys.new_tab = crate::config::BindingConfig::one("alt+t");
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));

    press(&mut state, KeyCode::Char('p'), KeyModifiers::ALT);
    press(&mut state, KeyCode::Char('t'), KeyModifiers::ALT);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
}

use ratatui::layout::Rect;

#[test]
fn the_navigator_takes_eighty_percent_centred_and_splits_only_when_wide() {
    let g = render::overlays::navigator_geometry(Rect::new(0, 0, 200, 50)).expect("geometry");
    assert_eq!(g.popup, Rect::new(20, 5, 160, 40));
    assert_eq!(g.search, Rect::new(23, 7, 154, 1));
    assert_eq!(g.footer, Rect::new(23, 42, 154, 1));
    assert_eq!(g.tree, Rect::new(23, 9, 69, 32));
    assert_eq!(g.preview, Some(Rect::new(95, 9, 82, 32)));
    assert_eq!(g.detail, None);
    assert_eq!(
        render::overlays::navigator_preview_capacity(200, 50),
        Some(32 - render::overlays::NAVIGATOR_PREVIEW_HEADER_ROWS)
    );

    let mid = render::overlays::navigator_geometry(Rect::new(0, 0, 120, 34)).expect("geometry");
    assert_eq!(mid.popup, Rect::new(12, 3, 96, 27));
    assert_eq!(render::overlays::navigator_preview_capacity(120, 34), None);
    assert!(mid.preview.is_none());
    assert!(mid.detail.is_some());

    let small = render::overlays::navigator_geometry(Rect::new(0, 0, 70, 18)).expect("geometry");
    assert_eq!(small.popup, Rect::new(5, 1, 60, 16));

    let short = render::overlays::navigator_geometry(Rect::new(0, 0, 280, 19)).expect("geometry");
    assert_eq!(short.popup, Rect::new(28, 1, 224, 16));
}

#[test]
fn the_navigator_popup_never_shrinks_as_the_terminal_grows() {
    let popup = |cols, rows| {
        render::overlays::navigator_geometry(Rect::new(0, 0, cols, rows))
            .expect("geometry")
            .popup
    };
    for cols in 20..300 {
        assert!(
            popup(cols + 1, 40).width >= popup(cols, 40).width,
            "{cols} -> {} columns",
            cols + 1
        );
    }
    for rows in 8..80 {
        assert!(
            popup(200, rows + 1).height >= popup(200, rows).height,
            "{rows} -> {} rows",
            rows + 1
        );
    }
    assert_eq!(popup(74, 40).width, 60);
    assert_eq!(popup(75, 40).width, 60);
}

#[test]
fn blank_lines_separate_workspaces_and_are_skipped_by_selection() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    let rows = rows_for(&mut state, "");
    assert_eq!(
        aggregate_navigation::navigator_lines(&rows),
        [Some(0), Some(1), None, Some(2), Some(3)]
    );
    let first_pane = rows[1].target.clone();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(first_pane);
    }
    state.move_navigator_selection(1);
    assert_eq!(navigator(&state).selected, Some(rows[2].target.clone()));
}

#[test]
fn machine_rows_get_a_gap_but_their_first_workspace_does_not() {
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").expect("profile id"),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let second_profile = SavedSshEndpoint {
        id: ProfileId::parse("fedcba9876543210fedcba9876543210").expect("profile id"),
        label: "Deploy".into(),
        target: "dev@deploy.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    let second_id = ClientEndpointId::Ssh(second_profile.id.clone());
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_endpoint_catalog(&[profile, second_profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Reconnecting);
    state.set_endpoint_status(&second_id, ClientEndpointStatus::Reconnecting);
    state.set_snapshot(Box::new(snapshot()));
    let rows = rows_for(&mut state, "");
    let lines = aggregate_navigation::navigator_lines(&rows);
    let machines: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row.target, ClientNavigatorTarget::Machine { .. }))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(machines.len(), 3);
    let position = |row: usize| lines.iter().position(|line| *line == Some(row));
    let second = position(machines[1]).expect("second machine line");
    assert_eq!(lines[second - 1], None, "gap before the second machine");
    assert_ne!(
        lines[position(machines[0]).expect("first machine line") + 1],
        None
    );
    assert!(matches!(
        rows[machines[0] + 1].target,
        ClientNavigatorTarget::Workspace { .. }
    ));
    let third = position(machines[2]).expect("third machine line");
    assert_eq!(lines[third - 1], None, "gap between adjacent machines");
    for machine in machines {
        let line = position(machine).expect("machine line");
        if let Some(Some(next)) = lines.get(line + 1) {
            assert!(
                !matches!(rows[*next].target, ClientNavigatorTarget::Machine { .. }),
                "a machine is never directly followed by another"
            );
        }
        if rows
            .get(machine + 1)
            .is_some_and(|row| matches!(row.target, ClientNavigatorTarget::Workspace { .. }))
        {
            assert_eq!(
                lines.get(line + 1),
                Some(&Some(machine + 1)),
                "no gap between a machine and its first workspace"
            );
        }
    }
}

#[test]
fn row_hits_follow_the_gap_lines() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    state.compose(200, 50).expect("frame");
    let hits = &state.hits.navigator_rows;
    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0].0.y, 9);
    assert_eq!(hits[1].0.y, hits[0].0.y + 1);
    assert_eq!(hits[2].0.y, hits[1].0.y + 2);
    assert_eq!(hits[3].0.y, hits[2].0.y + 1);
}

#[test]
fn resizing_across_the_split_threshold_keeps_the_selection() {
    let mut snapshot = two_workspace_snapshot();
    for k in 0..24 {
        snapshot.tabs.push(ClientShellTab {
            tab_id: format!("tab_x{k}"),
            workspace_id: "ws_2".into(),
            number: k + 2,
            label: format!("extra {k}"),
            custom_label: false,
            zoomed: false,
            focused: false,
            agent_status: AgentStatus::Idle,
            parent_tab_id: None,
        });
        snapshot.panes.push(ClientShellPane {
            pane_id: format!("pane_x{k}"),
            workspace_id: "ws_2".into(),
            tab_id: format!("tab_x{k}"),
            label: Some(format!("extra {k}")),
            cwd: Some("/repo".into()),
            foreground_cwd: Some("/repo".into()),
            focused: false,
            right_click_passthrough: false,
        });
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let rows = rows_for(&mut state, "");
    let target = rows.last().expect("rows").target.clone();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(target.clone());
    }
    for (cols, height) in [(200, 50), (120, 34), (200, 50)] {
        state.compose(cols, height).expect("frame");
        assert_eq!(navigator(&state).selected, Some(target.clone()));
        assert!(state
            .hits
            .navigator_rows
            .iter()
            .any(|(_, hit)| *hit == target));
        if cols == 120 {
            assert!(
                state.hits.navigator_rows.len() < rows.len(),
                "the narrow layout scrolls"
            );
        }
    }
}

#[test]
fn a_query_with_no_matches_shows_zero_of_total_and_enter_is_inert() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.query = TextEditor::from("zzzz");
    }
    let text = frame_text(&mut state, 200, 50);
    assert!(text.contains("0 of 2 panes"), "{text}");
    assert!(state.hits.navigator_rows.is_empty());
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::Navigator(_))
    ));
}

fn frame_text(state: &mut ClientShellState, cols: u16, rows: u16) -> String {
    let frame = state.compose(cols, rows).expect("frame");
    frame_rows(&frame).join("\n")
}

fn frame_line(state: &mut ClientShellState, cols: u16, rows: u16, area: Rect) -> String {
    let frame = state.compose(cols, rows).expect("frame");
    frame_rows(&frame)[area.y as usize]
        .chars()
        .skip(area.x as usize)
        .take(area.width as usize)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[test]
fn the_count_names_its_noun_with_and_without_a_query() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    snapshot
        .agents
        .push(agent("ws_2", "tab_2", "pane_2", Some("codex"), None));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let search = render::overlays::navigator_geometry(Rect::new(0, 0, 200, 50))
        .expect("geometry")
        .search;
    let count = |state: &mut ClientShellState, query: &str, agents_only, filter| {
        if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
            navigator.query = TextEditor::from(query);
            navigator.agents_only = agents_only;
            navigator.filter = filter;
            navigator.search_focused = false;
        }
        let line = frame_line(state, 200, 50, search);
        line.rsplit("  ")
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    assert_eq!(count(&mut state, "", false, None), "2 panes");
    assert_eq!(count(&mut state, "zeta", false, None), "1 of 2 panes");
    assert_eq!(count(&mut state, "", true, None), "2 agents");
    assert_eq!(count(&mut state, "zeta", true, None), "1 of 2 agents");
    assert_eq!(
        count(&mut state, "", true, Some(ClientNavigatorFilter::Working)),
        "working · 2 agents"
    );
    assert_eq!(
        count(
            &mut state,
            "zeta",
            true,
            Some(ClientNavigatorFilter::Working)
        ),
        "working · 1 of 2 agents"
    );
}

#[test]
fn a_long_tab_name_beside_a_short_workspace_is_drawn_whole() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = snapshot();
    let name = "a forty-five character tab name for searching";
    assert_eq!(name.chars().count(), 45);
    snapshot.tabs[0].label = name.into();
    snapshot.workspaces[0].label = "web".into();
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    set_search(&mut state, "forty");
    state.compose(160, 40).expect("frame");
    let rect = state.hits.navigator_rows.first().expect("search row").0;
    let row = frame_line(&mut state, 160, 40, rect);
    assert!(row.contains(name), "{row}");
    assert!(row.contains("web"), "{row}");
}

#[test]
fn the_narrow_detail_line_leads_with_the_workspace_and_ends_with_the_path() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    assert!(
        matches!(&navigator(&state).selected, Some(ClientNavigatorTarget::Pane { pane_id, .. }) if pane_id == "pane_1")
    );
    let detail = render::overlays::navigator_geometry(Rect::new(0, 0, 120, 34))
        .expect("geometry")
        .detail
        .expect("detail line");
    let line = frame_line(&mut state, 120, 34, detail);
    assert_eq!(line, "client-shell › 1 · /repo");

    state.set_endpoint_catalog(&[SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").expect("profile id"),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    }]);
    let machine = state.endpoints[0].label.clone();
    let line = frame_line(&mut state, 120, 34, detail);
    assert_eq!(line, format!("{machine} › client-shell › 1 · /repo"));
}

#[test]
fn the_narrow_detail_line_is_hidden_under_the_key_sheet() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let detail = render::overlays::navigator_geometry(Rect::new(0, 0, 120, 34))
        .expect("geometry")
        .detail
        .expect("detail line");
    assert!(frame_line(&mut state, 120, 34, detail).ends_with("/repo"));
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    assert_eq!(frame_line(&mut state, 120, 34, detail), "");
}

#[test]
fn the_preview_column_shows_the_selected_pane_lines_and_placeholders() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = snapshot();
    snapshot.agents.push(agent(
        "ws_1",
        "tab_1",
        "pane_1",
        Some("claude"),
        Some("Code review"),
    ));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();

    let text = frame_text(&mut state, 160, 40);
    assert!(text.contains("loading"), "no read yet:\n{text}");
    assert!(text.contains("claude · working"));

    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.preview = Some(ClientNavigatorPreview {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_1".into(),
            lines: preview_ansi::parse_lines("$ cargo test\nok"),
            error: None,
            scroll: 0,
            requested_lines: 0,
            requested_at: None,
            received_at: None,
        });
    }
    let text = frame_text(&mut state, 160, 40);
    assert!(text.contains("$ cargo test"), "{text}");
    assert!(!text.contains("loading"));

    let text = frame_text(&mut state, 80, 30);
    assert!(
        !text.contains("$ cargo test"),
        "narrow layout drops the preview"
    );
}

#[test]
fn in_flight_is_true_only_while_a_request_has_no_newer_reply() {
    let now = std::time::Instant::now();
    let later = now + std::time::Duration::from_secs(1);
    let preview = |requested_at, received_at| ClientNavigatorPreview {
        endpoint_id: ClientEndpointId::Local,
        pane_id: "pane_1".into(),
        lines: Vec::new(),
        error: None,
        scroll: 0,
        requested_lines: 0,
        requested_at,
        received_at,
    };
    assert!(!preview(None, None).in_flight(), "never requested");
    assert!(preview(Some(now), None).in_flight(), "awaiting a reply");
    assert!(
        preview(Some(later), Some(now)).in_flight(),
        "the reply predates the latest request"
    );
    assert!(
        !preview(Some(now), Some(later)).in_flight(),
        "the reply answers the latest request"
    );
}

use std::time::{Duration, Instant};

fn preview_state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = snapshot();
    snapshot.agents.push(agent(
        "ws_1",
        "tab_1",
        "pane_1",
        Some("claude"),
        Some("Code review"),
    ));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(160, 40).expect("frame");
    state
}

fn pane_reads(actions: &[ClientShellAction]) -> Vec<&crate::api::schema::PaneReadParams> {
    actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match &request.method {
                crate::api::schema::Method::PaneRead(params) => Some(params),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn read_result(pane_id: &str, lines: &[&str]) -> crate::api::schema::ResponseResult {
    crate::api::schema::ResponseResult::PaneRead {
        read: crate::api::schema::PaneReadResult {
            pane_id: pane_id.into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            source: crate::api::schema::ReadSource::Visible,
            format: crate::api::schema::ReadFormat::Text,
            text: lines.join("\n"),
            revision: 1,
            truncated: false,
        },
    }
}

#[test]
fn the_preview_read_is_allowed_on_the_client_shell_lane() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    let methods = outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(&request.method),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!methods.is_empty(), "the tick should have polled");
    for method in methods {
        assert!(
            crate::server::client_commands::supports_client_shell_method(method),
            "{} is not advertised on the client shell lane, so the preview would only show an error",
            crate::api::api_method_name(method)
        );
    }
}

#[test]
fn the_preview_does_not_poll_while_another_request_is_pending() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let mut busy = ClientShellInput::default();
    state.push_endpoint_method_with_kind(
        crate::api::schema::Method::WorkspaceList(crate::api::schema::EmptyParams::default()),
        PendingEndpointKind::Generic,
        &mut busy,
    );
    assert_eq!(busy.actions.len(), 1);
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(
        pane_reads(&outcome.actions).is_empty(),
        "the client shell lane runs one command at a time; the preview must yield"
    );
}

#[test]
fn a_long_pane_title_is_truncated_so_the_status_dot_stays_visible() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = snapshot();
    let long = "Claude Code /Users/someone/Documents/code/some/very/deep/project/directory/name";
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), Some(long)));
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        label: Some("cargo watch".into()),
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
    });
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();

    let text = frame_text(&mut state, 100, 30);
    let row = text
        .lines()
        .find(|line| line.contains("Claude Code"))
        .expect("pane row");
    assert!(row.contains('…'), "{row}");
    assert!(
        row.trim_end_matches(|c: char| c.is_whitespace() || c == '│')
            .ends_with('●'),
        "the status dot must survive a long title: {row}"
    );

    let detail = frame_text(&mut state, 100, 30);
    assert!(
        detail
            .lines()
            .any(|line| line.contains("…") && line.contains("· /repo")),
        "the detail line keeps the path:\n{detail}"
    );

    let text = frame_text(&mut state, 160, 40);
    assert!(
        text.lines().any(|line| line.contains("/repo")),
        "the preview header shows the path:\n{text}"
    );
}

fn preview_of(state: &ClientShellState) -> &ClientNavigatorPreview {
    navigator(state).preview.as_ref().expect("preview entry")
}

fn scroll_wheel(state: &mut ClientShellState, kind: MouseEventKind, column: u16, row: u16) {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })]);
}

#[test]
fn scrolling_the_preview_reads_recent_history_and_clamps_at_the_top() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let now = Instant::now();
    let mut first = ClientShellInput::default();
    state.tick_navigator(now, &mut first);
    let capacity = render::overlays::navigator_preview_capacity(160, 40).expect("capacity");
    let first_id = match &first.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        _ => panic!("one request"),
    };
    state.handle_endpoint_result("boot-1", &first_id, Ok(read_result("pane_1", &["tail"])));

    let mut outcome = ClientShellInput::default();
    state.scroll_navigator_preview(10, &mut outcome);
    assert!(outcome.repaint);
    assert_eq!(preview_of(&state).scroll, 10);

    let mut next = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(100), &mut next);
    let reads = pane_reads(&next.actions);
    assert_eq!(reads.len(), 1, "a scroll refetches on the next tick");
    assert_eq!(reads[0].source, crate::api::schema::ReadSource::Recent);
    assert_eq!(
        reads[0].lines,
        Some(u32::from(capacity) * 2 + 10),
        "the read fetches a page beyond the target so wheel steps hit the buffer"
    );

    let short: Vec<String> = (0..usize::from(capacity) + 4)
        .map(|index| format!("line {index}"))
        .collect();
    let short_refs: Vec<&str> = short.iter().map(String::as_str).collect();
    let request_id = match &next.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        _ => panic!("one request"),
    };
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(read_result("pane_1", &short_refs)),
    );
    assert_eq!(
        preview_of(&state).scroll,
        4,
        "history shorter than requested clamps the offset to what exists"
    );

    let text = frame_text(&mut state, 160, 40);
    assert!(
        text.contains("line 0"),
        "window starts at the oldest row:\n{text}"
    );
    assert!(!text.contains(&format!("line {}", usize::from(capacity) + 3)));
    assert!(text.contains("↑4"));

    let mut down = ClientShellInput::default();
    state.scroll_navigator_preview(-100, &mut down);
    assert_eq!(preview_of(&state).scroll, 0);
}

fn numbered(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("line {index}")).collect()
}

fn complete_read(state: &mut ClientShellState, outcome: &ClientShellInput, lines: &[String]) {
    let request_id = match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        _ => panic!("one request"),
    };
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    state.handle_endpoint_result("boot-1", &request_id, Ok(read_result("pane_1", &refs)));
}

#[test]
fn a_scroll_past_the_held_buffer_keeps_the_window_full_until_history_arrives() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let capacity =
        usize::from(render::overlays::navigator_preview_capacity(160, 40).expect("capacity"));
    let mut first = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut first);
    complete_read(&mut state, &first, &numbered(capacity));

    state.scroll_navigator_preview(10, &mut ClientShellInput::default());
    let text = frame_text(&mut state, 160, 40);
    assert!(text.contains("line 0"), "oldest held row is shown:\n{text}");
    assert!(
        text.contains(&format!("line {}", capacity - 1)),
        "the window stays full instead of dropping rows the buffer has not fetched yet:\n{text}"
    );
    assert!(
        text.split('↑')
            .skip(1)
            .all(|rest| !rest.starts_with(|c: char| c.is_ascii_digit())),
        "no offset is claimed until the buffer has it:\n{text}"
    );
}

#[test]
fn scrolling_while_a_read_is_in_flight_refetches_as_soon_as_it_lands() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let now = Instant::now();
    let capacity =
        usize::from(render::overlays::navigator_preview_capacity(160, 40).expect("capacity"));
    let mut first = ClientShellInput::default();
    state.tick_navigator(now, &mut first);

    state.scroll_navigator_preview(10, &mut ClientShellInput::default());
    complete_read(&mut state, &first, &numbered(capacity));

    let mut next = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(10), &mut next);
    let reads = pane_reads(&next.actions);
    assert_eq!(
        reads.len(),
        1,
        "the interval does not delay a read the scroll needs"
    );
    assert_eq!(
        reads[0].lines,
        Some(u32::try_from(capacity * 2 + 10).expect("fits"))
    );

    state.scroll_navigator_preview(-5, &mut ClientShellInput::default());
    complete_read(&mut state, &next, &numbered(capacity * 2 + 10));
    let mut settled = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(20), &mut settled);
    assert!(
        pane_reads(&settled.actions).is_empty(),
        "a target inside the held buffer waits for the normal interval"
    );
}

#[test]
fn changing_the_selection_resets_the_preview_scroll() {
    let mut state = preview_state();
    let mut snapshot = snapshot();
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        label: None,
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
    });
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_2", Some("codex"), None));
    state.set_snapshot(Box::new(snapshot));
    state.open_navigator_overlay();
    let now = Instant::now();
    state.tick_navigator(now, &mut ClientShellInput::default());
    state.scroll_navigator_preview(5, &mut ClientShellInput::default());
    assert_eq!(preview_of(&state).scroll, 5);

    state.move_navigator_selection(1);
    state.tick_navigator(
        now + Duration::from_millis(10),
        &mut ClientShellInput::default(),
    );
    assert_eq!(preview_of(&state).pane_id, "pane_2");
    assert_eq!(preview_of(&state).scroll, 0);
}

#[test]
fn the_wheel_scrolls_the_preview_over_the_preview_column_and_the_tree_elsewhere() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    state.tick_navigator(Instant::now(), &mut ClientShellInput::default());
    state.compose(160, 40).expect("frame");
    let preview = state.hits.navigator_preview;
    assert!(preview.width > 0);

    scroll_wheel(
        &mut state,
        MouseEventKind::ScrollUp,
        preview.x + 2,
        preview.y + 5,
    );
    assert_eq!(preview_of(&state).scroll, NAVIGATOR_PREVIEW_WHEEL_LINES);
    scroll_wheel(
        &mut state,
        MouseEventKind::ScrollDown,
        preview.x + 2,
        preview.y + 5,
    );
    assert_eq!(preview_of(&state).scroll, 0);

    let mut outcome = ClientShellInput::default();
    state.scroll_navigator_preview(7, &mut outcome);
    press(&mut state, KeyCode::PageDown, KeyModifiers::NONE);
    assert_eq!(
        preview_of(&state).scroll,
        0,
        "page down from 7 lands at the bottom"
    );
    press(&mut state, KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(preview_of(&state).scroll, state.navigator_preview_page());
}

#[test]
fn ansi_colour_in_the_read_reaches_the_frame() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    let request_id = match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        _ => panic!("one request"),
    };
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(read_result("pane_1", &["plain \x1b[31mred\x1b[0m end"])),
    );
    let frame = state.compose(160, 40).expect("frame");
    let preview = state.hits.navigator_preview;
    let (x, y) = cell_symbol_position(&frame, preview, "red");
    let cell = &frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)];
    assert_eq!(
        cell.fg,
        crate::protocol::color_to_u32(ratatui::style::Color::Red)
    );
    let (px, py) = cell_symbol_position(&frame, preview, "plain");
    let plain = &frame.cells[usize::from(py) * usize::from(frame.width) + usize::from(px)];
    assert_ne!(plain.fg, cell.fg);
}

#[test]
fn a_request_with_no_reply_is_retried_after_it_goes_stale() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let now = Instant::now();
    let mut first = ClientShellInput::default();
    state.tick_navigator(now, &mut first);
    assert_eq!(pane_reads(&first.actions).len(), 1);

    let mut waiting = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_secs(2), &mut waiting);
    assert!(waiting.actions.is_empty(), "still waiting on the reply");

    let mut retry = ClientShellInput::default();
    state.tick_navigator(
        now + NAVIGATOR_PREVIEW_STALE + Duration::from_millis(1),
        &mut retry,
    );
    assert_eq!(
        pane_reads(&retry.actions).len(),
        1,
        "a lost reply must not block polling forever"
    );
}

#[test]
fn the_tick_requests_a_preview_for_the_selected_pane_and_waits_for_the_reply() {
    let mut state = preview_state();
    let now = Instant::now();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(now, &mut outcome);
    assert!(outcome.actions.is_empty(), "nothing polls while closed");

    state.open_navigator_overlay();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(now, &mut outcome);
    let reads = pane_reads(&outcome.actions);
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].pane_id, "pane_1");
    assert_eq!(reads[0].source, crate::api::schema::ReadSource::Visible);
    assert_eq!(reads[0].format, crate::api::schema::ReadFormat::Ansi);
    assert!(!reads[0].strip_ansi);
    assert_eq!(
        reads[0].lines,
        render::overlays::navigator_preview_capacity(160, 40).map(u32::from)
    );

    let mut again = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_secs(2), &mut again);
    assert!(again.actions.is_empty(), "one request in flight at a time");

    let request_id = match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        other => panic!("expected one request, got {}", other.len()),
    };
    let (repaint, _) = state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(read_result("pane_1", &["$ cargo test", "ok"])),
    );
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        if let Some(preview) = navigator.preview.as_mut() {
            preview.received_at = Some(now);
        }
    }
    assert!(repaint);
    assert_eq!(
        navigator(&state).preview.as_ref().unwrap().lines,
        preview_ansi::parse_lines("$ cargo test\nok")
    );

    let mut soon = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(100), &mut soon);
    assert!(
        soon.actions.is_empty(),
        "waits for the interval after a reply"
    );
    let mut later = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(700), &mut later);
    assert_eq!(pane_reads(&later.actions).len(), 1);
}

#[test]
fn changing_the_selection_requests_the_new_pane_immediately_and_drops_stale_replies() {
    let mut state = preview_state();
    let mut snapshot = snapshot();
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        label: None,
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
    });
    snapshot.agents.push(agent(
        "ws_1",
        "tab_1",
        "pane_1",
        Some("claude"),
        Some("Code review"),
    ));
    snapshot.agents.push(agent(
        "ws_1",
        "tab_1",
        "pane_2",
        Some("codex"),
        Some("Docs"),
    ));
    state.set_snapshot(Box::new(snapshot));
    state.open_navigator_overlay();
    let now = Instant::now();
    let mut first = ClientShellInput::default();
    state.tick_navigator(now, &mut first);
    let first_id = match &first.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        _ => panic!("one request"),
    };

    state.move_navigator_selection(1);
    let mut second = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(10), &mut second);
    assert_eq!(pane_reads(&second.actions)[0].pane_id, "pane_2");

    state.handle_endpoint_result("boot-1", &first_id, Ok(read_result("pane_1", &["old"])));
    let preview = navigator(&state).preview.as_ref().unwrap();
    assert_eq!(preview.pane_id, "pane_2");
    assert!(
        preview.lines.is_empty(),
        "a reply for the previous pane is discarded"
    );
}

#[test]
fn a_failed_read_records_an_error_without_an_endpoint_notice() {
    let mut state = preview_state();
    state.open_navigator_overlay();
    let now = Instant::now();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(now, &mut outcome);
    let request_id = match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        _ => panic!("one request"),
    };
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("endpoint_timeout".into()),
            message: "timed out".into(),
        }),
    );
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        if let Some(preview) = navigator.preview.as_mut() {
            preview.received_at = Some(now);
        }
    }
    assert_eq!(
        navigator(&state).preview.as_ref().unwrap().error.as_deref(),
        Some("timed out")
    );
    assert!(state.visible_endpoint_notice.is_none());

    let mut retry = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(100), &mut retry);
    assert!(retry.actions.is_empty());
    let mut retry = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_millis(600), &mut retry);
    assert_eq!(pane_reads(&retry.actions).len(), 1);
}

#[test]
fn a_pane_on_another_machine_shows_the_current_machine_placeholder() {
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").expect("profile id"),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.workspaces[0].label = "remote-workspace".into();
    remote.panes[0].pane_id = "remote_pane".into();
    remote
        .agents
        .push(agent("ws_1", "tab_1", "remote_pane", Some("claude"), None));
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    state.navigator_agents_only = true;
    state.compose(160, 40).expect("frame");
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Pane {
            endpoint_id: endpoint_id.clone(),
            pane_id: "remote_pane".into(),
        });
    }

    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(pane_reads(&outcome.actions).is_empty());
    let preview = navigator(&state).preview.as_ref().expect("preview");
    assert_eq!(preview.endpoint_id, endpoint_id);
    assert_eq!(preview.pane_id, "remote_pane");
    assert_eq!(
        preview.error.as_deref(),
        Some("preview shows panes on the current machine only")
    );
    let text = frame_text(&mut state, 160, 40);
    assert!(
        text.contains("preview shows panes on the current machine only"),
        "{text}"
    );
}

#[test]
fn an_endpoint_without_pane_read_is_not_polled() {
    let mut state = preview_state();
    if let Some(endpoint) = state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id.is_local())
    {
        endpoint.methods = Some(std::collections::HashSet::from([
            "workspace.list".to_owned()
        ]));
    }
    state.open_navigator_overlay();
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(outcome.actions.is_empty());
    assert!(navigator(&state)
        .preview
        .as_ref()
        .unwrap()
        .error
        .as_deref()
        .unwrap()
        .contains("unsupported"));
    assert!(state.visible_endpoint_notice.is_none());
}

#[test]
fn navigator_on_start_opens_after_the_first_snapshot_and_after_onboarding() {
    let mut config = Config::default();
    config.ui.navigator_on_start = true;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(state.overlay.is_none(), "no snapshot yet");

    state.set_snapshot(Box::new(snapshot()));
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
    assert!(outcome.repaint);

    state.overlay = None;
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(state.overlay.is_none(), "opens once, not every tick");

    let mut state = ClientShellState::new(
        ClientShellConfig::from_config(&config).with_startup_onboarding(true),
    );
    state.set_snapshot(Box::new(snapshot()));
    let mut outcome = ClientShellInput::default();
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Onboarding)
    ));
    state.overlay = None;
    state.tick_navigator(Instant::now(), &mut outcome);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
}

fn peach(state: &ClientShellState) -> u32 {
    crate::protocol::color_to_u32(state.config.palette.peach)
}

fn cell_at(frame: &FrameData, x: u16, y: u16) -> &crate::protocol::CellData {
    &frame.cells[y as usize * frame.width as usize + x as usize]
}

fn set_search(state: &mut ClientShellState, query: &str) {
    state.open_navigator_overlay();
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
        panic!("navigator open");
    };
    navigator.query = TextEditor::from(query);
    navigator.agents_only = false;
}

#[test]
fn matched_characters_are_highlighted_in_search_rows() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_pane_tab_snapshot()));
    state.set_pane_surface(surface());
    set_search(&mut state, "gap");
    let frame = state.compose(200, 50).expect("frame");
    let hits = state.hits.navigator_rows.clone();
    assert_eq!(hits.len(), 2);
    let bold = ratatui::style::Modifier::BOLD.bits();
    let underlined = ratatui::style::Modifier::UNDERLINED.bits();

    let (x, y) = cell_symbol_position(&frame, hits[1].0, "Gap");
    for dx in 0..3 {
        let cell = cell_at(&frame, x + dx, y);
        assert_eq!(cell.fg, peach(&state), "unselected cell {dx}");
        assert_eq!(cell.modifier & bold, bold);
    }
    assert_ne!(cell_at(&frame, x + 3, y).fg, peach(&state));

    let (x, y) = cell_symbol_position(&frame, hits[0].0, "Gap");
    let rest = cell_at(&frame, x + 3, y);
    for dx in 0..3 {
        let cell = cell_at(&frame, x + dx, y);
        assert_eq!(cell.fg, rest.fg, "selected row keeps the selected fg");
        assert_eq!(cell.bg, rest.bg);
        assert_eq!(cell.modifier & (bold | underlined), bold | underlined);
    }
    assert_eq!(rest.modifier & underlined, 0);

    set_search(&mut state, "client");
    let frame = state.compose(200, 50).expect("frame");
    let hits = state.hits.navigator_rows.clone();
    assert_eq!(hits.len(), 2);
    let (x, y) = cell_symbol_position(&frame, hits[1].0, "client-shell");
    for dx in 0..6 {
        assert_eq!(
            cell_at(&frame, x + dx, y).fg,
            peach(&state),
            "meta cell {dx}"
        );
    }
    assert_ne!(cell_at(&frame, x + 6, y).fg, peach(&state));
}

#[test]
fn status_dots_stay_left_of_the_scrollbar() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_pane_tab_snapshot();
    for index in 0..40 {
        let pane_id = format!("extra_{index}");
        snapshot.panes.push(ClientShellPane {
            pane_id: pane_id.clone(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            label: Some(format!("extra {index}")),
            cwd: Some("/repo".into()),
            foreground_cwd: Some("/repo".into()),
            focused: false,
            right_click_passthrough: false,
        });
        snapshot
            .agents
            .push(agent("ws_1", "tab_1", &pane_id, Some("claude"), None));
    }
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let frame = state.compose(160, 40).expect("frame");
    let scrollbar = state.hits.navigator_scrollbar;
    assert!(scrollbar.width > 0, "expected a scrollbar");
    let dot = status_dot(AgentStatus::Working);
    let panes = state
        .hits
        .navigator_rows
        .iter()
        .filter(|(_, target)| matches!(target, ClientNavigatorTarget::Pane { .. }))
        .collect::<Vec<_>>();
    assert!(panes.len() > 10);
    for (rect, _) in panes {
        assert_eq!(rect.right(), scrollbar.x);
        assert_eq!(cell_at(&frame, rect.right() - 1, rect.y).symbol, dot);
        assert_ne!(cell_at(&frame, scrollbar.x, rect.y).symbol, dot);
    }
}

#[test]
fn status_dots_share_one_column() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    snapshot
        .agents
        .push(agent("ws_2", "tab_2", "pane_2", Some("claude"), None));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let frame = state.compose(160, 40).expect("frame");
    let dot = status_dot(AgentStatus::Working);
    let panes = state
        .hits
        .navigator_rows
        .iter()
        .filter(|(_, target)| matches!(target, ClientNavigatorTarget::Pane { .. }))
        .collect::<Vec<_>>();
    assert_eq!(panes.len(), 2);
    for (rect, _) in panes {
        let dots = (rect.x..rect.right())
            .filter(|x| cell_at(&frame, *x, rect.y).symbol == dot)
            .collect::<Vec<_>>();
        assert_eq!(dots, vec![rect.right() - 1]);
    }
}

#[test]
fn a_truncated_label_never_highlights_the_ellipsis() {
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 10, 1));
    let base = ratatui::style::Style::default().fg(ratatui::style::Color::Blue);
    let lit = ratatui::style::Style::default().fg(ratatui::style::Color::Red);
    let width = render::overlays::paint_navigator_text(
        &mut buffer,
        0,
        0,
        4,
        "abcdef",
        &[3, 4, 5],
        base,
        lit,
    );
    assert_eq!(width, 4);
    assert_eq!(buffer[(3, 0)].symbol(), "…");
    assert_eq!(buffer[(3, 0)].fg, ratatui::style::Color::Blue);
    for x in 0..3 {
        assert_eq!(buffer[(x, 0)].fg, ratatui::style::Color::Blue);
    }
}

#[test]
fn highlights_land_on_wide_characters_by_display_column() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot.tabs[0].label = "日本 gap".into();
    snapshot.tabs[1].label = "日本 gap".into();
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    set_search(&mut state, "gap");
    let frame = state.compose(200, 50).expect("frame");
    let hits = state.hits.navigator_rows.clone();
    assert_eq!(hits.len(), 2);
    let rect = hits[1].0;
    let (start, y) = cell_symbol_position(&frame, rect, "日");
    let lit = (rect.x..rect.right())
        .filter(|x| cell_at(&frame, *x, y).fg == peach(&state))
        .collect::<Vec<_>>();
    assert_eq!(lit, vec![start + 5, start + 6, start + 7]);
}

#[test]
fn the_preview_header_shows_title_path_and_status() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_pane_tab_snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
        panic!("navigator open");
    };
    navigator.agents_only = false;
    navigator.selected = Some(ClientNavigatorTarget::Pane {
        endpoint_id: state.active_endpoint_id.clone(),
        pane_id: "pane_1".into(),
    });
    let frame = state.compose(200, 50).expect("frame");
    let g = render::overlays::navigator_geometry(Rect::new(0, 0, 200, 50)).expect("geometry");
    let preview = g.preview.expect("preview");
    let line = |frame: &FrameData, offset: u16| {
        frame_rows(frame)[(preview.y + offset) as usize]
            .chars()
            .skip(preview.x as usize)
            .take(preview.width as usize)
            .collect::<String>()
            .trim_end()
            .to_owned()
    };
    assert_eq!(line(&frame, 0), "Gap between panes › Shell");
    assert_eq!(line(&frame, 1), "/repo");
    assert_eq!(line(&frame, 2), "claude · working");
    assert_eq!(line(&frame, 3), "");
    assert_eq!(line(&frame, 4), "loading");
    assert_eq!(
        frame_rows(&frame)[preview.y as usize]
            .chars()
            .nth(usize::from(preview.x) - 2),
        Some('│'),
        "one blank column separates the divider from the preview text"
    );

    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Tab {
            endpoint_id: ClientEndpointId::Local,
            tab_id: "tab_1".into(),
        });
    }
    let frame = state.compose(200, 50).expect("frame");
    assert_eq!(line(&frame, 0), "select a pane");
}

#[test]
fn ctrl_j_and_ctrl_k_move_the_selection_without_editing_the_query() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"repo");
    assert_eq!(navigator(&state).query.as_str(), "repo");
    let before = navigator(&state).selected.clone();

    press(&mut state, KeyCode::Char('j'), KeyModifiers::CONTROL);
    let moved = navigator(&state).selected.clone();
    assert!(moved.is_some());
    assert_ne!(moved, before);
    assert_eq!(navigator(&state).query.as_str(), "repo");

    press(&mut state, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert_ne!(navigator(&state).selected, moved);
    assert_eq!(navigator(&state).query.as_str(), "repo");
}

#[test]
fn f1_toggles_the_key_sheet_and_esc_closes_it_before_goto() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    assert!(!navigator(&state).keys_open);

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    assert!(navigator(&state).keys_open);
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!navigator(&state).keys_open);
    assert!(state.overlay.is_some());

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    assert!(!navigator(&state).keys_open);
    assert!(state.overlay.is_some());
}

#[test]
fn question_mark_opens_the_key_sheet_in_tree_mode_and_types_in_search_mode() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"?");
    assert_eq!(navigator(&state).query.as_str(), "?");
    assert!(!navigator(&state).keys_open);

    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    assert!(navigator(&state).keys_open);
}

#[test]
fn the_key_sheet_replaces_the_rows_and_wraps_into_a_second_column_when_short() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);

    let text = frame_text(&mut state, 160, 22);
    assert!(text.contains("Tree keys"), "{text}");
    assert!(text.contains("new workspace"), "{text}");
    assert!(text.contains("j/k  ↑↓  ctrl+j/k"), "{text}");
    assert!(!text.contains("select a pane"), "{text}");
}

#[test]
fn the_open_key_sheet_swallows_plain_keys() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"repo");
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    let selected = navigator(&state).selected.clone();
    state.handle_input_bytes(b"x");
    press(&mut state, KeyCode::Char('x'), KeyModifiers::NONE);
    press(&mut state, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(navigator(&state).query.as_str(), "repo");
    assert_eq!(navigator(&state).selected, selected);
    assert!(navigator(&state).keys_open);

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(navigator(&state).selected, selected);
    assert!(navigator(&state).keys_open);
}

#[test]
fn a_paste_while_the_key_sheet_is_open_leaves_the_query_alone() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"repo");
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    assert!(!state.modal_paste_target_active());
    state.handle_raw_events(vec![RawInputEvent::Paste("pasted".into())]);
    assert_eq!(navigator(&state).query.as_str(), "repo");
}

#[test]
fn a_shifted_question_mark_with_generated_text_opens_the_key_sheet_in_tree_mode() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    open(&mut state);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    state.handle_raw_events(vec![RawInputEvent::Key(
        crate::input::TerminalKey::new(KeyCode::Char('/'), KeyModifiers::SHIFT)
            .with_generated_text(Some("?".into())),
    )]);
    assert!(navigator(&state).keys_open);
}

#[test]
fn the_wheel_does_nothing_underneath_the_open_key_sheet() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = two_workspace_snapshot();
    snapshot
        .agents
        .push(agent("ws_1", "tab_1", "pane_1", Some("claude"), None));
    snapshot
        .agents
        .push(agent("ws_2", "tab_2", "pane_2", Some("codex"), None));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(160, 40).expect("frame");
    state.open_navigator_overlay();
    state.tick_navigator(Instant::now(), &mut ClientShellInput::default());
    state.compose(160, 40).expect("frame");
    assert!(state.hits.navigator_rows.len() >= 4);
    assert!(state.hits.navigator_scrollbar.width == 0);
    let preview = state.hits.navigator_preview;
    let tree = state.hits.navigator_rows[0].0;
    assert_eq!(preview_of(&state).scroll, 0);

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    state.compose(160, 40).expect("frame");
    let selected = navigator(&state).selected.clone();
    for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
        scroll_wheel(&mut state, kind, preview.x + 2, preview.y + 5);
        assert_eq!(navigator(&state).selected, selected);
        assert_eq!(preview_of(&state).scroll, 0);
        scroll_wheel(&mut state, kind, tree.x + 2, tree.y);
        assert_eq!(navigator(&state).selected, selected);
    }

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    state.compose(160, 40).expect("frame");
    scroll_wheel(
        &mut state,
        MouseEventKind::ScrollUp,
        preview.x + 2,
        preview.y + 5,
    );
    assert_eq!(
        preview_of(&state).scroll,
        NAVIGATOR_PREVIEW_WHEEL_LINES,
        "with the sheet closed the same wheel scrolls the preview"
    );
    scroll_wheel(&mut state, MouseEventKind::ScrollDown, tree.x + 2, tree.y);
    assert_ne!(
        navigator(&state).selected,
        selected,
        "with the sheet closed the same wheel moves the selection"
    );
}

fn click(state: &mut ClientShellState, column: u16, row: u16) {
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        })]);
    }
}

#[test]
fn clicks_inside_goto_do_nothing_while_the_key_sheet_is_open() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    state.set_pane_surface(surface());
    open(&mut state);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    state.compose(160, 40).expect("frame");
    let search = state.hits.navigator_search;
    let (row, _) = state.hits.navigator_rows[3].clone();
    let popup = state.hits.navigator_popup;
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    state.compose(160, 40).expect("frame");
    let selected = navigator(&state).selected.clone();

    click(&mut state, search.x + 3, search.y);
    assert!(!navigator(&state).search_focused);
    assert!(navigator(&state).keys_open);
    click(&mut state, row.x + 4, row.y);
    assert_eq!(navigator(&state).selected, selected);
    assert!(navigator(&state).keys_open);
    click(&mut state, popup.x + 1, popup.bottom() - 2);
    assert!(navigator(&state).keys_open);

    click(&mut state, popup.right() + 2, popup.y);
    assert!(state.overlay.is_none(), "a click outside still closes goto");
}

fn sheet_has(text: &str, key: &str, word: &str) -> bool {
    text.lines().any(|line| {
        line.match_indices(key).any(|(index, _)| {
            let before = line[..index].chars().next_back();
            let rest = &line[index + key.len()..];
            before.is_none_or(|c| c == ' ' || c == '│')
                && rest.starts_with(' ')
                && rest.trim_start().starts_with(word)
        })
    })
}

#[test]
fn the_key_sheet_lists_every_key_on_a_small_terminal() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    let text = frame_text(&mut state, 80, 24);
    for (key, word) in [
        ("j/k  ↑↓  ctrl+j/k", "move"),
        ("ctrl+d/u", "move"),
        ("h/l  ←→", "prev"),
        ("home  end/G", "first"),
        ("space", "coll"),
        ("enter", "open"),
        ("/  tab", "search"),
        ("a", "agents"),
        ("b/w/i/d", "filt"),
        ("backspace", "clear"),
        ("p", "edit"),
        ("ctrl+o", "new"),
        ("pgup/pgdn", "scroll"),
        ("?  F1", "close"),
        ("esc", "close"),
    ] {
        assert!(sheet_has(&text, key, word), "{key} {word}:\n{text}");
    }

    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    let text = frame_text(&mut state, 80, 24);
    for (key, word) in [
        ("type", "search"),
        ("↑↓  ctrl+j/k  ctrl+n/p", "move"),
        ("enter", "open"),
        ("tab", "tree"),
        ("alt+a", "agents"),
        ("alt+b/w/i/d", "filt"),
        ("pgup/pgdn", "scroll"),
        ("F1", "close"),
        ("esc", "clear"),
    ] {
        assert!(sheet_has(&text, key, word), "{key} {word}:\n{text}");
    }
}

#[test]
fn the_key_sheet_keeps_its_close_keys_when_rows_run_out() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    let text = frame_text(&mut state, 64, 18);
    assert!(sheet_has(&text, "j/k  ↑↓  ctrl+j/k", "move"), "{text}");
    assert!(sheet_has(&text, "?  F1", "close"), "{text}");
    assert!(sheet_has(&text, "esc", "close"), "{text}");
}

#[test]
fn key_sheet_columns_take_the_width_their_content_needs() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    let text = frame_text(&mut state, 98, 15);
    assert!(sheet_has(&text, "↑↓  ctrl+j/k  ctrl+n/p", "move"), "{text}");

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    let text = frame_text(&mut state, 104, 24);
    assert!(
        sheet_has(&text, "h/l  ←→", "previous / next workspace "),
        "{text}"
    );
}

#[test]
fn key_sheet_keys_are_never_clipped_across_terminal_sizes() {
    use unicode_width::UnicodeWidthStr;
    for cols in (60..=200).step_by(4) {
        for rows in 13..=50 {
            let g = render::overlays::navigator_geometry(Rect::new(0, 0, cols, rows))
                .expect("geometry");
            let right = g.preview.map_or(g.tree.right(), |preview| preview.right());
            let width = right - g.tree.x;
            let capacity = usize::from(g.tree.height.saturating_sub(1));
            for search_focused in [true, false] {
                let columns =
                    render::overlays::navigator_key_sheet_columns(width, capacity, search_focused);
                let context = format!("{cols}x{rows} search={search_focused}");
                assert!(!columns.is_empty(), "{context}");
                let last = columns.last().expect("a column");
                assert!(
                    last.x + last.key_width + last.action_width <= width,
                    "{context}: columns overflow the sheet"
                );
                let natural: u16 = columns
                    .iter()
                    .map(|column| {
                        let actions = column
                            .entries
                            .iter()
                            .map(|(_, action)| action.width() as u16)
                            .max()
                            .unwrap_or(0);
                        column.key_width + actions
                    })
                    .sum::<u16>()
                    + 3 * (columns.len() as u16 - 1);
                for column in &columns {
                    let widest = column
                        .entries
                        .iter()
                        .map(|(key, _)| key.width() as u16)
                        .max()
                        .unwrap_or(0);
                    assert!(
                        column.key_width >= widest + 2,
                        "{context}: a key is clipped"
                    );
                    if natural <= width {
                        for (_, action) in &column.entries {
                            assert!(
                                action.width() as u16 <= column.action_width,
                                "{context}: {action} clipped though the columns fit"
                            );
                        }
                    }
                }
                let drawn = columns
                    .iter()
                    .flat_map(|column| column.entries.iter())
                    .filter(|(_, action)| action.contains("close") && !action.contains("pane"))
                    .count();
                assert_eq!(drawn, 2, "{context}: both close entries are drawn");
            }
        }
    }
}

#[test]
fn a_very_short_key_sheet_keeps_both_close_entries() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    let text = frame_text(&mut state, 80, 13);
    assert!(sheet_has(&text, "F1", "close"), "{text}");
    assert!(sheet_has(&text, "esc", "clear"), "{text}");

    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    let text = frame_text(&mut state, 80, 13);
    assert!(sheet_has(&text, "?  F1", "close"), "{text}");
    assert!(sheet_has(&text, "esc", "close"), "{text}");
}

#[test]
fn the_footer_offers_only_esc_while_the_key_sheet_is_open() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    let footer = render::overlays::navigator_geometry(Rect::new(0, 0, 160, 40))
        .expect("geometry")
        .footer;
    assert!(frame_line(&mut state, 160, 40, footer).starts_with("↑↓ move"));
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    assert_eq!(frame_line(&mut state, 160, 40, footer), "esc close keys");
    press(&mut state, KeyCode::F(1), KeyModifiers::NONE);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Char('?'), KeyModifiers::NONE);
    assert_eq!(frame_line(&mut state, 160, 40, footer), "esc close keys");
}

#[test]
fn editing_a_query_selects_the_top_match_not_the_current_pane() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    open(&mut state);
    state.handle_input_bytes(b"zq");
    press(&mut state, KeyCode::Backspace, KeyModifiers::NONE);
    let rows = render::client_navigator_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        navigator(&state),
    );
    assert!(matches!(
        super::super::aggregate_navigation::selected_navigator_target(&rows, navigator(&state)),
        Some(ClientNavigatorTarget::Pane { pane_id, .. }) if pane_id == "pane_2"
    ));
}

#[test]
fn ctrl_x_closes_the_selected_pane_and_keeps_the_overview_open() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_close = false;
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    open(&mut state);
    let outcome = press(&mut state, KeyCode::Char('x'), KeyModifiers::CONTROL);
    let closed = outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(&request.method),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        closed.as_slice(),
        [crate::api::schema::Method::PaneClose(target)] if target.pane_id == "pane_1"
    ));
    assert!(navigator(&state).query.is_empty());
    assert!(matches!(
        &navigator(&state).selected,
        Some(ClientNavigatorTarget::Pane { pane_id, .. }) if pane_id == "pane_2"
    ));
}

#[test]
fn x_in_tree_mode_ignores_workspace_rows() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_close = false;
    state.navigator_agents_only = false;
    state.set_snapshot(Box::new(two_workspace_snapshot()));
    open(&mut state);
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE);
    press(&mut state, KeyCode::Home, KeyModifiers::NONE);
    let outcome = press(&mut state, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(outcome
        .actions
        .iter()
        .all(|action| !matches!(action, ClientShellAction::Endpoint { .. })));
}
