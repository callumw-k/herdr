use super::*;

pub(super) fn family_tab(tab_id: &str, label: &str, parent: Option<&str>) -> ClientShellTab {
    ClientShellTab {
        tab_id: tab_id.into(),
        workspace_id: "ws_1".into(),
        number: 0,
        label: label.into(),
        custom_label: true,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
        parent_tab_id: parent.map(str::to_owned),
    }
}

#[test]
fn tab_display_label_prefixes_a_present_parent_only() {
    let tabs = vec![
        family_tab("tab_1", "web", None),
        family_tab("tab_2", "api", Some("tab_1")),
        family_tab("tab_3", "orphan", Some("tab_9")),
    ];

    assert_eq!(render::tab_display_label(&tabs[0], &tabs), "web");
    assert_eq!(render::tab_display_label(&tabs[1], &tabs), "web › api");
    assert_eq!(render::tab_display_label(&tabs[2], &tabs), "orphan");
}

fn family_agent(pane_id: &str, tab_id: &str) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: tab_id.into(),
        name: None,
        display_agent: Some("Claude".into()),
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Idle,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
    }
}

fn family_snapshot(
    tabs: Vec<ClientShellTab>,
    agents: Vec<ClientShellAgent>,
) -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    snapshot.tabs = tabs;
    snapshot.agents = agents;
    snapshot
}

fn grouped_config() -> ClientShellConfig {
    let mut config = ClientShellConfig::from_config(&crate::config::Config::default());
    config.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
    config
}

fn shape(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) -> Vec<(String, usize, Vec<bool>)> {
    super::agent_sidebar::agent_rows(snapshot, config, None)
        .into_iter()
        .map(|row| (row.pane_id, row.depth, row.continues))
        .collect()
}

#[test]
fn grouped_order_walks_the_tab_tree_after_a_manual_move() {
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_2", "child", Some("tab_1")),
            family_tab("tab_1", "parent", None),
            family_tab("tab_3", "other", None),
        ],
        vec![
            family_agent("pane_2", "tab_2"),
            family_agent("pane_1", "tab_1"),
            family_agent("pane_3", "tab_3"),
        ],
    );

    assert_eq!(
        super::agent_sidebar::ordered_agent_pane_ids(
            &snapshot,
            crate::config::AgentPanelSortConfig::Spaces
        ),
        vec!["pane_1", "pane_2", "pane_3"]
    );
}

#[test]
fn grouped_order_keeps_agents_of_tabs_in_a_parent_cycle() {
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_1", "loop a", Some("tab_2")),
            family_tab("tab_2", "loop b", Some("tab_1")),
            family_tab("tab_3", "root", None),
        ],
        vec![
            family_agent("pane_1", "tab_1"),
            family_agent("pane_2", "tab_2"),
            family_agent("pane_3", "tab_3"),
        ],
    );

    assert_eq!(
        super::agent_sidebar::ordered_agent_pane_ids(
            &snapshot,
            crate::config::AgentPanelSortConfig::Spaces
        ),
        vec!["pane_3", "pane_1", "pane_2"]
    );
}

#[test]
fn depth_skips_a_parent_tab_without_agents() {
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_1", "shell", None),
            family_tab("tab_2", "lead", Some("tab_1")),
            family_tab("tab_3", "worker", Some("tab_2")),
        ],
        vec![
            family_agent("pane_2", "tab_2"),
            family_agent("pane_3", "tab_3"),
        ],
    );

    assert_eq!(
        shape(&snapshot, &grouped_config()),
        vec![
            ("pane_2".into(), 0, vec![]),
            ("pane_3".into(), 1, vec![false])
        ]
    );
}

