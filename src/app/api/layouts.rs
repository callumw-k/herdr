use std::path::{Path, PathBuf};

use ratatui::layout::Direction;

use crate::api::schema::{
    ArrangementSchema, EventData, EventEnvelope, EventKind, LayoutApplyParams, LayoutDescription,
    LayoutExportParams, LayoutNode, LayoutPane, LayoutSetSplitRatioParams, ResponseResult,
    SplitDirection,
};
use crate::app::{App, Mode};
use crate::layout::{Arrangement, Node, PaneId, TileLayout};
use crate::workspace::NewPane;

use super::responses::{encode_error, encode_success};

const MAX_LAYOUT_PANES: usize = 24;
const MAX_LAYOUT_DEPTH: usize = 16;

impl App {
    pub(super) fn handle_layout_export(
        &mut self,
        id: String,
        params: LayoutExportParams,
    ) -> String {
        let Some((ws_idx, tab_idx)) = self.resolve_layout_export_target(&params) else {
            return encode_error(id, "layout_not_found", "layout target not found");
        };
        let Some(layout) = self.layout_description(ws_idx, tab_idx) else {
            return encode_error(id, "layout_not_found", "layout unavailable");
        };

        encode_success(id, ResponseResult::LayoutExport { layout })
    }

    pub(super) fn handle_layout_apply(&mut self, id: String, params: LayoutApplyParams) -> String {
        let replace_target = match params.tab_id.as_deref() {
            Some(tab_id) => match self.parse_tab_id(tab_id) {
                Some(target) => Some(target),
                None => {
                    return encode_error(id, "tab_not_found", format!("tab {tab_id} not found"))
                }
            },
            None => None,
        };
        if replace_target.is_some() && params.workspace_id.is_some() {
            return encode_error(
                id,
                "invalid_target",
                "use either tab_id or workspace_id, not both",
            );
        }

        let ws_idx = if let Some((ws_idx, _)) = replace_target {
            ws_idx
        } else if let Some(workspace_id) = params.workspace_id.as_deref() {
            let Some(ws_idx) = self.parse_workspace_id(workspace_id) else {
                return encode_error(
                    id,
                    "workspace_not_found",
                    format!("workspace {workspace_id} not found"),
                );
            };
            ws_idx
        } else if let Some(active) = self.state.active {
            active
        } else {
            return encode_error(id, "workspace_not_found", "no active workspace");
        };
        let roots: Vec<&LayoutNode> = std::iter::once(&params.root)
            .chain(params.float_root.as_ref())
            .collect();
        if let Err(message) = validate_layout_trees(&roots) {
            return encode_error(id, "invalid_layout", message);
        }

        let replacement_label = params.tab_label.clone().or_else(|| {
            let (_, tab_idx) = replace_target?;
            self.state
                .workspaces
                .get(ws_idx)?
                .tabs
                .get(tab_idx)?
                .custom_name
                .clone()
        });
        let replace_was_active = replace_target.is_some_and(|(target_ws, target_tab)| {
            self.state.active == Some(target_ws)
                && self
                    .state
                    .workspaces
                    .get(target_ws)
                    .is_some_and(|ws| ws.active_tab_index() == target_tab)
        });
        let root_leaf = first_layout_leaf(&params.root);
        let first_cwd = self.layout_root_cwd(ws_idx, replace_target, root_leaf);
        let (rows, cols) = self.state.estimate_pane_size();
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;
        let extra_env = match super::env::normalize_launch_env(root_leaf.env.clone()) {
            Ok(env) => env,
            Err((code, message)) => return encode_error(id, &code, message),
        };
        let command = match layout_command(root_leaf) {
            Ok(command) => command,
            Err(message) => return encode_error(id, "invalid_layout", message),
        };

        let created = {
            let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
                return encode_error(id, "workspace_not_found", "workspace not found");
            };
            if let Some(argv) = command.as_deref() {
                ws.create_tab_argv_command(
                    rows,
                    cols,
                    first_cwd,
                    argv,
                    extra_env,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                )
            } else {
                ws.create_tab(
                    rows,
                    cols,
                    first_cwd,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                    extra_env,
                )
            }
        };

        let (new_tab_idx, terminal, runtime) = match created {
            Ok(result) => result,
            Err(err) => return encode_error(id, "layout_apply_failed", err.to_string()),
        };
        let new_root_pane = self.state.workspaces[ws_idx].tabs[new_tab_idx].root_pane;
        self.terminal_runtimes.insert(terminal.id.clone(), runtime);
        self.state.remove_alias_shadowed_by_new_pane(new_root_pane);
        self.state.terminals.insert(terminal.id.clone(), terminal);
        if let Some(label) = replacement_label {
            self.state.workspaces[ws_idx].tabs[new_tab_idx].set_custom_name(label);
        }
        self.apply_layout_pane_label(ws_idx, new_root_pane, root_leaf);

        if let Err(message) = self.apply_layout_node_to_pane(ws_idx, new_root_pane, &params.root) {
            self.rollback_layout_tab(ws_idx, new_root_pane);
            return encode_error(id, "layout_apply_failed", message);
        }

