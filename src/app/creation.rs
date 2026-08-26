use std::path::PathBuf;

use crate::api::schema::{EventData, EventEnvelope, EventKind};
use crate::layout::PaneId;
#[cfg(test)]
use tracing::error;

use super::{
    api_helpers::{pane_agent_status, tab_attention_priority},
    App, Mode,
};
use crate::{config::NewTerminalCwdConfig, workspace::Workspace};

pub(crate) fn resolve_new_terminal_cwd(
    policy: &NewTerminalCwdConfig,
    follow_cwd: Option<PathBuf>,
) -> PathBuf {
    match policy {
        NewTerminalCwdConfig::Follow => follow_cwd
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/")),
        NewTerminalCwdConfig::Home => std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/")),
        NewTerminalCwdConfig::Current => {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
        }
        NewTerminalCwdConfig::Path(path) => crate::worktree::expand_tilde_path(path),
    }
}

pub(super) fn launch_cwd_for_terminal(
    terminal_id: &crate::terminal::TerminalId,
    terminals: &std::collections::HashMap<
        crate::terminal::TerminalId,
        crate::terminal::TerminalState,
    >,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
) -> Option<PathBuf> {
    terminal_runtimes
        .get(terminal_id)
        .and_then(|runtime| runtime.follow_cwd())
        .or_else(|| {
            terminals
                .get(terminal_id)
                .map(|terminal| terminal.cwd.clone())
        })
}

impl App {
    pub(super) fn seed_cwd_from_workspace(&self, ws_idx: usize) -> Option<PathBuf> {
        self.state
            .workspaces
            .get(ws_idx)?
            .resolved_identity_cwd_from(&self.state.terminals, &self.terminal_runtimes)
    }

