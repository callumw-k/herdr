use std::path::PathBuf;

use crate::api::schema::{
    EventData, EventEnvelope, EventKind, ResponseResult, TabCreateParams, TabListParams,
    TabMoveParams, TabRenameParams, TabTarget,
};
use crate::app::{App, Mode};

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_tab_list(&mut self, id: String, params: TabListParams) -> String {
        let tabs = if let Some(workspace_id) = params.workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            let Some(_) = self.state.workspaces.get(ws_idx) else {
                return workspace_not_found(id, &workspace_id);
            };
            self.tab_list_info(ws_idx)
        } else {
            let mut tabs = Vec::new();
            for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
                for tab_idx in 0..ws.tabs.len() {
                    if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                        tabs.push(tab);
                    }
                }
            }
            tabs
        };

        encode_success(id, ResponseResult::TabList { tabs })
    }

    pub(super) fn handle_tab_get(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_create(&mut self, id: String, params: TabCreateParams) -> String {
        let TabCreateParams {
            workspace_id,
            cwd,
            focus,
            label,
            env,
            parent_tab_id,
            caller_pane_id,
        } = params;
        let requested_ws = match workspace_id.as_deref() {
            Some(workspace_id) => match self.parse_workspace_id(workspace_id) {
                Some(ws_idx) => Some(ws_idx),
                None => return workspace_not_found(id, workspace_id),
            },
            None => None,
        };
        let parent = if let Some(parent_tab_id) = parent_tab_id.as_deref() {
            let Some((parent_ws, parent_idx)) = self.parse_tab_id(parent_tab_id) else {
                return tab_not_found(id, parent_tab_id);
            };
            if requested_ws.is_some_and(|ws_idx| ws_idx != parent_ws) {
                return encode_error(
                    id,
                    "parent_tab_not_in_workspace",
                    format!("tab {parent_tab_id} is not in the requested workspace"),
                );
            }
            Some((parent_ws, parent_idx))
        } else {
            caller_pane_id
                .as_deref()
                .and_then(|pane_id| self.parse_pane_id(pane_id))
                .and_then(|(ws_idx, pane_id)| {
                    let tab_idx = self
                        .state
                        .workspaces
                        .get(ws_idx)?
                        .presenting_tab_index(pane_id)?;
                    Some((ws_idx, tab_idx))
                })
                .filter(|(ws_idx, _)| requested_ws.is_none_or(|requested| requested == *ws_idx))
        };
        let ws_idx = if let Some(ws_idx) = requested_ws.or(parent.map(|(ws_idx, _)| ws_idx)) {
            ws_idx
        } else if let Some(active) = self.state.active {
            active
        } else {
            return encode_error(id, "workspace_not_found", "no active workspace");
        };
        let parent_number = parent.and_then(|(parent_ws, parent_idx)| {
            self.state
                .workspaces
                .get(parent_ws)?
                .public_tab_number(parent_idx)
        });
        let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| {
            self.resolve_new_terminal_cwd(self.focused_pane_cwd_in_workspace(ws_idx))
        });
        let (rows, cols) = self.state.new_pane_size(crate::ui::NewPanePlacement::Alone);
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;
        let extra_env = match super::env::normalize_launch_env(env) {
            Ok(env) => env,
            Err((code, message)) => return encode_error(id, &code, message),
        };
        let result = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| std::io::Error::other("workspace disappeared"))
            .and_then(|ws| {
                ws.create_tab(
                    rows,
                    cols,
                    cwd,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                    extra_env,
                )
            });
        match result {
            Ok((created_idx, terminal, runtime)) => {
                let tab_idx = match parent_number {
                    Some(parent_number) => self.state.workspaces[ws_idx]
                        .attach_tab_to_parent(created_idx, parent_number),
                    None => created_idx,
                };
                self.terminal_runtimes.insert(terminal.id.clone(), runtime);
                self.state.terminals.insert(terminal.id.clone(), terminal);
                self.state.remove_alias_shadowed_by_new_pane(
                    self.state.workspaces[ws_idx].tabs[tab_idx].root_pane,
                );
                if let Some(label) = label {
                    let workspace_id = self.state.workspaces[ws_idx].id.clone();
                    let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
                        crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
                    });
                    if let Some(tab) = self
                        .state
                        .workspaces
                        .get_mut(ws_idx)
                        .and_then(|ws| ws.tabs.get_mut(tab_idx))
                    {
                        tab.set_custom_name(label);
                        crate::logging::tab_renamed(&workspace_id, &tab_id);
                    }
                }
                if focus {
                    self.state.switch_workspace_tab(ws_idx, tab_idx);
                    self.state.mode = Mode::Terminal;
                }
                self.schedule_session_save();
                self.emit_tab_created_events(ws_idx, tab_idx);
                if tab_idx != created_idx {
                    if let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) {
                        let tabs = self.tab_list_info(ws_idx);
                        self.emit_tab_moved_event(ws_idx, tab_id, tab_idx, tabs);
                    }
                }
                encode_success(
                    id,
                    self.tab_created_result(ws_idx, tab_idx)
                        .expect("new tab should produce a complete create response"),
                )
            }
            Err(err) => encode_error(id, "tab_create_failed", err.to_string()),
        }
    }

    pub(super) fn handle_tab_focus(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        self.state.switch_workspace_tab(ws_idx, tab_idx);
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_rename(&mut self, id: String, params: TabRenameParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
            crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
        });
        let Some(tab) = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return tab_not_found(id, &params.tab_id);
        };
        tab.set_custom_name(params.label.clone());
        crate::logging::tab_renamed(&workspace_id, &tab_id);
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabRenamed,
            data: EventData::TabRenamed {
                tab_id: self.public_tab_id(ws_idx, tab_idx).unwrap(),
                workspace_id: self.public_workspace_id(ws_idx),
                label: params.label,
            },
        });
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_move(&mut self, id: String, params: TabMoveParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &params.tab_id);
        };
        if params.insert_index > ws.tabs.len() {
            return encode_error(
                id,
                "tab_move_failed",
                format!("insert_index {} is out of bounds", params.insert_index),
            );
        }

        let tab_id = self
            .public_tab_id(ws_idx, tab_idx)
            .unwrap_or_else(|| crate::workspace::public_tab_id_for_number(&ws.id, tab_idx + 1));
        let insert_index = params.insert_index;
        let moved = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .is_some_and(|ws| ws.move_tab(tab_idx, insert_index));
        let tabs = self.tab_list_info(ws_idx);
        if moved {
            self.schedule_session_save();
            self.emit_tab_moved_event(ws_idx, tab_id, insert_index, tabs.clone());
        }

        encode_success(id, ResponseResult::TabList { tabs })
    }

    fn emit_tab_moved_event(
        &mut self,
        ws_idx: usize,
        tab_id: String,
        insert_index: usize,
        tabs: Vec<crate::api::schema::TabInfo>,
    ) {
        let workspace_id = self.public_workspace_id(ws_idx);
        self.emit_event(EventEnvelope {
            event: EventKind::TabMoved,
            data: EventData::TabMoved {
                tab_id,
                workspace_id,
                insert_index,
                tabs,
            },
        });
    }

    pub(super) fn handle_tab_close(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let workspace_id = self.public_workspace_id(ws_idx);
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let closes_workspace = ws.tabs.len() <= 1;
        let terminal_ids = self.state.terminal_ids_for_tab(ws_idx, tab_idx);
        let pane_ids = ws
            .tabs
            .get(tab_idx)
            .map(|tab| tab.layout.pane_ids())
            .unwrap_or_default();

        if closes_workspace {
            if let Err(response) = self.require_restored_group_close_ready(
                &id,
                &self.state.workspace_close_indices(ws_idx),
            ) {
                return response;
            }
            if self.state.confirm_implicit_worktree_group_close(ws_idx) {
                return encode_error(
                    id,
                    "confirmation_required",
                    "closing this tab would close a worktree group",
                );
            }
            let workspace = self.workspace_info(ws_idx);
            self.state.selected = ws_idx;
            self.state.close_selected_workspace();
            self.state.remove_plugin_pane_records(pane_ids);
            self.shutdown_detached_terminal_runtimes();
            self.emit_event(EventEnvelope {
                event: EventKind::TabClosed,
                data: EventData::TabClosed {
                    tab_id,
                    workspace_id: workspace_id.clone(),
                },
            });
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id,
                    workspace: Some(workspace),
                },
            });
            return encode_success(id, ResponseResult::Ok {});
        }

        let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        if !ws.close_tab(tab_idx) {
            return encode_error(
                id,
                "tab_close_failed",
                format!("tab {} could not be closed", target.tab_id),
            );
        }
        self.state.remove_plugin_pane_records(pane_ids);
        self.state.remove_unattached_terminal_ids(terminal_ids);
        self.shutdown_detached_terminal_runtimes();
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id,
                workspace_id,
            },
        });

        encode_success(id, ResponseResult::Ok {})
    }

    fn tab_list_info(&self, ws_idx: usize) -> Vec<crate::api::schema::TabInfo> {
        self.state
            .workspaces
            .get(ws_idx)
            .map(|ws| {
                (0..ws.tabs.len())
                    .filter_map(|idx| self.tab_info(ws_idx, idx))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn workspace_not_found(id: String, workspace_id: &str) -> String {
    encode_error(
        id,
        "workspace_not_found",
        format!("workspace {workspace_id} not found"),
    )
}

fn tab_not_found(id: String, tab_id: &str) -> String {
    encode_error(id, "tab_not_found", format!("tab {tab_id} not found"))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};
    use super::*;
    use crate::{
        api::schema::SuccessResponse,
        config::{Config, ShellModeConfig},
        workspace::Workspace,
    };

    #[test]
    fn api_tab_close_last_tab_closes_workspace_and_emits_both_events() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        app.state.workspaces = vec![Workspace::test_new("tabs")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let workspace_id = app.public_workspace_id(0);

        let response = app.handle_tab_close(
            "req".into(),
            TabTarget {
                tab_id: tab_id.clone(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(success.result, ResponseResult::Ok {});
        assert!(app.state.workspaces.is_empty());
        assert!(app.state.active.is_none());
        let events = event_hub.events_after(0);
        assert_eq!(
            events
                .iter()
                .map(|(_, event)| event.event)
                .collect::<Vec<_>>(),
            [EventKind::TabClosed, EventKind::WorkspaceClosed]
        );
        assert!(matches!(
            &events[0].1.data,
            EventData::TabClosed {
                tab_id: closed_tab_id,
                workspace_id: closed_workspace_id,
            } if closed_tab_id == &tab_id && closed_workspace_id == &workspace_id
        ));
        assert!(matches!(
            &events[1].1.data,
            EventData::WorkspaceClosed {
                workspace_id: closed_workspace_id,
                workspace: Some(workspace),
            } if closed_workspace_id == &workspace_id
                && workspace.workspace_id == workspace_id
        ));
    }

    #[test]
    fn api_tab_move_reorders_tabs_in_target_workspace() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        let mut workspace = Workspace::test_new("tabs");
        workspace.test_add_tab(Some("two"));
        workspace.test_add_tab(Some("three"));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        let moved_root = app.state.workspaces[0].tabs[0].root_pane;
        let moved_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_tab_move(
            "req".into(),
            TabMoveParams {
                tab_id: moved_id.clone(),
                insert_index: 3,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabList { tabs } = success.result else {
            panic!("expected tab list");
        };
        assert_eq!(app.state.workspaces[0].tabs[2].root_pane, moved_root);
        assert_eq!(tabs[2].tab_id, app.public_tab_id(0, 2).unwrap());
        let events = event_hub.events_after(0);
        assert!(events.iter().any(|(_, event)| {
            matches!(
                &event.data,
                EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index: 3,
                    tabs,
                } if tab_id == &moved_id
                    && workspace_id == &app.public_workspace_id(0)
                    && tabs[2].tab_id == moved_id
            )
        }));
    }

    #[tokio::test]
    async fn tab_create_follows_cached_focused_pane_cwd_without_runtime() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub,
        );
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        let workspace = Workspace::test_new("tabs");
        let focused_pane = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        let cached_cwd = std::env::temp_dir();
        let terminal_id = app.state.workspaces[0]
            .terminal_id(focused_pane)
            .cloned()
            .unwrap();
        app.state.terminals.get_mut(&terminal_id).unwrap().cwd = cached_cwd.clone();

        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: None,
                env: Default::default(),
                parent_tab_id: None,
                caller_pane_id: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::TabCreated { .. }));
        let created = &app.state.workspaces[0].tabs[1];
        let created_terminal_id = created.terminal_id(created.root_pane).unwrap();
        let created_cwd = &app.state.terminals.get(created_terminal_id).unwrap().cwd;
        assert_eq!(
            crate::worktree::canonical_or_original(created_cwd),
            crate::worktree::canonical_or_original(&cached_cwd)
        );
        shutdown_test_runtimes(&mut app);
    }

    fn parent_test_app(
        workspaces: Vec<Workspace>,
    ) -> (
        App,
        tokio::sync::mpsc::UnboundedSender<crate::api::ApiRequestMessage>,
    ) {
        let (api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        app.state.workspaces = workspaces;
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        (app, api_tx)
    }

    fn create_tab_with_parent(
        app: &mut App,
        workspace_id: Option<String>,
        parent_tab_id: Option<String>,
        caller_pane_id: Option<String>,
    ) -> String {
        app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id,
                cwd: None,
                focus: false,
                label: None,
                env: Default::default(),
                parent_tab_id,
                caller_pane_id,
            },
        )
    }

    fn created_tab(response: &str) -> crate::api::schema::TabInfo {
        let success: SuccessResponse = serde_json::from_str(response).unwrap();
        let ResponseResult::TabCreated { tab, .. } = success.result else {
            panic!("expected tab_created, got {response}");
        };
        tab
    }

    #[tokio::test]
    async fn tab_create_parents_to_the_callers_tab_not_the_active_one() {
        let mut workspace = Workspace::test_new("one");
        let caller_tab = workspace.test_add_tab(Some("caller"));
        let caller_pane = workspace.tabs[caller_tab].root_pane;
        let (mut app, _api_tx) = parent_test_app(vec![workspace]);
        let caller_pane_id = app.public_pane_id(0, caller_pane).unwrap();

        let tab = created_tab(&create_tab_with_parent(
            &mut app,
            None,
            None,
            Some(caller_pane_id),
        ));

        assert_eq!(tab.parent_tab_id, app.public_tab_id(0, caller_tab));
        assert_eq!(
            app.state.workspaces[0].tabs[2].parent,
            Some(app.state.workspaces[0].tabs[1].number)
        );
        assert!(!app
            .event_hub
            .events_after(0)
            .iter()
            .any(|(_, event)| event.event == EventKind::TabMoved));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn tab_create_announces_a_child_placed_mid_list() {
        let mut workspace = Workspace::test_new("one");
        let caller_tab = workspace.test_add_tab(Some("caller"));
        workspace.test_add_tab(Some("after"));
        let caller_pane = workspace.tabs[caller_tab].root_pane;
        let (mut app, _api_tx) = parent_test_app(vec![workspace]);
        let caller_pane_id = app.public_pane_id(0, caller_pane).unwrap();

        let tab = created_tab(&create_tab_with_parent(
            &mut app,
            None,
            None,
            Some(caller_pane_id),
        ));

        let events = app.event_hub.events_after(0);
        let kinds = events
            .iter()
            .map(|(_, event)| event.event)
            .collect::<Vec<_>>();
        let created = kinds
            .iter()
            .position(|kind| *kind == EventKind::TabCreated)
            .expect("tab.created event");
        let moved = kinds
            .iter()
            .position(|kind| *kind == EventKind::TabMoved)
            .expect("tab.moved event");
        assert!(created < moved, "{kinds:?}");
        assert!(matches!(
            &events[moved].1.data,
            EventData::TabMoved {
                tab_id,
                workspace_id,
                insert_index: 2,
                tabs,
            } if tab_id == &tab.tab_id
                && workspace_id == &app.public_workspace_id(0)
                && tabs[2].tab_id == tab.tab_id
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn tab_create_without_workspace_lands_in_the_callers_workspace() {
        let first = Workspace::test_new("first");
        let second = Workspace::test_new("second");
        let caller_pane = second.tabs[0].root_pane;
        let (mut app, _api_tx) = parent_test_app(vec![first, second]);
        let caller_pane_id = app.public_pane_id(1, caller_pane).unwrap();

        let tab = created_tab(&create_tab_with_parent(
            &mut app,
            None,
            None,
            Some(caller_pane_id),
        ));

        assert_eq!(tab.workspace_id, app.public_workspace_id(1));
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.workspaces[1].tabs.len(), 2);
        shutdown_test_runtimes(&mut app);
    }

    #[test]
    fn tab_create_rejects_a_missing_or_foreign_parent() {
        let (mut app, _api_tx) = parent_test_app(vec![
            Workspace::test_new("first"),
            Workspace::test_new("second"),
        ]);
        let first_workspace = app.public_workspace_id(0);
        let foreign_parent = app.public_tab_id(1, 0).unwrap();

        let foreign =
            create_tab_with_parent(&mut app, Some(first_workspace), Some(foreign_parent), None);
        let missing = create_tab_with_parent(&mut app, None, Some("w_missing:t9".into()), None);

        assert!(foreign.contains("parent_tab_not_in_workspace"), "{foreign}");
        assert!(missing.contains("tab_not_found"), "{missing}");
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.workspaces[1].tabs.len(), 1);
    }

    #[tokio::test]
    async fn tab_create_in_another_workspace_ignores_the_caller() {
        let first = Workspace::test_new("first");
        let second = Workspace::test_new("second");
        let caller_pane = second.tabs[0].root_pane;
        let (mut app, _api_tx) = parent_test_app(vec![first, second]);
        let caller_pane_id = app.public_pane_id(1, caller_pane).unwrap();
        let first_workspace = app.public_workspace_id(0);

        let tab = created_tab(&create_tab_with_parent(
            &mut app,
            Some(first_workspace.clone()),
            None,
            Some(caller_pane_id),
        ));

        assert_eq!(tab.workspace_id, first_workspace);
        assert_eq!(tab.parent_tab_id, None);
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn tab_create_ignores_an_unknown_caller_pane() {
        let (mut app, _api_tx) = parent_test_app(vec![Workspace::test_new("one")]);

        let tab = created_tab(&create_tab_with_parent(
            &mut app,
            None,
            None,
            Some("w_missing:p9".into()),
        ));

        assert_eq!(tab.parent_tab_id, None);
        assert_eq!(tab.workspace_id, app.public_workspace_id(0));
        shutdown_test_runtimes(&mut app);
    }
}