        if let Some(float_root) = params.float_root.as_ref() {
            let float_leaf = first_layout_leaf(float_root);
            let float_cwd = self.layout_root_cwd(ws_idx, replace_target, float_leaf);
            if let Err(message) =
                self.apply_float_layout_root(ws_idx, new_tab_idx, float_root, float_cwd)
            {
                self.rollback_layout_tab(ws_idx, new_root_pane);
                return encode_error(id, "layout_apply_failed", message);
            }
        }
        // Building the trees above goes through the same split and float
        // primitives as ordinary pane creation, which mark each layer for a
        // re-flow. That re-flow is meant for pane create/close/arrangement-
        // cycle, not for a tree layout.apply just finished building to spec —
        // left set, the next render would discard it back into the layer's
        // arrangement.
        if let Some(tab) = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(new_tab_idx))
        {
            tab.needs_reflow = false;
            tab.float_needs_reflow = false;
            // An applied stack root is the tab's arrangement, not a one-off
            // shape. Leaving `arrangement` alone made `layout.export` report a
            // grid for a stacked tree, and the next pane create or close would
            // re-flow that stack away.
            if matches!(params.root, LayoutNode::Stack { .. }) {
                tab.arrangement = Arrangement::Stacked;
            }
            if matches!(params.float_root, Some(LayoutNode::Stack { .. })) {
                tab.float_arrangement = Arrangement::Stacked;
            }
        }

        if let Some((target_ws_idx, target_tab_idx)) = replace_target {
            let closed_tab_id = self
                .public_tab_id(target_ws_idx, target_tab_idx)
                .unwrap_or_else(|| {
                    crate::workspace::public_tab_id_for_number(
                        &self.public_workspace_id(target_ws_idx),
                        target_tab_idx + 1,
                    )
                });
            let terminal_ids = self
                .state
                .terminal_ids_for_tab(target_ws_idx, target_tab_idx);
            let plugin_pane_ids = self.state.pane_ids_for_tab(target_ws_idx, target_tab_idx);
            let Some(ws) = self.state.workspaces.get_mut(target_ws_idx) else {
                return encode_error(id, "tab_not_found", "tab not found");
            };
            if ws.close_tab(target_tab_idx) {
                self.state.remove_plugin_pane_records(plugin_pane_ids);
                self.state.remove_unattached_terminal_ids(terminal_ids);
                self.shutdown_detached_terminal_runtimes();
                self.emit_event(EventEnvelope {
                    event: EventKind::TabClosed,
                    data: EventData::TabClosed {
                        tab_id: closed_tab_id,
                        workspace_id: self.public_workspace_id(target_ws_idx),
                    },
                });
            }
        }

        let Some(new_tab_idx) = self.state.workspaces[ws_idx]
            .tabs
            .iter()
            .position(|tab| tab.root_pane == new_root_pane)
        else {
            return encode_error(id, "layout_apply_failed", "new layout tab disappeared");
        };

        if params.focus || replace_was_active {
            self.state.switch_workspace_tab(ws_idx, new_tab_idx);
            self.state.mode = Mode::Terminal;
        }
        self.schedule_session_save();
        if let Some(tab) = self.tab_info(ws_idx, new_tab_idx) {
            self.emit_event(EventEnvelope {
                event: EventKind::TabCreated,
                data: EventData::TabCreated { tab },
            });
        }
        for pane_id in self.state.workspaces[ws_idx].tabs[new_tab_idx]
            .layout
            .pane_ids()
        {
            if let Some(pane) = self.pane_info(ws_idx, pane_id) {
                self.emit_event(EventEnvelope {
                    event: EventKind::PaneCreated,
                    data: EventData::PaneCreated { pane },
                });
            }
        }
        self.emit_layout_updated_event(ws_idx, new_tab_idx);

        let Some(layout) = self.layout_description(ws_idx, new_tab_idx) else {
            return encode_error(id, "layout_apply_failed", "new layout unavailable");
        };
        encode_success(id, ResponseResult::LayoutApply { layout })
    }

    pub(super) fn handle_layout_set_split_ratio(
        &mut self,
        id: String,
        params: LayoutSetSplitRatioParams,
    ) -> String {
        if !params.ratio.is_finite() {
            return encode_error(id, "invalid_ratio", "ratio must be finite");
        }
        let Some((ws_idx, tab_idx)) = self.resolve_layout_export_target(&LayoutExportParams {
            tab_id: params.tab_id,
            pane_id: params.pane_id,
        }) else {
            return encode_error(id, "layout_not_found", "layout target not found");
        };

        let changed = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
            .is_some_and(|tab| tab.layout.set_ratio_at(&params.path, params.ratio));
        if !changed {
            return encode_error(id, "split_not_found", "split path not found");
        }

        self.schedule_session_save();
        let Some(layout) = self.layout_description(ws_idx, tab_idx) else {
            return encode_error(id, "layout_not_found", "layout unavailable");
        };
        self.emit_layout_updated_event(ws_idx, tab_idx);
        encode_success(id, ResponseResult::LayoutSplitRatioSet { layout })
    }

    fn resolve_layout_export_target(&self, params: &LayoutExportParams) -> Option<(usize, usize)> {
        match (params.tab_id.as_deref(), params.pane_id.as_deref()) {
            (Some(_), Some(_)) => None,
            (Some(tab_id), None) => self.parse_tab_id(tab_id),
            (None, Some(pane_id)) => {
                let (ws_idx, pane_id) = self.parse_pane_id(pane_id)?;
                let tab_idx = self
                    .state
                    .workspaces
                    .get(ws_idx)?
                    .find_tab_index_for_pane(pane_id)?;
                Some((ws_idx, tab_idx))
            }
            (None, None) => {
                let ws_idx = self.state.active?;
                let tab_idx = self.state.workspaces.get(ws_idx)?.active_tab_index();
                Some((ws_idx, tab_idx))
            }
        }
    }

    fn layout_description(&self, ws_idx: usize, tab_idx: usize) -> Option<LayoutDescription> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab = ws.tabs.get(tab_idx)?;
        let float_root = match tab.float_layout.as_ref() {
            Some(layout) => Some(self.layout_node_description(ws_idx, tab_idx, layout.root())?),
            None => None,
        };
        Some(LayoutDescription {
            workspace_id: self.public_workspace_id(ws_idx),
            tab_id: self.public_tab_id(ws_idx, tab_idx)?,
            zoomed: tab.zoomed,
            focused_pane_id: self.public_pane_id(ws_idx, tab.focused_pane())?,
            arrangement: arrangement_schema(tab.arrangement),
            float_arrangement: arrangement_schema(tab.float_arrangement),
            float_root,
            root: self.layout_node_description(ws_idx, tab_idx, tab.layout.root())?,
        })
    }

    fn layout_node_description(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        node: &Node,
    ) -> Option<LayoutNode> {
        match node {
            Node::Pane(pane_id) => Some(LayoutNode::Pane {
                pane: self.layout_pane_description(ws_idx, tab_idx, *pane_id)?,
            }),
            Node::Split {
                direction,
                ratio,
                first,
                second,
            } => Some(LayoutNode::Split {
                direction: match direction {
                    Direction::Horizontal => SplitDirection::Right,
                    Direction::Vertical => SplitDirection::Down,
                },
                ratio: *ratio,
                first: Box::new(self.layout_node_description(ws_idx, tab_idx, first)?),
                second: Box::new(self.layout_node_description(ws_idx, tab_idx, second)?),
            }),
            Node::Stack { panes, active } => {
                let panes = panes
                    .iter()
                    .map(|pane_id| self.layout_pane_description(ws_idx, tab_idx, *pane_id))
                    .collect::<Option<Vec<_>>>()?;
                Some(LayoutNode::Stack {
                    panes,
                    active: *active,
                })
            }
        }
    }

    fn layout_pane_description(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
    ) -> Option<LayoutPane> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab = ws.tabs.get(tab_idx)?;
        let terminal_id = tab.terminal_id(pane_id)?;
        let terminal = self.state.terminals.get(terminal_id);
        Some(LayoutPane {
            pane_id: Some(self.public_pane_id(ws_idx, pane_id)?),
            label: terminal.and_then(|terminal| terminal.manual_label.clone()),
            cwd: tab
                .cwd_for_pane(pane_id, &self.state.terminals, &self.terminal_runtimes)
                .map(|cwd| cwd.display().to_string()),
            command: terminal.and_then(|terminal| terminal.launch_argv.clone()),
            env: Default::default(),
        })
    }

    fn layout_root_cwd(
        &self,
        ws_idx: usize,
        replace_target: Option<(usize, usize)>,
        pane: &LayoutPane,
    ) -> PathBuf {
        if let Some(cwd) = pane.cwd.as_ref() {
            return PathBuf::from(cwd);
        }
        let follow_cwd = replace_target.and_then(|(_, tab_idx)| {
            let pane_id = self
                .state
                .workspaces
                .get(ws_idx)?
                .tabs
                .get(tab_idx)?
                .layout
                .focused();
            self.launch_cwd_for_pane_in_workspace(ws_idx, pane_id)
        });
        self.resolve_new_terminal_cwd(
            follow_cwd.or_else(|| self.focused_pane_cwd_in_workspace(ws_idx)),
        )
    }

    fn apply_layout_node_to_pane(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        node: &LayoutNode,
    ) -> Result<(), String> {
        match node {
            LayoutNode::Pane { pane } => {
                self.apply_layout_pane_label(ws_idx, pane_id, pane);
                Ok(())
            }
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let second_leaf = first_layout_leaf(second);
                let new_pane = self.layout_split_pane(
                    ws_idx,
                    pane_id,
                    direction.clone(),
                    *ratio,
                    second_leaf,
                )?;
                self.apply_layout_node_to_pane(ws_idx, pane_id, first)?;
                self.apply_layout_node_to_pane(ws_idx, new_pane, second)
            }
            LayoutNode::Stack { panes, active } => {
                // `pane_id` already carries stack_panes[0]'s cwd/command/env: the
                // caller created it (tab root or a split's leaf) using
                // `first_layout_leaf`, which descends into a stack the same way it
                // descends into a split's first child.
                let Some(first) = panes.first() else {
                    return Err("stack must have at least one pane".into());
                };
                self.apply_layout_pane_label(ws_idx, pane_id, first);

                let mut members = vec![pane_id];
                for pane in &panes[1..] {
                    let new_pane =
                        self.layout_split_pane(ws_idx, pane_id, SplitDirection::Right, 0.5, pane)?;
                    members.push(new_pane);
                }
                let active = (*active).min(members.len() - 1);
                self.collapse_layout_pane_into_stack(ws_idx, &members, active);
                Ok(())
            }
        }
    }

    /// Fold the panes created for a `LayoutNode::Stack` into a single
    /// `Node::Stack`. The panes were created one at a time via ordinary splits
    /// (there is no dedicated stack-insertion primitive), so this rewrites the
    /// resulting split chain — the smallest subtree whose panes are exactly
    /// `members` — into the flat shape the API describes. Pane ids are unique
    /// for the process lifetime, so that subtree is unambiguous.
    ///
    /// Focus moves to the requested active member: `TileLayout` tracks one
    /// focus pane for the whole tab, and `from_saved` derives a stack's active
    /// index from it, so the active member has to be the focus for the
    /// requested index to stick.
    fn collapse_layout_pane_into_stack(
        &mut self,
        ws_idx: usize,
        members: &[PaneId],
        active: usize,
    ) {
        let Some(&focus_member) = members.get(active).or_else(|| members.first()) else {
            return;
        };
        let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
            return;
        };
        let Some(tab_idx) = ws.find_tab_index_for_pane(focus_member) else {
            return;
        };
        let tab = &mut ws.tabs[tab_idx];
        let new_root = rebuild_layout_node_as_stack(tab.layout.root(), members, active);
        tab.layout = TileLayout::from_saved(new_root, focus_member);
    }

    fn layout_split_pane(
        &mut self,
        ws_idx: usize,
        target_pane_id: PaneId,
        direction: SplitDirection,
        ratio: f32,
        pane: &LayoutPane,
    ) -> Result<PaneId, String> {
        let (rows, cols) = self.state.estimate_pane_size();
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;
        let cwd = pane
            .cwd
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| self.launch_cwd_for_pane_in_workspace(ws_idx, target_pane_id));
        let extra_env = super::env::normalize_launch_env(pane.env.clone())
            .map_err(|(_, message)| message.to_string())?;
        let direction = match direction {
            SplitDirection::Right => Direction::Horizontal,
            SplitDirection::Down => Direction::Vertical,
        };
        let command = layout_command(pane)?;
        let result = {
            let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
                return Err("workspace not found".into());
            };
            if let Some(argv) = command.as_deref() {
                ws.split_pane_argv_command_with_ratio(
                    target_pane_id,
                    direction,
                    ratio,
                    rows,
                    cols,
                    cwd,
                    argv,
                    extra_env,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    false,
                )
            } else {
                ws.split_pane_with_ratio(
                    target_pane_id,
                    direction,
                    ratio,
                    rows,
                    cols,
                    cwd,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                    extra_env,
                    false,
                )
            }
        };
        let (_, new_pane) = result
            .ok_or_else(|| "pane not found".to_string())?
            .map_err(|err| err.to_string())?;
        let new_pane_id = new_pane.pane_id;
        self.attach_new_layout_pane(new_pane);
        self.apply_layout_pane_label(ws_idx, new_pane_id, pane);
        Ok(new_pane_id)
    }

    fn attach_new_layout_pane(&mut self, new_pane: NewPane) {
        self.terminal_runtimes
            .insert(new_pane.terminal.id.clone(), new_pane.runtime);
        self.state
            .remove_alias_shadowed_by_new_pane(new_pane.pane_id);
        self.state
            .terminals
            .insert(new_pane.terminal.id.clone(), new_pane.terminal);
    }

    fn apply_layout_pane_label(&mut self, ws_idx: usize, pane_id: PaneId, pane: &LayoutPane) {
        let Some(label) = pane
            .label
            .as_ref()
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
        else {
            return;
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
            .cloned()
        else {
            return;
        };
        if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
            terminal.set_manual_label(label.to_string());
        }
    }

    /// Builds the float layer for a layout.apply request. Unlike the tiled
    /// root, which grows through incremental splits so it can attach to an
    /// already-running tab, the float layer starts empty for a freshly
    /// created tab, so the whole tree can be spawned and assembled in one pass.
    fn apply_float_layout_root(
        &mut self,
        ws_idx: usize,
        tab_idx: usize,
        node: &LayoutNode,
        default_cwd: PathBuf,
    ) -> Result<(), String> {
        let (float_node, focus) = self.build_float_node(ws_idx, tab_idx, node, &default_cwd)?;
        let Some(tab) = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return Err("tab not found".into());
        };
        tab.float_layout = Some(TileLayout::from_saved(float_node, focus));
        Ok(())
    }

    fn build_float_node(
        &mut self,
        ws_idx: usize,
        tab_idx: usize,
        node: &LayoutNode,
        default_cwd: &Path,
    ) -> Result<(Node, PaneId), String> {
        match node {
            LayoutNode::Pane { pane } => {
                let pane_id = self.spawn_float_pane(ws_idx, tab_idx, pane, default_cwd)?;
                Ok((Node::Pane(pane_id), pane_id))
            }
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_node, focus) =
                    self.build_float_node(ws_idx, tab_idx, first, default_cwd)?;
                let (second_node, _) =
                    self.build_float_node(ws_idx, tab_idx, second, default_cwd)?;
                let direction = match direction {
                    SplitDirection::Right => Direction::Horizontal,
                    SplitDirection::Down => Direction::Vertical,
                };
                Ok((
                    Node::Split {
                        direction,
                        ratio: *ratio,
                        first: Box::new(first_node),
                        second: Box::new(second_node),
                    },
                    focus,
                ))
            }
            LayoutNode::Stack { panes, active } => {
                let mut ids = Vec::with_capacity(panes.len());
                for pane in panes {
                    ids.push(self.spawn_float_pane(ws_idx, tab_idx, pane, default_cwd)?);
                }
                // validate_layout_tree already rejected an empty stack and an
                // out-of-range active index before this ran.
                let focus = ids[*active];
                Ok((
                    Node::Stack {
                        panes: ids,
                        active: *active,
                    },
                    focus,
                ))
            }
        }
    }

    /// Spawns a runtime for one float leaf. There is no existing float pane to
    /// split from here — unlike `layout_split_pane`, which extends the tiled
    /// tree — so this mirrors `App::open_float_pane`'s spawn plumbing rather
    /// than routing through it.
    ///
    /// ponytail: every leaf without its own `cwd` falls back to the same
    /// `default_cwd` (the float root's), rather than chaining from its
    /// nearest sibling the way the tiled root's split-by-split build does.
    /// Upgrade to per-sibling chaining if float trees with deep, mixed
    /// explicit/inherited cwds turn out to matter in practice.
    fn spawn_float_pane(
        &mut self,
        ws_idx: usize,
        tab_idx: usize,
        pane: &LayoutPane,
        default_cwd: &Path,
    ) -> Result<PaneId, String> {
        let cwd = pane
            .cwd
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_cwd.to_path_buf());
        let extra_env = super::env::normalize_launch_env(pane.env.clone())
            .map_err(|(_, message)| message.to_string())?;
        let command = layout_command(pane)?;

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

        let pane_id = PaneId::alloc();
        let pane_number = self.state.workspaces[ws_idx].next_public_pane_number;
        let workspace_id = self.public_workspace_id(ws_idx);
        let tab_number = self.state.workspaces[ws_idx].tabs[tab_idx].number;
        let launch_env = crate::pane::PaneLaunchEnv::from_extra(extra_env).with_identity(
            workspace_id.clone(),
            crate::workspace::public_tab_id_for_number(&workspace_id, tab_number),
            crate::workspace::public_pane_id_for_number(&workspace_id, pane_number),
        );
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;

        let runtime = if let Some(argv) = command.as_deref() {
            crate::terminal::TerminalRuntime::spawn_argv_command(
                pane_id,
                rows,
                cols,
                cwd.clone(),
                argv,
                &launch_env,
                crate::pane::AgentDetection::Enabled,
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                self.event_tx.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            )
        } else {
            crate::terminal::TerminalRuntime::spawn(
                pane_id,
                rows,
                cols,
                cwd.clone(),
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                &launch_env,
                self.event_tx.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            )
        }
        .map_err(|err| err.to_string())?;

        let terminal_id = crate::terminal::TerminalId::alloc();
        let terminal = match command {
            Some(argv) => {
                crate::terminal::TerminalState::new(terminal_id.clone(), cwd).with_launch_argv(argv)
            }
            None => crate::terminal::TerminalState::new(terminal_id.clone(), cwd),
        };
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.remove_alias_shadowed_by_new_pane(pane_id);
        self.state.terminals.insert(terminal_id.clone(), terminal);

        let ws = &mut self.state.workspaces[ws_idx];
        ws.register_new_pane_with_number(pane_id, pane_number);
        ws.tabs[tab_idx]
            .panes
            .insert(pane_id, crate::pane::PaneState::new(terminal_id));

        self.apply_layout_pane_label(ws_idx, pane_id, pane);
        Ok(pane_id)
    }

    fn rollback_layout_tab(&mut self, ws_idx: usize, root_pane: PaneId) {
        let Some(tab_idx) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.tabs.iter().position(|tab| tab.root_pane == root_pane))
        else {
            return;
        };
        let terminal_ids = self.state.terminal_ids_for_tab(ws_idx, tab_idx);
        let plugin_pane_ids = self.state.pane_ids_for_tab(ws_idx, tab_idx);
        if self
            .state
            .workspaces
            .get_mut(ws_idx)
            .is_some_and(|ws| ws.close_tab(tab_idx))
        {
            self.state.remove_plugin_pane_records(plugin_pane_ids);
            self.state.remove_unattached_terminal_ids(terminal_ids);
            self.shutdown_detached_terminal_runtimes();
        }
    }
}