    pub(super) fn launch_cwd_for_pane_in_workspace(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<PathBuf> {
        let workspace = self.state.workspaces.get(ws_idx)?;
        let tab = workspace
            .tabs
            .get(workspace.find_tab_index_for_pane(pane_id)?)?;
        launch_cwd_for_terminal(
            tab.terminal_id(pane_id)?,
            &self.state.terminals,
            &self.terminal_runtimes,
        )
    }

    pub(super) fn focused_pane_cwd_in_workspace(&self, ws_idx: usize) -> Option<PathBuf> {
        let pane_id = self.state.workspaces.get(ws_idx)?.focused_pane_id()?;
        self.launch_cwd_for_pane_in_workspace(ws_idx, pane_id)
    }

    pub(super) fn resolve_new_terminal_cwd(&self, follow_cwd: Option<PathBuf>) -> PathBuf {
        resolve_new_terminal_cwd(&self.state.new_terminal_cwd, follow_cwd)
    }

    pub(super) fn workspace_creation_source(&self) -> Option<usize> {
        if self.state.mode == Mode::Navigate
            && self.state.workspaces.get(self.state.selected).is_some()
        {
            return Some(self.state.selected);
        }

        self.state.active.or_else(|| {
            self.state
                .workspaces
                .get(self.state.selected)
                .map(|_| self.state.selected)
        })
    }

    pub(super) fn begin_tui_workspace_create(&mut self, request_id: &'static str) {
        if self.state.prompt_new_workspace_name {
            let follow_cwd = self.workspace_creation_source().and_then(|ws_idx| {
                self.focused_pane_cwd_in_workspace(ws_idx)
                    .or_else(|| self.seed_cwd_from_workspace(ws_idx))
            });
            let cwd = self.resolve_new_terminal_cwd(follow_cwd);
            super::input::open_new_workspace_dialog(&mut self.state, cwd);
            return;
        }

        self.runtime_workspace_create(
            request_id,
            crate::api::schema::WorkspaceCreateParams {
                cwd: None,
                path: None,
                focus: true,
                label: None,
                env: Default::default(),
            },
        );
        self.state.mode = if self.state.active.is_some() {
            Mode::Terminal
        } else {
            Mode::Navigate
        };
    }

    /// Create a workspace with a real PTY (needs event_tx).
    #[cfg(test)]
    pub(crate) fn create_workspace(&mut self) {
        let follow_cwd = self.workspace_creation_source().and_then(|ws_idx| {
            self.focused_pane_cwd_in_workspace(ws_idx)
                .or_else(|| self.seed_cwd_from_workspace(ws_idx))
        });
        let initial_cwd = self.resolve_new_terminal_cwd(follow_cwd);
        if let Err(e) = self.create_workspace_with_events(initial_cwd, true) {
            error!(err = %e, "failed to create workspace");
            self.state.mode = Mode::Navigate;
        }
    }

    #[cfg(test)]
    pub(crate) fn create_tab(&mut self) {
        let custom_name = self.state.requested_new_tab_name.take();
        let active_before = self.state.active;
        let follow_cwd = self.state.active.and_then(|ws_idx| {
            self.focused_pane_cwd_in_workspace(ws_idx)
                .or_else(|| self.seed_cwd_from_workspace(ws_idx))
        });
        let initial_cwd = self.resolve_new_terminal_cwd(follow_cwd);
        match self.create_tab_with_options(initial_cwd, true) {
            Ok(created_idx) => {
                let created_workspace = active_before.is_none();
                let ws_idx = if created_workspace {
                    Some(created_idx)
                } else {
                    self.state.active
                };
                let tab_idx = if created_workspace { 0 } else { created_idx };
                if let Some(name) = custom_name {
                    if let Some(ws) =
                        ws_idx.and_then(|ws_idx| self.state.workspaces.get_mut(ws_idx))
                    {
                        if let Some(tab) = ws.tabs.get_mut(tab_idx) {
                            tab.set_custom_name(name);
                        }
                        self.schedule_session_save();
                    }
                }
                if let Some(ws_idx) = ws_idx {
                    if created_workspace {
                        self.emit_workspace_open_events(ws_idx);
                    } else {
                        self.emit_tab_created_events(ws_idx, tab_idx);
                    }
                }
            }
            Err(e) => {
                error!(err = %e, "failed to create tab");
            }
        }
    }

    #[cfg(test)]
    pub(super) fn create_tab_with_options(
        &mut self,
        initial_cwd: PathBuf,
        focus: bool,
    ) -> std::io::Result<usize> {
        let Some(ws_idx) = self.state.active else {
            return self.create_workspace_with_options(initial_cwd, focus);
        };
        let (rows, cols) = self.state.estimate_pane_size();
        let ws = &mut self.state.workspaces[ws_idx];
        let (idx, terminal, runtime) = ws.create_tab(
            rows,
            cols,
            initial_cwd,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
            crate::pane::PaneShellConfig::new(&self.state.default_shell, self.state.shell_mode),
            Vec::new(),
        )?;
        let root_pane = ws.tabs[idx].root_pane;
        self.terminal_runtimes.insert(terminal.id.clone(), runtime);
        self.state.terminals.insert(terminal.id.clone(), terminal);
        self.state.remove_alias_shadowed_by_new_pane(root_pane);
        if focus {
            self.state.switch_workspace_tab(ws_idx, idx);
            self.state.mode = Mode::Terminal;
        }
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        let tab_id = self
            .public_tab_id(ws_idx, idx)
            .unwrap_or_else(|| crate::workspace::public_tab_id_for_number(&workspace_id, idx + 1));
        let root_pane = self.state.workspaces[ws_idx].tabs[idx].root_pane.raw();
        crate::logging::tab_created(&workspace_id, &tab_id, root_pane);
        self.schedule_session_save();
        Ok(idx)
    }

    pub(crate) fn create_workspace_with_options(
        &mut self,
        initial_cwd: PathBuf,
        focus: bool,
    ) -> std::io::Result<usize> {
        self.create_workspace_with_launch_env(initial_cwd, focus, Vec::new())
    }

    #[cfg(test)]
    pub(crate) fn create_workspace_with_events(
        &mut self,
        initial_cwd: PathBuf,
        focus: bool,
    ) -> std::io::Result<()> {
        let ws_idx = self.create_workspace_with_options(initial_cwd, focus)?;
        self.emit_workspace_open_events(ws_idx);
        Ok(())
    }

    pub(crate) fn create_workspace_with_launch_env(
        &mut self,
        initial_cwd: PathBuf,
        focus: bool,
        extra_env: Vec<(String, String)>,
    ) -> std::io::Result<usize> {
        let (rows, cols) = self.state.estimate_pane_size();
        let (ws, terminal, runtime) = Workspace::new_with_extra_env(
            initial_cwd,
            rows,
            cols,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
            crate::pane::PaneShellConfig::new(&self.state.default_shell, self.state.shell_mode),
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
            extra_env,
        )?;
        self.terminal_runtimes.insert(terminal.id.clone(), runtime);
        self.state.terminals.insert(terminal.id.clone(), terminal);
        self.state.workspaces.push(ws);
        let idx = self.state.workspaces.len() - 1;
        self.state
            .remove_alias_shadowed_by_new_pane(self.state.workspaces[idx].tabs[0].root_pane);
        let workspace_id = self.state.workspaces[idx].id.clone();
        let root_pane = self.state.workspaces[idx].tabs[0].root_pane.raw();
        crate::logging::workspace_created(&workspace_id, root_pane);
        if focus || self.state.active.is_none() {
            self.state.switch_workspace(idx);
            self.state.mode = Mode::Terminal;
        }
        self.schedule_session_save();
        Ok(idx)
    }

    pub(crate) fn open_float_pane(
        &mut self,
        ws_idx: usize,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<crate::layout::PaneId> {
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return Err(std::io::Error::other("workspace not found"));
        };
        let tab_idx = ws.active_tab_index();
        let cwd = cwd
            .or_else(|| {
                let tab = ws.active_tab()?;
                let focused = tab.focused_pane();
                tab.cwd_for_pane(focused, &self.state.terminals, &self.terminal_runtimes)
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));

        let terminal_area = self.state.view.terminal_area;
        let geometry = crate::popup_size::resolve_popup_geometry(
            self.state.floating_pane_width,
            self.state.floating_pane_height,
            terminal_area,
        );
        let (rows, cols) = match geometry {
            Some(geometry) => (geometry.inner.height, geometry.inner.width),
            None => self.state.estimate_pane_size(),
        };

        let pane_id = crate::layout::PaneId::alloc();
        let pane_number = self.state.workspaces[ws_idx].next_public_pane_number;
        let launch_env = self
            .pane_launch_env(ws_idx, pane_id, Vec::new())
            .unwrap_or_else(|| crate::pane::PaneLaunchEnv::from_extra(Vec::new()));

        let runtime = crate::terminal::TerminalRuntime::spawn(
            pane_id,
            rows,
            cols,
            cwd.clone(),
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
            crate::pane::PaneShellConfig::new(&self.state.default_shell, self.state.shell_mode),
            &launch_env,
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )?;

        let terminal_id = crate::terminal::TerminalId::alloc();
        let terminal = crate::terminal::TerminalState::new(terminal_id.clone(), cwd);
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(terminal_id.clone(), terminal);

        let ws = &mut self.state.workspaces[ws_idx];
        ws.register_new_pane_with_number(pane_id, pane_number);
        ws.tabs[tab_idx].push_float(pane_id, crate::pane::PaneState::new(terminal_id));

        self.schedule_session_save();
        Ok(pane_id)
    }

    pub(super) fn collect_panes_for_workspace(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<crate::api::schema::PaneInfo>, (String, String)> {
        if let Some(workspace_id) = workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(workspace_id) else {
                return Err((
                    "workspace_not_found".into(),
                    format!("workspace {workspace_id} not found"),
                ));
            };
            let Some(ws) = self.state.workspaces.get(ws_idx) else {
                return Err((
                    "workspace_not_found".into(),
                    format!("workspace {workspace_id} not found"),
                ));
            };
            Ok(ws
                .tabs
                .iter()
                .flat_map(|tab| tab.all_pane_ids())
                .filter_map(|pane_id| self.pane_info(ws_idx, pane_id))
                .collect())
        } else {
            Ok(self
                .state
                .workspaces
                .iter()
                .enumerate()
                .flat_map(|(ws_idx, ws)| {
                    ws.tabs
                        .iter()
                        .flat_map(|tab| tab.all_pane_ids())
                        .filter_map(move |pane_id| self.pane_info(ws_idx, pane_id))
                })
                .collect())
        }
    }

    pub(super) fn tab_info(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Option<crate::api::schema::TabInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab = ws.tabs.get(tab_idx)?;
        let (agg_state, seen) = tab
            .panes
            .values()
            .filter_map(|pane| {
                self.state
                    .terminals
                    .get(&pane.attached_terminal_id)
                    .map(|terminal| (terminal.state, pane.seen))
            })
            .max_by_key(|(state, seen)| tab_attention_priority(*state, *seen))
            .unwrap_or((crate::detect::AgentState::Unknown, true));
        Some(crate::api::schema::TabInfo {
            tab_id: self.public_tab_id(ws_idx, tab_idx)?,
            workspace_id: self.public_workspace_id(ws_idx),
            number: tab.number,
            label: ws.tab_display_name(tab_idx)?,
            focused: self.state.active == Some(ws_idx) && ws.active_tab == tab_idx,
            pane_count: tab.panes.len(),
            agent_status: pane_agent_status(agg_state, seen),
        })
    }

    pub(crate) fn emit_workspace_open_events(&mut self, ws_idx: usize) {
        let workspace_info = self.workspace_info(ws_idx);
        let Some(tab) = self.tab_info(ws_idx, 0) else {
            return;
        };
        let Some(root_pane) = self.root_pane_info(ws_idx, 0) else {
            return;
        };
        self.emit_event(EventEnvelope {
            event: EventKind::WorkspaceCreated,
            data: EventData::WorkspaceCreated {
                workspace: workspace_info,
            },
        });
        self.emit_tab_and_pane_created_events(tab, root_pane);
        self.emit_layout_updated_event(ws_idx, 0);
    }

    pub(crate) fn emit_tab_created_events(&mut self, ws_idx: usize, tab_idx: usize) {
        let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
            return;
        };
        let Some(root_pane) = self.root_pane_info(ws_idx, tab_idx) else {
            return;
        };
        self.emit_tab_and_pane_created_events(tab, root_pane);
        self.emit_layout_updated_event(ws_idx, tab_idx);
    }

