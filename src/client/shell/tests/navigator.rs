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
    assert_eq!(pane_rows(&rows)[0].label, "Code review");

    let rows = rows_for(&mut state, "review");
    assert_eq!(
        rows[0].label, "Code review",
        "flat query rows use the same label"
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
    assert_eq!(pane_rows(&rows_for(&mut state, ""))[0].label, "renamed");

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    assert_eq!(pane_rows(&rows_for(&mut state, ""))[0].label, "pane 1");
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
    assert_eq!(labels(&rows), vec!["beta", "1", "alpha-runner"]);

    let rows = rows_with(&mut state, "", false);
    assert_eq!(
        labels(&rows),
        vec!["beta", "1", "alpha-runner", "alpha", "1", "zeta"]
    );
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

use ratatui::layout::Rect;

#[test]
fn the_navigator_fills_the_screen_and_splits_only_when_wide() {
    let narrow = overlays::navigator_geometry(Rect::new(0, 0, 80, 30)).expect("geometry");
    assert_eq!(narrow.popup, Rect::new(1, 1, 78, 28));
    assert!(narrow.preview.is_none());
    assert_eq!(narrow.tree.width, narrow.inner.width);

    let wide = overlays::navigator_geometry(Rect::new(0, 0, 160, 40)).expect("geometry");
    let preview = wide.preview.expect("preview column");
    assert_eq!(wide.tree.width, 70, "45% of 156 inner columns");
    assert_eq!(preview.x, wide.tree.right() + 1);
    assert_eq!(preview.right(), wide.inner.right());
    assert_eq!(
        overlays::navigator_preview_capacity(160, 40),
        Some(preview.height - overlays::NAVIGATOR_PREVIEW_HEADER_ROWS)
    );
    assert_eq!(overlays::navigator_preview_capacity(80, 30), None);
}

fn frame_text(state: &mut ClientShellState, cols: u16, rows: u16) -> String {
    let frame = state.compose(cols, rows).expect("frame");
    frame_rows(&frame).join("\n")
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
    assert!(text.contains("Code review · claude · working"));

    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.preview = Some(ClientNavigatorPreview {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_1".into(),
            lines: vec!["$ cargo test".into(), "ok".into()],
            error: None,
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
    assert_eq!(reads[0].format, crate::api::schema::ReadFormat::Text);
    assert_eq!(
        reads[0].lines,
        overlays::navigator_preview_capacity(160, 40).map(u32::from)
    );

    let mut again = ClientShellInput::default();
    state.tick_navigator(now + Duration::from_secs(5), &mut again);
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
    assert!(repaint);
    assert_eq!(
        navigator(&state).preview.as_ref().unwrap().lines,
        vec!["$ cargo test", "ok"]
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