fn first_layout_leaf(node: &LayoutNode) -> &LayoutPane {
    match node {
        LayoutNode::Pane { pane } => pane,
        LayoutNode::Split { first, .. } => first_layout_leaf(first),
        // `validate_layout_tree` runs before this is called and rejects an
        // empty stack, so `panes` is guaranteed non-empty here.
        LayoutNode::Stack { panes, .. } => panes.first().expect("stack validated non-empty"),
    }
}

fn layout_command(pane: &LayoutPane) -> Result<Option<Vec<String>>, String> {
    match pane.command.as_ref() {
        Some(command) if command.is_empty() => Err("pane command must not be empty".into()),
        Some(command) => Ok(Some(command.clone())),
        None => Ok(None),
    }
}

/// Every root in one call shares a single pane budget: `layout.apply` spawns a
/// PTY per pane across all of them, so validating them separately would let a
/// tiled tree and a float tree each claim the cap.
fn validate_layout_trees(roots: &[&LayoutNode]) -> Result<(), String> {
    let mut stats = LayoutTreeStats {
        panes: 0,
        max_depth: 0,
    };
    for root in roots {
        validate_layout_node(root, 1, &mut stats)?;
    }
    if stats.panes > MAX_LAYOUT_PANES {
        return Err(format!(
            "layout has {} panes; maximum is {}",
            stats.panes, MAX_LAYOUT_PANES
        ));
    }
    if stats.max_depth > MAX_LAYOUT_DEPTH {
        return Err(format!(
            "layout depth is {}; maximum is {}",
            stats.max_depth, MAX_LAYOUT_DEPTH
        ));
    }
    Ok(())
}