    fn emit_tab_and_pane_created_events(
        &mut self,
        tab: crate::api::schema::TabInfo,
        root_pane: crate::api::schema::PaneInfo,
    ) {
        self.emit_event(EventEnvelope {
            event: EventKind::TabCreated,
            data: EventData::TabCreated { tab },
        });
        self.emit_event(EventEnvelope {
            event: EventKind::PaneCreated,
            data: EventData::PaneCreated { pane: root_pane },
        });
    }

    pub(super) fn workspace_created_result(
        &self,
        ws_idx: usize,
    ) -> Option<crate::api::schema::ResponseResult> {
        Some(crate::api::schema::ResponseResult::WorkspaceCreated {
            workspace: self.workspace_info(ws_idx),
            tab: self.tab_info(ws_idx, 0)?,
            root_pane: self.root_pane_info(ws_idx, 0)?,
        })
    }

    pub(super) fn tab_created_result(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Option<crate::api::schema::ResponseResult> {
        Some(crate::api::schema::ResponseResult::TabCreated {
            tab: self.tab_info(ws_idx, tab_idx)?,
            root_pane: self.root_pane_info(ws_idx, tab_idx)?,
        })
    }

    pub(super) fn root_pane_info(
        &self,
        ws_idx: usize,
        tab_idx: usize,
    ) -> Option<crate::api::schema::PaneInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab = ws.tabs.get(tab_idx)?;
        self.pane_info(ws_idx, tab.root_pane)
    }