#[test]
fn sibling_continuation_ignores_tabs_without_agent_rows() {
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_1", "lead", None),
            family_tab("tab_2", "worker", Some("tab_1")),
            family_tab("tab_3", "shell", Some("tab_1")),
            family_tab("tab_4", "reviewer", Some("tab_1")),
        ],
        vec![
            family_agent("pane_1", "tab_1"),
            family_agent("pane_2", "tab_2"),
            family_agent("pane_4", "tab_4"),
        ],
    );

    assert_eq!(
        shape(&snapshot, &grouped_config()),
        vec![
            ("pane_1".into(), 0, vec![]),
            ("pane_2".into(), 1, vec![true]),
            ("pane_4".into(), 1, vec![false]),
        ]
    );
}

#[test]
fn ancestor_columns_continue_through_nested_rows() {
    use super::agent_sidebar::tree_prefix;
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_1", "a", None),
            family_tab("tab_2", "b", Some("tab_1")),
            family_tab("tab_3", "c", Some("tab_2")),
            family_tab("tab_4", "d", Some("tab_1")),
        ],
        vec![
            family_agent("pane_1", "tab_1"),
            family_agent("pane_2", "tab_2"),
            family_agent("pane_3", "tab_3"),
            family_agent("pane_4", "tab_4"),
        ],
    );

    let rows = shape(&snapshot, &grouped_config());

    assert_eq!(
        rows,
        vec![
            ("pane_1".into(), 0, vec![]),
            ("pane_2".into(), 1, vec![true]),
            ("pane_3".into(), 2, vec![true, false]),
            ("pane_4".into(), 1, vec![false]),
        ]
    );
    let c = &rows[2].2;
    assert_eq!(tree_prefix(c, true), "  │  └─ ");
    assert_eq!(tree_prefix(c, false), "  │     ");
    let b = &rows[1].2;
    assert_eq!(tree_prefix(b, false), "  │  ");
}

#[test]
fn priority_sort_stays_flat() {
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_1", "lead", None),
            family_tab("tab_2", "worker", Some("tab_1")),
        ],
        vec![
            family_agent("pane_1", "tab_1"),
            family_agent("pane_2", "tab_2"),
        ],
    );
    let mut config = grouped_config();
    config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;

    assert!(shape(&snapshot, &config)
        .iter()
        .all(|(_, depth, continues)| *depth == 0 && continues.is_empty()));
}

#[test]
fn tree_prefix_keeps_continuation_rows_aligned() {
    use super::agent_sidebar::tree_prefix;
    assert_eq!(tree_prefix(&[], true), "  ");
    assert_eq!(tree_prefix(&[], false), "  ");
    assert_eq!(tree_prefix(&[true], true), "  ├─ ");
    assert_eq!(tree_prefix(&[false], true), "  └─ ");
    assert_eq!(tree_prefix(&[true], false), "  │  ");
    assert_eq!(tree_prefix(&[false], false), "     ");
    assert_eq!(tree_prefix(&[false, false], true), "     └─ ");
}

#[test]
fn agent_focus_targets_follow_the_sidebar_tree_order() {
    let snapshot = family_snapshot(
        vec![
            family_tab("tab_2", "child", Some("tab_1")),
            family_tab("tab_1", "parent", None),
            family_tab("tab_3", "other", None),
        ],
        vec![
            family_agent("pane_2", "tab_2"),
            family_agent("pane_1", "tab_1"),
            family_agent("pane_3", "tab_3"),
        ],
    );
    let config = grouped_config();
    let sidebar = super::agent_sidebar::agent_rows(&snapshot, &config, None)
        .into_iter()
        .map(|row| row.pane_id)
        .collect::<Vec<_>>();
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(snapshot));
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, None);

    let targets = aggregate_navigation::online_agent_targets(
        &state.endpoints,
        &state.active_endpoint_id,
        crate::config::AgentPanelSortConfig::Spaces,
    )
    .into_iter()
    .map(|target| target.pane_id)
    .collect::<Vec<_>>();

    assert_eq!(sidebar, ["pane_1", "pane_2", "pane_3"]);
    assert_eq!(targets, sidebar);
}