struct LayoutTreeStats {
    panes: usize,
    max_depth: usize,
}

fn validate_layout_node(
    node: &LayoutNode,
    depth: usize,
    stats: &mut LayoutTreeStats,
) -> Result<(), String> {
    stats.max_depth = stats.max_depth.max(depth);
    if depth > MAX_LAYOUT_DEPTH {
        return Err(format!(
            "layout depth is {}; maximum is {}",
            depth, MAX_LAYOUT_DEPTH
        ));
    }
    match node {
        LayoutNode::Pane { pane } => {
            stats.panes += 1;
            if stats.panes > MAX_LAYOUT_PANES {
                return Err(format!("layout has more than {} panes", MAX_LAYOUT_PANES));
            }
            layout_command(pane)?;
            super::env::normalize_launch_env(pane.env.clone())
                .map_err(|(_, message)| message.to_string())?;
            Ok(())
        }
        LayoutNode::Split {
            first,
            second,
            ratio,
            ..
        } => {
            if !ratio.is_finite() {
                return Err("split ratio must be finite".into());
            }
            validate_layout_node(first, depth + 1, stats)?;
            validate_layout_node(second, depth + 1, stats)
        }
        LayoutNode::Stack { panes, active } => {
            if panes.is_empty() {
                return Err("stack must have at least one pane".into());
            }
            if *active >= panes.len() {
                return Err(format!(
                    "stack active index {active} is out of range for {} panes",
                    panes.len()
                ));
            }
            for pane in panes {
                stats.panes += 1;
                if stats.panes > MAX_LAYOUT_PANES {
                    return Err(format!("layout has more than {} panes", MAX_LAYOUT_PANES));
                }
                layout_command(pane)?;
                super::env::normalize_launch_env(pane.env.clone())
                    .map_err(|(_, message)| message.to_string())?;
            }
            Ok(())
        }
    }
}