    pub(super) fn pane_info(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::api::schema::PaneInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane = ws.pane_state(pane_id)?;
        let terminal = self.state.terminals.get(&pane.attached_terminal_id)?;
        let tab_idx = ws.find_tab_index_for_pane(pane_id)?;
        let scroll = self
            .state
            .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws_idx, pane_id)
            .and_then(|runtime| runtime.scroll_metrics())
            .map(|metrics| crate::api::schema::PaneScrollInfo {
                offset_from_bottom: metrics.offset_from_bottom as u64,
                max_offset_from_bottom: metrics.max_offset_from_bottom as u64,
                viewport_rows: metrics.viewport_rows as u64,
            });
        let focused = self.state.active == Some(ws_idx)
            && ws.active_tab == tab_idx
            && ws
                .focused_pane_id()
                .is_some_and(|focused| focused == pane_id);
        let presentation = terminal.effective_presentation();
        Some(crate::api::schema::PaneInfo {
            pane_id: self.public_pane_id(ws_idx, pane_id)?,
            terminal_id: terminal.id.to_string(),
            workspace_id: self.public_workspace_id(ws_idx),
            tab_id: self.public_tab_id(ws_idx, tab_idx)?,
            focused,
            floating: ws.tabs[tab_idx].is_float(pane_id),
            cwd: ws.tabs[tab_idx]
                .cwd_for_pane(pane_id, &self.state.terminals, &self.terminal_runtimes)
                .map(|cwd| cwd.display().to_string()),
            foreground_cwd: ws.tabs[tab_idx]
                .foreground_cwd_for_pane(pane_id, &self.terminal_runtimes)
                .map(|cwd| cwd.display().to_string()),
            label: terminal.manual_label.clone(),
            agent: terminal.effective_agent_label().map(str::to_string),
            title: presentation.title,
            terminal_title: terminal.terminal_title.clone(),
            terminal_title_stripped: terminal.terminal_title_stripped(),
            display_agent: presentation.display_agent,
            agent_status: pane_agent_status(terminal.state, pane.seen),
            state_labels: presentation.state_labels,
            tokens: terminal.metadata_tokens.values(),
            agent_session: terminal_agent_session_info(terminal),
            scroll,
            revision: terminal.revision,
        })
    }

    pub(super) fn lookup_runtime(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<(&crate::terminal::TerminalRuntime, String)> {
        let runtime =
            self.state
                .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws_idx, pane_id)?;
        Some((runtime, self.public_workspace_id(ws_idx)))
    }

    pub(super) fn lookup_runtime_sender(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<&crate::terminal::TerminalRuntime> {
        self.state
            .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws_idx, pane_id)
    }

    pub(super) fn workspace_info(&self, index: usize) -> crate::api::schema::WorkspaceInfo {
        let ws = &self.state.workspaces[index];
        let (agg_state, seen) = ws.aggregate_state(&self.state.terminals);
        crate::api::schema::WorkspaceInfo {
            workspace_id: self.public_workspace_id(index),
            number: index + 1,
            label: ws.display_name_from(&self.state.terminals, &self.terminal_runtimes),
            focused: self.state.active == Some(index),
            pane_count: ws.public_pane_numbers.len(),
            tab_count: ws.tabs.len(),
            active_tab_id: self.public_tab_id(index, ws.active_tab).unwrap_or_else(|| {
                crate::workspace::public_tab_id_for_number(&ws.id, ws.active_tab + 1)
            }),
            agent_status: pane_agent_status(agg_state, seen),
            tokens: ws.metadata_tokens.values(),
            worktree: ws
                .worktree_space()
                .map(|space| crate::api::schema::WorkspaceWorktreeInfo {
                    repo_key: space.key.clone(),
                    repo_name: space.label.clone(),
                    repo_root: space.repo_root.display().to_string(),
                    checkout_path: space.checkout_path.display().to_string(),
                    is_linked_worktree: space.is_linked_worktree,
                }),
            path: ws
                .pinned_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }

    /// The workspace whose pinned path claims `cwd`, if it is not the one the
    /// pane is already in. Deepest pinned path wins so a worktree pinned
    /// under its repo takes precedence over the repo. The pane's own
    /// workspace competes on depth too: nothing pulls a pane towards a pin
    /// no more specific than the one it already sits under.
    pub(crate) fn claiming_workspace(
        &self,
        cwd: &std::path::Path,
        source_ws_idx: usize,
    ) -> Option<usize> {
        // Measured on the canonical path so the depths being compared match
        // the paths path_claims actually compared.
        let claim_depth = |ws: &Workspace| -> Option<usize> {
            let pinned = ws.pinned_path.as_ref()?;
            crate::workspace::path_claims(pinned, cwd).then(|| {
                crate::worktree::canonical_or_original(pinned)
                    .components()
                    .count()
            })
        };
        let (claimant, depth) = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .filter(|(ws_idx, _)| *ws_idx != source_ws_idx)
            .filter_map(|(ws_idx, ws)| Some((ws_idx, claim_depth(ws)?)))
            .max_by_key(|(_, depth)| *depth)?;
        // Two workspaces pinned to one directory would otherwise trade the
        // pane back and forth on every directory change inside it.
        let source_depth = self
            .state
            .workspaces
            .get(source_ws_idx)
            .and_then(claim_depth);
        source_depth
            .is_none_or(|source| source < depth)
            .then_some(claimant)
    }

    /// Route a freshly created pane into the workspace that claims its cwd.
    /// Best effort: a failure here never fails the creation that triggered it.
    /// `label` is the destination tab's name, if the caller wants one — e.g.
    /// `tab.create --label` passes its own label; a plain pane split has none
    /// and passes `None`. The mover does not guess this from ambient state:
    /// picking it up from the source tab's name would leak an unrelated
    /// tab's name onto the destination after a split.
    pub(crate) fn auto_move_pane_to_pinned_workspace(
        &mut self,
        source_ws_idx: usize,
        pane_id: PaneId,
        cwd: &std::path::Path,
        focus: bool,
        label: Option<String>,
    ) -> bool {
        let Some(target_ws_idx) = self.claiming_workspace(cwd, source_ws_idx) else {
            return false;
        };
        let Some(workspace_id) = self
            .state
            .workspaces
            .get(target_ws_idx)
            .map(|ws| ws.id.clone())
        else {
            return false;
        };
        let Some(public_pane_id) = self.public_pane_id(source_ws_idx, pane_id) else {
            return false;
        };
        let response = self.handle_pane_move(
            "auto-move".to_string(),
            crate::api::schema::PaneMoveParams {
                pane_id: public_pane_id,
                destination: crate::api::schema::PaneMoveDestination::NewTab {
                    workspace_id: Some(workspace_id.clone()),
                    label,
                },
                focus,
            },
        );
        // A move can succeed without moving anything: a zoomed source tab is
        // left alone on purpose. Report that as not routed so the caller and
        // the log both describe where the pane actually is.
        let move_result = serde_json::from_str::<crate::api::schema::SuccessResponse>(&response)
            .ok()
            .and_then(|success| match success.result {
                crate::api::schema::ResponseResult::PaneMove { move_result } => Some(move_result),
                _ => None,
            });
        match move_result {
            Some(move_result) if move_result.changed => true,
            Some(move_result) => {
                // Expected, not a fault: a zoomed source tab declines on
                // purpose, and the pane stays where the user put it.
                tracing::debug!(
                    %workspace_id,
                    reason = ?move_result.reason,
                    "auto-move into pinned workspace declined"
                );
                false
            }
            None => {
                tracing::warn!(%workspace_id, "auto-move into pinned workspace failed");
                false
            }
        }
    }

    /// Re-run the pinned-path claim after a pane reports a new cwd, so a
    /// directory jump relocates the pane the way opening it there would have.
    /// Only fires at an idle shell prompt: a foreground process or a detected
    /// agent means something is running that should not be moved out from
    /// under the user.
    pub(crate) fn reclaim_pane_after_cwd_change(&mut self, pane_id: PaneId, cwd: &std::path::Path) {
        let Some((ws_idx, pane)) = self.find_pane(pane_id) else {
            return;
        };
        let terminal_id = pane.attached_terminal_id.clone();
        let Some(terminal) = self.state.terminals.get(&terminal_id) else {
            return;
        };
        // AppState rejects reports that are not absolute directories without
        // storing them, so a stored cwd that differs from the report means it
        // was rejected.
        if terminal.cwd != cwd {
            return;
        }
        if terminal.foreground_process_name.is_some() || terminal.detected_agent.is_some() {
            return;
        }
        // Follow the pane only when it is the one the user is sitting in: a
        // background pane changing directory on its own must not drag them out
        // of the workspace they are working in.
        let focus = self.state.active == Some(ws_idx)
            && self
                .state
                .workspaces
                .get(ws_idx)
                .and_then(|ws| ws.focused_pane_id())
                == Some(pane_id);
        if self.claiming_workspace(cwd, ws_idx).is_some() {
            self.auto_move_pane_to_pinned_workspace(ws_idx, pane_id, cwd, focus, None);
            return;
        }
        // Only the pane the user is sitting in may conjure a workspace, and
        // never one the pane's own workspace is already pinned under:
        // `claiming_workspace` skips the source workspace, so its pin has to
        // be checked here.
        let source_pin_claims = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.pinned_path.as_deref())
            .is_some_and(|pinned| crate::workspace::path_claims(pinned, cwd));
        if !focus || source_pin_claims {
            return;
        }
        let Some(repo) = crate::workspace::declared_repo_for(cwd, &self.state.declared_repo_paths)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        // The pane's own workspace already originated at this repo: pin it in
        // place rather than conjuring a twin workspace and closing the
        // original, which would silently discard its name, tab labels,
        // sidebar position, worktree membership, and id.
        let source_ws_is_repo = self
            .state
            .workspaces
            .get(ws_idx)
            .is_some_and(|ws| crate::workspace::path_claims(&repo, &ws.identity_cwd));
        if source_ws_is_repo {
            if let Some(ws) = self.state.workspaces.get_mut(ws_idx) {
                ws.pinned_path = Some(repo);
            }
            self.schedule_session_save();
            return;
        }
        self.create_declared_repo_workspace_for_pane(ws_idx, pane_id, &repo);
    }

    /// Move `pane_id` into a fresh workspace pinned to `repo`, the workspace a
    /// declared repo should have had all along. Best effort, like the pinned
    /// auto-move: a failure here never fails the directory change that
    /// triggered it.
    fn create_declared_repo_workspace_for_pane(
        &mut self,
        source_ws_idx: usize,
        pane_id: PaneId,
        repo: &std::path::Path,
    ) {
        let Some(public_pane_id) = self.public_pane_id(source_ws_idx, pane_id) else {
            return;
        };
        let response = self.handle_pane_move(
            "declared-repo".to_string(),
            crate::api::schema::PaneMoveParams {
                pane_id: public_pane_id,
                destination: crate::api::schema::PaneMoveDestination::NewWorkspace {
                    label: None,
                    tab_label: None,
                },
                focus: true,
            },
        );
        let move_result = serde_json::from_str::<crate::api::schema::SuccessResponse>(&response)
            .ok()
            .and_then(|success| match success.result {
                crate::api::schema::ResponseResult::PaneMove { move_result } => Some(move_result),
                _ => None,
            });
        let created = match move_result {
            Some(move_result) if move_result.changed => move_result.created_workspace,
            Some(move_result) => {
                // Expected, not a fault: a zoomed source tab declines on
                // purpose, and the pane stays where the user put it.
                tracing::debug!(
                    repo = %repo.display(),
                    reason = ?move_result.reason,
                    "declared repo workspace move declined"
                );
                None
            }
            None => {
                tracing::warn!(repo = %repo.display(), "declared repo workspace creation failed");
                None
            }
        };
        let Some(workspace) = created else {
            return;
        };
        // Both arms below are unreachable in practice: the id was generated
        // and pushed into `self.state.workspaces` moments earlier. Warn
        // rather than restructure, so a regression here is diagnosable
        // instead of silently leaving the pane in an unpinned workspace that
        // a later `cd` would try to reclaim all over again.
        let Some(ws_idx) = self.parse_workspace_id(&workspace.workspace_id) else {
            tracing::warn!(
                repo = %repo.display(),
                workspace_id = %workspace.workspace_id,
                "declared repo workspace id did not parse after creation"
            );
            return;
        };
        let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
            tracing::warn!(
                repo = %repo.display(),
                workspace_id = %workspace.workspace_id,
                "declared repo workspace vanished before it could be pinned"
            );
            return;
        };
        ws.pinned_path = Some(repo.to_path_buf());
        self.schedule_session_save();
    }
}