/// Pane ids under `node`, in tree order.
fn node_pane_ids(node: &Node) -> Vec<PaneId> {
    match node {
        Node::Pane(id) => vec![*id],
        Node::Split { first, second, .. } => {
            let mut ids = node_pane_ids(first);
            ids.extend(node_pane_ids(second));
            ids
        }
        Node::Stack { panes, .. } => panes.clone(),
    }
}

/// Rebuild `node`, replacing the smallest subtree whose pane set exactly
/// matches `members` with a flat `Node::Stack`. Pane ids are unique for the
/// process lifetime, so that subtree is unambiguous.
fn rebuild_layout_node_as_stack(node: &Node, members: &[PaneId], active: usize) -> Node {
    let ids = node_pane_ids(node);
    if ids.len() == members.len() && members.iter().all(|id| ids.contains(id)) {
        return Node::Stack {
            panes: members.to_vec(),
            active,
        };
    }
    match node {
        Node::Pane(id) => Node::Pane(*id),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => Node::Split {
            direction: *direction,
            ratio: *ratio,
            first: Box::new(rebuild_layout_node_as_stack(first, members, active)),
            second: Box::new(rebuild_layout_node_as_stack(second, members, active)),
        },
        Node::Stack { panes, active } => Node::Stack {
            panes: panes.clone(),
            active: *active,
        },
    }
}