fn terminal_agent_session_info(
    terminal: &crate::terminal::TerminalState,
) -> Option<crate::api::schema::AgentSessionInfo> {
    if let Some(authority) = terminal.hook_authority.as_ref() {
        if let Some(session_ref) = authority.session_ref.as_ref() {
            return Some(crate::api::schema::AgentSessionInfo {
                source: authority.source.clone(),
                agent: authority.agent_label.clone(),
                kind: session_ref.kind,
                value: session_ref.value.clone(),
            });
        }
    }

    terminal
        .persisted_agent_session
        .as_ref()
        .map(|session| crate::api::schema::AgentSessionInfo {
            source: session.source.clone(),
            agent: session.agent.clone(),
            kind: session.session_ref.kind,
            value: session.session_ref.value.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::App;

    fn app_with_pinned_workspaces(pins: &[(&str, Option<&str>)]) -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = pins
            .iter()
            .map(|(name, pin)| {
                let mut ws = crate::workspace::Workspace::test_new(name);
                ws.pinned_path = pin.map(std::path::PathBuf::from);
                ws
            })
            .collect();
        app.state.active = Some(0);
        app
    }

    #[test]
    fn claims_a_pane_opened_below_the_pinned_path() {
        let app = app_with_pinned_workspaces(&[("a", None), ("b", Some("/ws"))]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/ws/src"), 0),
            Some(1)
        );
    }

    #[test]
    fn a_duplicate_pin_does_not_claim_a_pane_from_its_twin() {
        let app = app_with_pinned_workspaces(&[("a", Some("/ws")), ("b", Some("/ws"))]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/ws/src"), 0),
            None
        );
        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/ws/src"), 1),
            None
        );
    }

    #[test]
    fn a_shallower_pin_does_not_claim_a_pane_from_a_deeper_one() {
        let app = app_with_pinned_workspaces(&[("deep", Some("/a/b")), ("shallow", Some("/a"))]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/a/b/c"), 0),
            None
        );
    }

    #[test]
    fn a_deeper_pin_still_claims_a_pane_from_a_shallower_one() {
        let app = app_with_pinned_workspaces(&[("shallow", Some("/a")), ("deep", Some("/a/b"))]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/a/b/c"), 0),
            Some(1)
        );
    }

    #[test]
    fn does_not_claim_a_sibling_directory() {
        let app = app_with_pinned_workspaces(&[("a", None), ("b", Some("/ws"))]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/ws-worktrees/x"), 0),
            None
        );
    }

    #[test]
    fn does_not_claim_a_pane_already_in_the_claiming_workspace() {
        let app = app_with_pinned_workspaces(&[("b", Some("/ws"))]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/ws/src"), 0),
            None
        );
    }

    #[test]
    fn deepest_pinned_path_wins() {
        let app = app_with_pinned_workspaces(&[
            ("source", None),
            ("shallow", Some("/a")),
            ("deep", Some("/a/b")),
        ]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/a/b/c"), 0),
            Some(2)
        );
    }

    #[test]
    fn workspaces_without_a_pinned_path_never_claim() {
        let app = app_with_pinned_workspaces(&[("a", None), ("b", None)]);

        assert_eq!(
            app.claiming_workspace(std::path::Path::new("/ws/src"), 0),
            None
        );
    }
}