fn arrangement_schema(arrangement: Arrangement) -> ArrangementSchema {
    match arrangement {
        Arrangement::Vertical => ArrangementSchema::Vertical,
        Arrangement::Horizontal => ArrangementSchema::Horizontal,
        Arrangement::Grid => ArrangementSchema::Grid,
        Arrangement::Stacked => ArrangementSchema::Stacked,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};
    use super::*;
    use crate::{
        api::schema::{ErrorResponse, ResponseResult, SuccessResponse},
        config::{Config, ShellModeConfig},
        workspace::Workspace,
    };
    use ratatui::layout::Rect;

    fn app_with_workspace() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        app.state.workspaces = vec![Workspace::test_new("layout")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        app
    }

    #[test]
    fn layout_export_returns_portable_tree() {
        let mut app = app_with_workspace();
        let root = app.state.workspaces[0].tabs[0].root_pane;
        let right = app.state.workspaces[0].test_split(Direction::Horizontal);
        app.state.ensure_test_terminals();
        app.state.workspaces[0].tabs[0].layout.focus_pane(root);
        app.state.workspaces[0].tabs[0]
            .layout
            .set_ratio_at(&[], 0.65);
        let right_terminal_id = app.state.workspaces[0].tabs[0]
            .terminal_id(right)
            .cloned()
            .unwrap();
        app.state
            .terminals
            .get_mut(&right_terminal_id)
            .unwrap()
            .set_manual_label("tests".into());

        let response = app.handle_layout_export(
            "req".into(),
            LayoutExportParams {
                tab_id: None,
                pane_id: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutExport { layout } = success.result else {
            panic!("expected layout export response");
        };
        assert_eq!(layout.workspace_id, app.public_workspace_id(0));
        assert_eq!(layout.focused_pane_id, app.public_pane_id(0, root).unwrap());
        let LayoutNode::Split {
            direction,
            ratio,
            second,
            ..
        } = layout.root
        else {
            panic!("expected split layout root");
        };
        assert_eq!(direction, SplitDirection::Right);
        assert!((ratio - 0.65).abs() < f32::EPSILON);
        let LayoutNode::Pane { pane } = *second else {
            panic!("expected second pane");
        };
        assert_eq!(pane.label.as_deref(), Some("tests"));
        assert_eq!(pane.pane_id, Some(app.public_pane_id(0, right).unwrap()));
        assert_eq!(layout.arrangement, ArrangementSchema::Grid);
    }

    #[test]
    fn layout_export_returns_a_stack_node_for_a_stacked_tab() {
        let mut app = app_with_workspace();
        let root = app.state.workspaces[0].tabs[0].root_pane;
        let second = app.state.workspaces[0].test_split(Direction::Horizontal);
        app.state.ensure_test_terminals();
        {
            let tab = &mut app.state.workspaces[0].tabs[0];
            tab.layout = TileLayout::from_saved(
                Node::Stack {
                    panes: vec![root, second],
                    active: 1,
                },
                second,
            );
            tab.arrangement = Arrangement::Stacked;
        }

        let response = app.handle_layout_export(
            "req".into(),
            LayoutExportParams {
                tab_id: None,
                pane_id: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutExport { layout } = success.result else {
            panic!("expected layout export response");
        };
        assert_eq!(layout.arrangement, ArrangementSchema::Stacked);
        let LayoutNode::Stack { panes, active } = layout.root else {
            panic!("expected stack layout root");
        };
        assert_eq!(active, 1);
        assert_eq!(
            panes
                .iter()
                .map(|pane| pane.pane_id.clone())
                .collect::<Vec<_>>(),
            vec![app.public_pane_id(0, root), app.public_pane_id(0, second),]
        );
    }

    #[tokio::test]
    async fn layout_apply_installs_a_stack_of_panes() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("stack".into()),
                focus: true,
                root: LayoutNode::Stack {
                    panes: vec![
                        LayoutPane {
                            label: Some("one".into()),
                            ..Default::default()
                        },
                        LayoutPane {
                            label: Some("two".into()),
                            command: Some(vec!["sh".into(), "-c".into(), "true".into()]),
                            ..Default::default()
                        },
                        LayoutPane {
                            label: Some("three".into()),
                            ..Default::default()
                        },
                    ],
                    active: 1,
                },
                float_root: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutApply { layout } = success.result else {
            panic!("expected layout apply response");
        };
        let LayoutNode::Stack { panes, active } = layout.root else {
            panic!("expected stack layout root");
        };
        assert_eq!(active, 1);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].label.as_deref(), Some("one"));
        assert_eq!(panes[1].label.as_deref(), Some("two"));
        assert_eq!(panes[2].label.as_deref(), Some("three"));
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.workspaces[0].tabs[0].layout.pane_count(), 3);
        assert!(matches!(
            app.state.workspaces[0].tabs[0].layout.root(),
            Node::Stack { panes, .. } if panes.len() == 3
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_stack_survives_the_next_render_reflow() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("stack".into()),
                focus: true,
                root: LayoutNode::Stack {
                    panes: vec![
                        LayoutPane::default(),
                        LayoutPane::default(),
                        LayoutPane::default(),
                    ],
                    active: 1,
                },
                float_root: None,
            },
        );

        let tab = &mut app.state.workspaces[0].tabs[0];
        let members = tab.layout.pane_ids();
        assert_eq!(members.len(), 3);

        // A render tick reflows the active tab unconditionally
        // (src/ui.rs's compute_view/compute_mobile_view); the tree
        // layout.apply just built must survive it.
        tab.reflow(Rect::new(0, 0, 80, 20), None);

        assert!(matches!(
            tab.layout.root(),
            Node::Stack { panes, active } if *panes == members && *active == 1
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_stack_makes_the_tab_report_the_stacked_arrangement() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("stack".into()),
                focus: true,
                root: LayoutNode::Stack {
                    panes: vec![LayoutPane::default(), LayoutPane::default()],
                    active: 0,
                },
                float_root: None,
            },
        );

        let response = app.handle_layout_export(
            "req".into(),
            LayoutExportParams {
                tab_id: None,
                pane_id: None,
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutExport { layout } = success.result else {
            panic!("expected layout export response");
        };
        assert!(matches!(layout.root, LayoutNode::Stack { .. }));
        assert_eq!(layout.arrangement, ArrangementSchema::Stacked);

        // A mismatched arrangement would survive the render re-flow guard only
        // to be re-flowed away by the next pane create or close.
        let tab = &mut app.state.workspaces[0].tabs[0];
        let members = tab.layout.pane_ids();
        tab.needs_reflow = true;
        tab.reflow(Rect::new(0, 0, 80, 20), None);
        assert!(matches!(
            tab.layout.root(),
            Node::Stack { panes, .. } if *panes == members
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_rejects_stack_with_out_of_range_active() {
        let mut app = app_with_workspace();
        let original_tab_count = app.state.workspaces[0].tabs.len();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: None,
                tab_label: Some("bad".into()),
                focus: false,
                root: LayoutNode::Stack {
                    panes: vec![LayoutPane::default()],
                    active: 5,
                },
                float_root: None,
            },
        );

        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "invalid_layout");
        assert_eq!(app.state.workspaces[0].tabs.len(), original_tab_count);
    }

    #[tokio::test]
    async fn layout_apply_installs_a_float_layer() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("floats".into()),
                focus: true,
                root: LayoutNode::Pane {
                    pane: LayoutPane::default(),
                },
                float_root: Some(LayoutNode::Stack {
                    panes: vec![
                        LayoutPane {
                            label: Some("one".into()),
                            ..Default::default()
                        },
                        LayoutPane {
                            label: Some("two".into()),
                            ..Default::default()
                        },
                    ],
                    active: 1,
                }),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutApply { layout } = success.result else {
            panic!("expected layout apply response");
        };
        assert_eq!(layout.float_arrangement, ArrangementSchema::Stacked);
        let LayoutNode::Stack { panes, active } = layout.float_root.expect("float root") else {
            panic!("expected stack float root");
        };
        assert_eq!(active, 1);
        assert_eq!(panes[0].label.as_deref(), Some("one"));
        assert_eq!(panes[1].label.as_deref(), Some("two"));

        let tab = &app.state.workspaces[0].tabs[0];
        assert_eq!(tab.floats().len(), 2);
        let float_layout = tab.float_layout.as_ref().expect("float layout");
        assert!(matches!(
            float_layout.root(),
            Node::Stack { panes, .. } if panes.len() == 2
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_float_layer_survives_the_next_render_reflow() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        // The tiled root is a `Split`, not a bare `Pane`: building it goes
        // through the ordinary tiled split primitive, which marks the tab
        // for a re-flow while it works. That is what makes this test able to
        // detect a missing `needs_reflow` clear-up — with a single-pane tiled
        // root, nothing ever sets the flag true, and a no-op `reflow()` call
        // proves nothing either way.
        app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("floats".into()),
                focus: true,
                root: LayoutNode::Split {
                    direction: SplitDirection::Right,
                    ratio: 0.7,
                    first: Box::new(LayoutNode::Pane {
                        pane: LayoutPane::default(),
                    }),
                    second: Box::new(LayoutNode::Pane {
                        pane: LayoutPane::default(),
                    }),
                },
                float_root: Some(LayoutNode::Stack {
                    panes: vec![LayoutPane::default(), LayoutPane::default()],
                    active: 0,
                }),
            },
        );

        let tab = &mut app.state.workspaces[0].tabs[0];
        let float_members = tab.floats();
        assert_eq!(float_members.len(), 2);
        // Confirms the split primitive actually did mark the tab, so the
        // next assertion is exercising the clear-up rather than a no-op.
        assert!(!tab.needs_reflow);

        // A render tick reflows the active tab unconditionally
        // (src/ui.rs's compute_view/compute_mobile_view); both the split's
        // ratio and the float stack layout.apply just built must survive it.
        tab.reflow(Rect::new(0, 0, 80, 20), Some(Rect::new(0, 0, 40, 10)));

        let Node::Split { ratio, .. } = tab.layout.root() else {
            panic!("expected split tiled root");
        };
        assert!((*ratio - 0.7).abs() < f32::EPSILON);
        let float_layout = tab.float_layout.as_ref().expect("float layout");
        assert!(matches!(
            float_layout.root(),
            Node::Stack { panes, .. } if *panes == float_members
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_rejects_float_root_with_out_of_range_active() {
        let mut app = app_with_workspace();
        let original_tab_count = app.state.workspaces[0].tabs.len();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: None,
                tab_label: Some("bad".into()),
                focus: false,
                root: LayoutNode::Pane {
                    pane: LayoutPane::default(),
                },
                float_root: Some(LayoutNode::Stack {
                    panes: vec![LayoutPane::default()],
                    active: 5,
                }),
            },
        );

        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "invalid_layout");
        assert_eq!(app.state.workspaces[0].tabs.len(), original_tab_count);
    }

    #[tokio::test]
    async fn layout_export_focused_pane_id_names_the_tiled_pane_when_the_tiled_layer_holds_focus() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("floats".into()),
                focus: true,
                root: LayoutNode::Pane {
                    pane: LayoutPane::default(),
                },
                float_root: Some(LayoutNode::Pane {
                    pane: LayoutPane::default(),
                }),
            },
        );

        let tab = &app.state.workspaces[0].tabs[0];
        assert!(!tab.float_focused);
        let tiled_pane = tab.root_pane;

        let response = app.handle_layout_export(
            "req".into(),
            LayoutExportParams {
                tab_id: None,
                pane_id: None,
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutExport { layout } = success.result else {
            panic!("expected layout export response");
        };
        assert_eq!(
            layout.focused_pane_id,
            app.public_pane_id(0, tiled_pane).unwrap()
        );
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_export_focused_pane_id_names_the_focused_float_when_the_float_layer_holds_focus(
    ) {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("floats".into()),
                focus: true,
                root: LayoutNode::Pane {
                    pane: LayoutPane::default(),
                },
                float_root: Some(LayoutNode::Pane {
                    pane: LayoutPane::default(),
                }),
            },
        );

        let tab = &mut app.state.workspaces[0].tabs[0];
        let tiled_pane = tab.root_pane;
        let float_pane = tab.floats()[0];
        // layout.apply installs the float layer but does not move focus onto
        // it; simulate the layer already holding focus the way a real client
        // interaction (e.g. pane.float or the float-cycle keybind) would.
        tab.float_focused = true;

        let response = app.handle_layout_export(
            "req".into(),
            LayoutExportParams {
                tab_id: None,
                pane_id: None,
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutExport { layout } = success.result else {
            panic!("expected layout export response");
        };
        assert_eq!(
            layout.focused_pane_id,
            app.public_pane_id(0, float_pane).unwrap()
        );
        assert_ne!(
            layout.focused_pane_id,
            app.public_pane_id(0, tiled_pane).unwrap()
        );
        shutdown_test_runtimes(&mut app);
    }

    #[test]
    fn layout_validation_rejects_an_empty_stack() {
        let root = LayoutNode::Stack {
            panes: vec![],
            active: 0,
        };

        let err = validate_layout_trees(&[&root]).unwrap_err();
        assert!(err.contains("at least one pane"));
    }

    #[test]
    fn layout_set_split_ratio_updates_existing_split() {
        let mut app = app_with_workspace();
        app.state.workspaces[0].test_split(Direction::Horizontal);

        let response = app.handle_layout_set_split_ratio(
            "req".into(),
            LayoutSetSplitRatioParams {
                tab_id: None,
                pane_id: None,
                path: vec![],
                ratio: 0.72,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutSplitRatioSet { layout } = success.result else {
            panic!("expected layout split ratio set response");
        };
        let LayoutNode::Split { ratio, .. } = layout.root else {
            panic!("expected split layout root");
        };
        assert!((ratio - 0.72).abs() < f32::EPSILON);
        assert!(matches!(
            &app.event_hub.events_after(0).last().expect("layout event").1.data,
            EventData::LayoutUpdated { layout }
                if layout.tab_id == app.public_tab_id(0, 0).unwrap()
                    && (layout.splits[0].ratio - 0.72).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn layout_set_split_ratio_rejects_missing_split() {
        let mut app = app_with_workspace();

        let response = app.handle_layout_set_split_ratio(
            "req".into(),
            LayoutSetSplitRatioParams {
                tab_id: None,
                pane_id: None,
                path: vec![],
                ratio: 0.72,
            },
        );

        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "split_not_found");
    }

    #[tokio::test]
    async fn layout_apply_replaces_tab_with_requested_tree() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("dev".into()),
                focus: true,
                root: LayoutNode::Split {
                    direction: SplitDirection::Right,
                    ratio: 0.7,
                    first: Box::new(LayoutNode::Pane {
                        pane: LayoutPane {
                            label: Some("editor".into()),
                            ..Default::default()
                        },
                    }),
                    second: Box::new(LayoutNode::Pane {
                        pane: LayoutPane {
                            label: Some("tests".into()),
                            command: Some(vec!["sh".into(), "-c".into(), "true".into()]),
                            env: std::collections::HashMap::from([(
                                "HERDR_ROLE".into(),
                                "tests".into(),
                            )]),
                            ..Default::default()
                        },
                    }),
                },
                float_root: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::LayoutApply { layout } = success.result else {
            panic!("expected layout apply response");
        };
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(
            app.state.workspaces[0].tab_display_name(0).as_deref(),
            Some("dev")
        );
        let LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } = layout.root
        else {
            panic!("expected split layout root");
        };
        assert_eq!(direction, SplitDirection::Right);
        assert!((ratio - 0.7).abs() < f32::EPSILON);
        let LayoutNode::Pane { pane: first_pane } = *first else {
            panic!("expected first pane");
        };
        let LayoutNode::Pane { pane: second_pane } = *second else {
            panic!("expected second pane");
        };
        assert_eq!(first_pane.label.as_deref(), Some("editor"));
        assert_eq!(second_pane.label.as_deref(), Some("tests"));
        assert_eq!(
            second_pane.command,
            Some(vec!["sh".into(), "-c".into(), "true".into()])
        );
        assert!(matches!(
            &app.event_hub.events_after(0).last().expect("layout event").1.data,
            EventData::LayoutUpdated { layout }
                if layout.tab_id == app.public_tab_id(0, 0).unwrap()
                    && layout.panes.len() == 2
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_split_survives_the_next_render_reflow() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();

        app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("dev".into()),
                focus: true,
                root: LayoutNode::Split {
                    direction: SplitDirection::Right,
                    ratio: 0.7,
                    first: Box::new(LayoutNode::Pane {
                        pane: LayoutPane::default(),
                    }),
                    second: Box::new(LayoutNode::Pane {
                        pane: LayoutPane::default(),
                    }),
                },
                float_root: None,
            },
        );

        let tab = &mut app.state.workspaces[0].tabs[0];

        // A render tick reflows the active tab unconditionally
        // (src/ui.rs's compute_view/compute_mobile_view); the split
        // layout.apply just built, including its ratio, must survive it.
        tab.reflow(Rect::new(0, 0, 80, 20), None);

        assert!(matches!(
            tab.layout.root(),
            Node::Split { ratio, .. } if (*ratio - 0.7).abs() < f32::EPSILON
        ));
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_new_tab_follows_cached_focused_pane_cwd_without_runtime() {
        let mut app = app_with_workspace();
        let focused_pane = app.state.workspaces[0].tabs[0].root_pane;
        let cached_cwd = std::env::temp_dir();
        let terminal_id = app.state.workspaces[0]
            .terminal_id(focused_pane)
            .cloned()
            .unwrap();
        app.state.terminals.get_mut(&terminal_id).unwrap().cwd = cached_cwd.clone();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: None,
                tab_label: Some("cached".into()),
                focus: false,
                root: LayoutNode::Pane {
                    pane: LayoutPane::default(),
                },
                float_root: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::LayoutApply { .. }));
        let created = &app.state.workspaces[0].tabs[1];
        let created_terminal_id = created.terminal_id(created.root_pane).unwrap();
        let created_cwd = &app.state.terminals.get(created_terminal_id).unwrap().cwd;
        assert_eq!(
            crate::worktree::canonical_or_original(created_cwd),
            crate::worktree::canonical_or_original(&cached_cwd)
        );
        shutdown_test_runtimes(&mut app);
    }

    #[tokio::test]
    async fn layout_apply_replace_drops_plugin_pane_records_of_replaced_tab() {
        let mut app = app_with_workspace();
        let original_tab_id = app.public_tab_id(0, 0).unwrap();
        let replaced_pane = app.state.workspaces[0].tabs[0].root_pane;
        app.state.plugin_panes.insert(
            replaced_pane,
            crate::app::state::PluginPaneRecord {
                plugin_id: "example.layout".into(),
                entrypoint: "board".into(),
            },
        );

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: Some(original_tab_id),
                tab_label: Some("dev".into()),
                focus: true,
                root: LayoutNode::Pane {
                    pane: LayoutPane {
                        label: Some("editor".into()),
                        ..Default::default()
                    },
                },
                float_root: None,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::LayoutApply { .. }));
        assert!(!app.state.plugin_panes.contains_key(&replaced_pane));
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn layout_apply_rejects_invalid_deep_leaf_without_creating_tab() {
        let mut app = app_with_workspace();
        let original_tab_count = app.state.workspaces[0].tabs.len();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: Some(app.public_workspace_id(0)),
                tab_id: None,
                tab_label: Some("bad".into()),
                focus: false,
                root: LayoutNode::Split {
                    direction: SplitDirection::Right,
                    ratio: 0.5,
                    first: Box::new(LayoutNode::Pane {
                        pane: LayoutPane {
                            label: Some("editor".into()),
                            ..Default::default()
                        },
                    }),
                    second: Box::new(LayoutNode::Pane {
                        pane: LayoutPane {
                            command: Some(Vec::new()),
                            ..Default::default()
                        },
                    }),
                },
                float_root: None,
            },
        );

        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "invalid_layout");
        assert_eq!(app.state.workspaces[0].tabs.len(), original_tab_count);
    }

    #[test]
    fn layout_validation_rejects_too_many_panes() {
        let mut root = LayoutNode::Pane {
            pane: LayoutPane::default(),
        };
        for _ in 0..MAX_LAYOUT_PANES {
            root = LayoutNode::Split {
                direction: SplitDirection::Right,
                ratio: 0.5,
                first: Box::new(root),
                second: Box::new(LayoutNode::Pane {
                    pane: LayoutPane::default(),
                }),
            };
        }

        let err = validate_layout_trees(&[&root]).unwrap_err();
        assert!(err.contains("maximum"));
    }

    #[tokio::test]
    async fn layout_apply_rejects_a_combined_tree_over_the_pane_cap() {
        let flat_stack = |count: usize| LayoutNode::Stack {
            panes: (0..count).map(|_| LayoutPane::default()).collect(),
            active: 0,
        };
        // Each half fits on its own; together they exceed the cap, and
        // `layout.apply` spawns a PTY for every pane in both.
        let half = MAX_LAYOUT_PANES / 2 + 1;
        assert!(validate_layout_trees(&[&flat_stack(half)]).is_ok());

        let mut app = app_with_workspace();
        let original_tab_count = app.state.workspaces[0].tabs.len();

        let response = app.handle_layout_apply(
            "req".into(),
            LayoutApplyParams {
                workspace_id: None,
                tab_id: None,
                tab_label: Some("too big".into()),
                focus: false,
                root: flat_stack(half),
                float_root: Some(flat_stack(half)),
            },
        );

        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "invalid_layout");
        assert!(
            error.error.message.contains("more than 24 panes"),
            "{}",
            error.error.message
        );
        assert_eq!(app.state.workspaces[0].tabs.len(), original_tab_count);
    }
}
