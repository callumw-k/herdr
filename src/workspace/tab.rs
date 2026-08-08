use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::layout::{Direction, Rect};
use tokio::sync::{mpsc, Notify};

use crate::events::AppEvent;
use crate::layout::{arrange, Arrangement, Node, PaneId, TileLayout};
use crate::pane::{PaneLaunchEnv, PaneState};
use crate::render_signal::RenderSignal;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalRuntimeRegistry, TerminalState};

pub(crate) type DetachedPane = (PaneId, TerminalId);

pub(crate) struct MovedPane {
    pub pane_id: PaneId,
    pub pane_state: PaneState,
}

pub struct NewPane {
    pub pane_id: PaneId,
    pub terminal: TerminalState,
    pub runtime: TerminalRuntime,
}

enum SplitCommand<'a> {
    Shell {
        command: &'a str,
        launch_env: &'a PaneLaunchEnv,
    },
    Argv {
        argv: &'a [String],
        launch_env: &'a PaneLaunchEnv,
    },
}

pub struct Tab {
    pub custom_name: Option<String>,
    pub number: usize,
    /// Identity source for this tab's pane tree.
    pub root_pane: PaneId,
    pub layout: TileLayout,
    /// Pane viewport state — always present, testable without PTYs.
    pub panes: HashMap<PaneId, PaneState>,
    #[cfg(test)]
    pub runtimes: HashMap<PaneId, TerminalRuntime>,
    pub zoomed: bool,
    /// The floating layer's own layout, or None when no floats exist.
    /// `TileLayout::new()` allocates a pane, so an empty layer cannot hold one.
    pub float_layout: Option<TileLayout>,
    /// Defaults to Stacked so a second float looks as it did before this layer
    /// gained arrangements: one expanded, the rest as collapsed bars.
    pub float_arrangement: Arrangement,
    /// Hide the whole floating layer without closing anything.
    pub floats_hidden: bool,
    /// When true, keyboard focus is the focused float.
    pub float_focused: bool,
    /// The arrangement this tab's tiled panes are laid out under.
    pub arrangement: Arrangement,
    /// Set by tiled pane creation, closure and arrangement cycling.
    /// `compute_view` is the only place that knows the tab's real rect, so it
    /// performs the re-flow.
    pub needs_reflow: bool,
    /// The same for the floating layer. The layers carry separate flags because
    /// a re-flow rebuilds a layer under its arrangement and so discards manual
    /// sizing in it; a float mutation must not cost the tiled layer its dragged
    /// borders, nor the other way round.
    pub float_needs_reflow: bool,
    pub events: mpsc::Sender<AppEvent>,
    pub(crate) render_notify: Arc<Notify>,
    pub(crate) render_dirty: Arc<RenderSignal>,
}

impl Tab {
    // Tab construction threads pane runtime geometry, host context, and render hooks.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        number: usize,
        initial_cwd: PathBuf,
        rows: u16,
        cols: u16,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        shell_config: crate::pane::PaneShellConfig<'_>,
        launch_env: &PaneLaunchEnv,
        events: mpsc::Sender<AppEvent>,
        render_notify: Arc<Notify>,
        render_dirty: Arc<RenderSignal>,
    ) -> std::io::Result<(Self, TerminalState, TerminalRuntime)> {
        Self::new_with_runtime(
            number,
            initial_cwd,
            rows,
            cols,
            scrollback_limit_bytes,
            host_terminal_theme,
            host_terminal_appearance,
            shell_config,
            launch_env,
            events,
            render_notify,
            render_dirty,
            None,
        )
    }

    // Command tab construction mirrors the shell tab runtime arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn new_argv_command(
        number: usize,
        initial_cwd: PathBuf,
        rows: u16,
        cols: u16,
        argv: &[String],
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        launch_env: &PaneLaunchEnv,
        events: mpsc::Sender<AppEvent>,
        render_notify: Arc<Notify>,
        render_dirty: Arc<RenderSignal>,
    ) -> std::io::Result<(Self, TerminalState, TerminalRuntime)> {
        Self::new_with_runtime(
            number,
            initial_cwd,
            rows,
            cols,
            scrollback_limit_bytes,
            host_terminal_theme,
            host_terminal_appearance,
            crate::pane::PaneShellConfig::new("", crate::config::ShellModeConfig::NonLogin),
            launch_env,
            events,
            render_notify,
            render_dirty,
            Some(argv),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_runtime(
        number: usize,
        initial_cwd: PathBuf,
        rows: u16,
        cols: u16,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        shell_config: crate::pane::PaneShellConfig<'_>,
        launch_env: &PaneLaunchEnv,
        events: mpsc::Sender<AppEvent>,
        render_notify: Arc<Notify>,
        render_dirty: Arc<RenderSignal>,
        argv: Option<&[String]>,
    ) -> std::io::Result<(Self, TerminalState, TerminalRuntime)> {
        let (layout, root_id) = TileLayout::new();
        let runtime = if let Some(argv) = argv {
            TerminalRuntime::spawn_argv_command(
                root_id,
                rows,
                cols,
                initial_cwd.clone(),
                argv,
                launch_env,
                crate::pane::AgentDetection::Enabled,
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                events.clone(),
                render_notify.clone(),
                render_dirty.clone(),
            )?
        } else {
            TerminalRuntime::spawn(
                root_id,
                rows,
                cols,
                initial_cwd.clone(),
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                shell_config,
                launch_env,
                events.clone(),
                render_notify.clone(),
                render_dirty.clone(),
            )?
        };

        let terminal_id = TerminalId::alloc();
        let terminal = match argv {
            Some(argv) => {
                TerminalState::new(terminal_id.clone(), initial_cwd).with_launch_argv(argv.to_vec())
            }
            None => TerminalState::new(terminal_id.clone(), initial_cwd),
        };
        let mut panes = HashMap::new();
        panes.insert(root_id, PaneState::new(terminal_id));

        Ok((
            Self {
                custom_name: None,
                number,
                root_pane: root_id,
                layout,
                panes,
                #[cfg(test)]
                runtimes: HashMap::new(),
                zoomed: false,
                float_layout: None,
                float_arrangement: Arrangement::Stacked,
                floats_hidden: false,
                float_focused: false,
                arrangement: Arrangement::default(),
                needs_reflow: false,
                float_needs_reflow: false,
                events,
                render_notify,
                render_dirty,
            },
            terminal,
            runtime,
        ))
    }

    pub fn is_auto_named(&self) -> bool {
        self.custom_name.is_none()
    }

    pub fn set_custom_name(&mut self, name: String) {
        self.custom_name = Some(name);
    }

    pub fn is_float(&self, pane_id: PaneId) -> bool {
        self.float_layout
            .as_ref()
            .is_some_and(|layout| layout.pane_ids().contains(&pane_id))
    }

    /// The floating layer's panes, in tree order.
    pub fn floats(&self) -> Vec<PaneId> {
        self.float_layout
            .as_ref()
            .map(TileLayout::pane_ids)
            .unwrap_or_default()
    }

    /// Every pane in this tab, tiled then floating. Use this over `layout.pane_ids()`
    /// wherever "every pane" is meant rather than "every tiled pane".
    pub fn all_pane_ids(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.layout.pane_ids().into_iter().chain(self.floats())
    }

    /// The float that holds focus within the layer, or None when the layer is
    /// hidden. Replaces the old `top_float`: without a z-order there is no top.
    pub fn focused_float(&self) -> Option<PaneId> {
        if self.floats_hidden {
            return None;
        }
        self.float_layout.as_ref().map(TileLayout::focused)
    }

    /// Focus resolves to the focused float when the floating layer holds focus,
    /// otherwise to the tiled layer. `layout.focused()` keeps tracking the
    /// tiled focus independently, so returning to it needs no saved-focus field.
    /// The pane ids of whichever layer holds focus. Cycling and directional
    /// movement both stay within one layer, because the layers overlap.
    pub fn focused_layer_pane_ids(&self) -> Vec<PaneId> {
        if self.float_focused && !self.floats_hidden {
            self.floats()
        } else {
            self.layout.pane_ids()
        }
    }

    pub fn focused_pane(&self) -> PaneId {
        self.float_focused
            .then(|| self.focused_float())
            .flatten()
            .unwrap_or_else(|| self.layout.focused())
    }

    pub fn push_float(&mut self, pane_id: PaneId, pane_state: PaneState) {
        match self.float_layout.as_mut() {
            Some(layout) => {
                let target = layout.focused();
                // Placement here is provisional: the re-flow below rebuilds the
                // layer under float_arrangement. insert_pane_near declines when
                // `pane_id` is already in the layer (a re-push, matching the old
                // dedup-on-push behaviour) or when `target` is missing, which
                // can't happen since `target` is the layer's own focus. Only the
                // second case would orphan a live PTY in `self.panes` with no
                // slot in the tree, so fall back to a stack entry rather than
                // trust that invariant blindly.
                if !layout.insert_pane_near(target, pane_id, Direction::Vertical, 0.5, true)
                    && !layout.pane_ids().contains(&pane_id)
                {
                    let mut ids = layout.pane_ids();
                    ids.push(pane_id);
                    let active = ids.len() - 1;
                    *layout = TileLayout::from_saved(Node::Stack { panes: ids, active }, pane_id);
                }
            }
            None => {
                self.float_layout = Some(TileLayout::from_saved(Node::Pane(pane_id), pane_id));
            }
        }
        self.panes.insert(pane_id, pane_state);
        self.floats_hidden = false;
        self.float_focused = true;
        self.float_needs_reflow = true;
    }

    /// Unlike `detach_pane`, there is no last-pane guard: closing the final float
    /// is valid because the tiled layer still exists.
    pub fn close_float(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        let layout = self.float_layout.as_mut()?;
        if !layout.pane_ids().contains(&pane_id) {
            return None;
        }
        let previous = layout.focused();
        layout.focus_pane(pane_id);
        if layout.close_focused() {
            if previous != pane_id {
                layout.focus_pane(previous);
            }
        } else {
            // close_focused refuses the last pane, so the layer is now empty.
            self.float_layout = None;
            self.float_focused = false;
        }
        let pane = self.panes.remove(&pane_id)?;
        self.float_needs_reflow = true;
        Some((pane_id, pane.attached_terminal_id))
    }

    pub fn focus_floats(&mut self) -> bool {
        if self.float_layout.is_none() || (self.float_focused && !self.floats_hidden) {
            return false;
        }
        self.floats_hidden = false;
        self.float_focused = true;
        true
    }

    pub fn set_floats_hidden(&mut self, hidden: bool) -> bool {
        if self.floats_hidden == hidden {
            return false;
        }
        self.floats_hidden = hidden;
        if hidden {
            self.float_focused = false;
        } else if self.float_layout.is_some() {
            self.float_focused = true;
        }
        true
    }

    #[cfg(test)]
    pub fn split_focused(
        &mut self,
        direction: Direction,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        shell_config: crate::pane::PaneShellConfig<'_>,
        launch_env: &PaneLaunchEnv,
    ) -> std::io::Result<NewPane> {
        self.split_pane_with_runtime(
            self.layout.focused(),
            true,
            direction,
            None,
            rows,
            cols,
            cwd,
            scrollback_limit_bytes,
            host_terminal_theme,
            host_terminal_appearance,
            shell_config,
            launch_env,
            None,
        )
    }

    pub fn split_focused_command(
        &mut self,
        direction: Direction,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        command: &str,
        launch_env: &PaneLaunchEnv,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
    ) -> std::io::Result<NewPane> {
        self.split_pane_with_runtime(
            self.layout.focused(),
            true,
            direction,
            None,
            rows,
            cols,
            cwd,
            scrollback_limit_bytes,
            host_terminal_theme,
            host_terminal_appearance,
            crate::pane::PaneShellConfig::new("", crate::config::ShellModeConfig::NonLogin),
            launch_env,
            Some(SplitCommand::Shell {
                command,
                launch_env,
            }),
        )
    }

    /// Split `target` with a shell pane. Focus moves to the new pane only when
    /// `focus_new_pane` is set; a spawn failure rolls the layout back without
    /// touching focus or its history.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn split_pane_shell(
        &mut self,
        target: PaneId,
        focus_new_pane: bool,
        direction: Direction,
        ratio: Option<f32>,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        shell_config: crate::pane::PaneShellConfig<'_>,
        launch_env: &PaneLaunchEnv,
    ) -> std::io::Result<NewPane> {
        self.split_pane_with_runtime(
            target,
            focus_new_pane,
            direction,
            ratio,
            rows,
            cols,
            cwd,
            scrollback_limit_bytes,
            host_terminal_theme,
            host_terminal_appearance,
            shell_config,
            launch_env,
            None,
        )
    }

    /// Split `target` with an argv-command pane. Same focus contract as
    /// `split_pane_shell`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn split_pane_argv(
        &mut self,
        target: PaneId,
        focus_new_pane: bool,
        direction: Direction,
        ratio: Option<f32>,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        argv: &[String],
        launch_env: &PaneLaunchEnv,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
    ) -> std::io::Result<NewPane> {
        self.split_pane_with_runtime(
            target,
            focus_new_pane,
            direction,
            ratio,
            rows,
            cols,
            cwd,
            scrollback_limit_bytes,
            host_terminal_theme,
            host_terminal_appearance,
            crate::pane::PaneShellConfig::new("", crate::config::ShellModeConfig::NonLogin),
            launch_env,
            Some(SplitCommand::Argv { argv, launch_env }),
        )
    }

    // Split construction threads geometry, host context, launch policy, and command state.
    #[allow(clippy::too_many_arguments)]
    fn split_pane_with_runtime(
        &mut self,
        target: PaneId,
        focus_new_pane: bool,
        direction: Direction,
        ratio: Option<f32>,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        shell_config: crate::pane::PaneShellConfig<'_>,
        launch_env: &PaneLaunchEnv,
        command: Option<SplitCommand<'_>>,
    ) -> std::io::Result<NewPane> {
        let Some(new_id) = self
            .layout
            .split_pane(target, direction, ratio.unwrap_or(0.5))
        else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "split target pane is not in the layout",
            ));
        };
        let actual_cwd =
            cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let launch_argv = if let Some(SplitCommand::Argv { argv, .. }) = &command {
            Some((*argv).to_vec())
        } else {
            None
        };
        let runtime = match command {
            Some(SplitCommand::Shell {
                command,
                launch_env,
            }) => TerminalRuntime::spawn_shell_command(
                new_id,
                rows,
                cols,
                actual_cwd.clone(),
                command,
                launch_env,
                crate::pane::AgentDetection::Enabled,
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                self.events.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            ),
            Some(SplitCommand::Argv { argv, launch_env }) => TerminalRuntime::spawn_argv_command(
                new_id,
                rows,
                cols,
                actual_cwd.clone(),
                argv,
                launch_env,
                crate::pane::AgentDetection::Enabled,
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                self.events.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            ),
            None => TerminalRuntime::spawn(
                new_id,
                rows,
                cols,
                actual_cwd.clone(),
                scrollback_limit_bytes,
                host_terminal_theme,
                host_terminal_appearance,
                shell_config,
                launch_env,
                self.events.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
            ),
        };
        let runtime = match runtime {
            Ok(runtime) => runtime,
            Err(err) => {
                self.layout.close_pane(new_id);
                return Err(err);
            }
        };
        let terminal_id = TerminalId::alloc();
        let terminal = match launch_argv {
            Some(argv) => {
                TerminalState::new(terminal_id.clone(), actual_cwd).with_launch_argv(argv)
            }
            None => TerminalState::new(terminal_id.clone(), actual_cwd),
        };
        if focus_new_pane {
            self.layout.focus_pane(new_id);
        }
        self.panes.insert(new_id, PaneState::new(terminal_id));
        self.zoomed = false;
        self.needs_reflow = true;
        Ok(NewPane {
            pane_id: new_id,
            terminal,
            runtime,
        })
    }

    #[cfg(test)]
    pub fn close_focused(&mut self) -> Option<DetachedPane> {
        let pane_id = self.layout.focused();
        self.detach_pane(pane_id)
    }

    pub fn close_pane(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        self.detach_pane(pane_id)
    }

    pub fn remove_pane(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        self.detach_pane(pane_id)
    }

    pub(crate) fn from_existing_pane(
        number: usize,
        custom_name: Option<String>,
        moved: MovedPane,
        events: mpsc::Sender<AppEvent>,
        render_notify: Arc<Notify>,
        render_dirty: Arc<RenderSignal>,
    ) -> Self {
        let mut panes = HashMap::new();
        let pane_id = moved.pane_id;
        panes.insert(pane_id, moved.pane_state);
        Self {
            custom_name,
            number,
            root_pane: pane_id,
            layout: TileLayout::from_saved(Node::Pane(pane_id), pane_id),
            panes,
            #[cfg(test)]
            runtimes: HashMap::new(),
            zoomed: false,
            float_layout: None,
            float_arrangement: Arrangement::Stacked,
            floats_hidden: false,
            float_focused: false,
            arrangement: Arrangement::default(),
            needs_reflow: false,
            float_needs_reflow: false,
            events,
            render_notify,
            render_dirty,
        }
    }

    pub(crate) fn take_pane_for_move(&mut self, pane_id: PaneId) -> Option<MovedPane> {
        if !self.panes.contains_key(&pane_id) {
            return None;
        }
        if self.is_float(pane_id) {
            return None;
        }

        if self.layout.pane_count() > 1 {
            let next_root = self.promoted_root_if_needed(pane_id);
            self.layout.close_pane(pane_id);
            if let Some(next_root) = next_root {
                self.root_pane = next_root;
            }
        }

        let pane_state = self.panes.remove(&pane_id)?;
        self.zoomed = false;
        self.needs_reflow = true;
        Some(MovedPane {
            pane_id,
            pane_state,
        })
    }

    pub(crate) fn insert_existing_pane(
        &mut self,
        target_pane_id: PaneId,
        moved: MovedPane,
        direction: Direction,
        ratio: f32,
        focus: bool,
    ) -> Result<PaneId, MovedPane> {
        if !self
            .layout
            .insert_pane_near(target_pane_id, moved.pane_id, direction, ratio, focus)
        {
            return Err(moved);
        }
        let pane_id = moved.pane_id;
        self.panes.insert(pane_id, moved.pane_state);
        self.zoomed = false;
        self.needs_reflow = true;
        Ok(pane_id)
    }

    fn detach_pane(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        if self.layout.pane_count() <= 1 {
            return None;
        }

        let next_root = self.promoted_root_if_needed(pane_id);

        self.layout.close_pane(pane_id);

        let pane = self.panes.remove(&pane_id)?;
        let terminal_id = pane.attached_terminal_id;
        self.zoomed = false;
        self.needs_reflow = true;
        if let Some(next_root) = next_root {
            self.root_pane = next_root;
        }
        Some((pane_id, terminal_id))
    }

    fn promoted_root_if_needed(&self, closing: PaneId) -> Option<PaneId> {
        if self.root_pane != closing {
            return None;
        }
        self.layout.pane_ids().into_iter().find(|id| *id != closing)
    }

    pub fn terminal_id(&self, pane_id: PaneId) -> Option<&TerminalId> {
        self.panes
            .get(&pane_id)
            .map(|pane| &pane.attached_terminal_id)
    }

    pub fn cycle_arrangement(&mut self, forward: bool) {
        self.arrangement = if forward {
            self.arrangement.next()
        } else {
            self.arrangement.previous()
        };
        self.needs_reflow = true;
    }

    /// Regenerate whichever layers are dirty under their arrangements,
    /// preserving pane order and focus. `float_region` is None when the region
    /// cannot be resolved, which leaves the float layer dirty so it re-flows
    /// once the terminal is big enough to hold one.
    pub fn reflow(&mut self, area: Rect, float_region: Option<Rect>) {
        if self.needs_reflow {
            self.needs_reflow = false;
            let panes = self.layout.pane_ids();
            let focus = self.layout.focused();
            if let Some(root) = arrange(self.arrangement, &panes, focus, area) {
                self.layout = TileLayout::from_saved(root, focus);
            }
        }

        if !self.float_needs_reflow {
            return;
        }
        let Some((float_panes, float_focus)) = self
            .float_layout
            .as_ref()
            .map(|layout| (layout.pane_ids(), layout.focused()))
        else {
            self.float_needs_reflow = false;
            return;
        };
        let Some(region) = float_region else {
            return;
        };
        self.float_needs_reflow = false;
        if let Some(root) = arrange(self.float_arrangement, &float_panes, float_focus, region) {
            self.float_layout = Some(TileLayout::from_saved(root, float_focus));
        }
    }

    pub fn cwd_for_pane(
        &self,
        pane_id: PaneId,
        terminals: &HashMap<TerminalId, TerminalState>,
        terminal_runtimes: &TerminalRuntimeRegistry,
    ) -> Option<PathBuf> {
        let terminal_id = self.terminal_id(pane_id)?;
        terminal_runtimes
            .get(terminal_id)
            .and_then(|rt| rt.cwd())
            .or_else(|| {
                terminals
                    .get(terminal_id)
                    .map(|terminal| terminal.cwd.clone())
            })
    }

    pub fn foreground_cwd_for_pane(
        &self,
        pane_id: PaneId,
        terminal_runtimes: &TerminalRuntimeRegistry,
    ) -> Option<PathBuf> {
        let terminal_id = self.terminal_id(pane_id)?;
        terminal_runtimes
            .get(terminal_id)
            .and_then(|rt| rt.foreground_cwd())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Arrangement, Node};
    use crate::workspace::Workspace;
    use ratatui::layout::Direction;

    fn test_tab() -> Tab {
        let ws = crate::workspace::Workspace::test_new("float-test");
        ws.tabs
            .into_iter()
            .next()
            .expect("test workspace has a tab")
    }

    #[test]
    fn a_tab_starts_with_no_float_layout() {
        let tab = test_tab();
        assert!(tab.float_layout.is_none());
        assert_eq!(tab.float_arrangement, Arrangement::Stacked);
        assert!(tab.floats().is_empty());
    }

    #[test]
    fn pushing_the_first_float_creates_the_layout() {
        let mut tab = test_tab();
        let first = PaneId::alloc();
        tab.push_float(first, PaneState::new(TerminalId::alloc()));
        assert_eq!(tab.floats(), vec![first]);
        assert_eq!(tab.focused_float(), Some(first));
        assert!(tab.float_focused);
    }

    #[test]
    fn pushing_more_floats_keeps_them_all_in_order() {
        let mut tab = test_tab();
        let ids: Vec<_> = (0..3).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        assert_eq!(tab.floats(), ids);
        assert_eq!(tab.focused_float(), Some(ids[2]));
    }

    #[test]
    fn focused_float_is_none_while_the_layer_is_hidden() {
        let mut tab = test_tab();
        tab.push_float(PaneId::alloc(), PaneState::new(TerminalId::alloc()));
        tab.set_floats_hidden(true);
        assert_eq!(tab.focused_float(), None);
    }

    #[test]
    fn closing_a_float_removes_it_and_keeps_the_others() {
        let mut tab = test_tab();
        let ids: Vec<_> = (0..3).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        assert!(tab.close_float(ids[1]).is_some());
        assert_eq!(tab.floats(), vec![ids[0], ids[2]]);
        assert!(tab.float_layout.is_some());
    }

    #[test]
    fn closing_the_last_float_clears_the_layout_and_focus() {
        let mut tab = test_tab();
        let only = PaneId::alloc();
        tab.push_float(only, PaneState::new(TerminalId::alloc()));
        assert!(tab.close_float(only).is_some());
        assert!(tab.float_layout.is_none());
        assert!(!tab.float_focused);
        assert!(tab.floats().is_empty());
    }

    #[test]
    fn closing_a_pane_that_is_not_a_float_returns_none() {
        let mut tab = test_tab();
        tab.push_float(PaneId::alloc(), PaneState::new(TerminalId::alloc()));
        assert!(tab.close_float(PaneId::alloc()).is_none());
    }

    #[test]
    fn is_float_and_all_pane_ids_see_the_float_layout() {
        let mut tab = test_tab();
        let tiled = tab.layout.focused();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        assert!(tab.is_float(float));
        assert!(!tab.is_float(tiled));
        let all: Vec<_> = tab.all_pane_ids().collect();
        assert_eq!(all, vec![tiled, float]);
    }

    #[test]
    fn focused_pane_prefers_the_focused_float() {
        let mut tab = test_tab();
        let tiled = tab.layout.focused();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        assert_eq!(tab.focused_pane(), float);
        tab.float_focused = false;
        assert_eq!(tab.focused_pane(), tiled);
    }

    #[test]
    fn push_float_with_duplicate_id_does_not_duplicate_stack_entry() {
        let mut tab = test_tab();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        let updated_terminal = TerminalId::alloc();
        tab.push_float(float, PaneState::new(updated_terminal.clone()));

        assert_eq!(tab.floats(), vec![float]);
        assert_eq!(
            tab.panes.get(&float).map(|p| &p.attached_terminal_id),
            Some(&updated_terminal)
        );
    }

    #[test]
    fn focus_floats_does_nothing_without_a_float() {
        let mut tab = test_tab();

        assert!(!tab.focus_floats());
        assert!(!tab.float_focused);
    }

    #[test]
    fn focus_floats_unhides_and_focuses_an_existing_float() {
        let mut tab = test_tab();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        tab.set_floats_hidden(true);

        assert!(tab.focus_floats());

        assert!(!tab.floats_hidden);
        assert!(tab.float_focused);
        assert_eq!(tab.focused_pane(), float);
    }

    #[test]
    fn focus_floats_does_nothing_when_already_focused() {
        let mut tab = test_tab();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));

        assert!(!tab.focus_floats(), "already focused, nothing to do");
    }

    #[test]
    fn close_float_drops_state_and_clears_focus_when_last() {
        let mut tab = test_tab();
        let tiled = tab.layout.focused();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));

        assert!(tab.close_float(float).is_some());

        assert!(tab.floats().is_empty());
        assert!(!tab.panes.contains_key(&float));
        assert!(!tab.float_focused);
        assert_eq!(tab.focused_pane(), tiled);
    }

    #[test]
    fn close_float_ignores_a_tiled_pane() {
        let mut tab = test_tab();
        let tiled = tab.layout.focused();

        assert!(tab.close_float(tiled).is_none());

        assert!(tab.panes.contains_key(&tiled), "tiled pane must survive");
    }

    #[test]
    fn hiding_the_layer_clears_float_focus() {
        let mut tab = test_tab();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        assert!(tab.float_focused);

        tab.set_floats_hidden(true);

        assert!(!tab.float_focused);
    }

    #[test]
    fn showing_the_layer_restores_float_focus() {
        let mut tab = test_tab();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        tab.set_floats_hidden(true);

        tab.set_floats_hidden(false);

        assert!(tab.float_focused);
        assert_eq!(tab.focused_pane(), float);
    }

    #[test]
    fn a_new_tab_starts_in_the_grid_arrangement() {
        let workspace = Workspace::test_new("arrangements");
        let tab = &workspace.tabs[0];
        assert_eq!(tab.arrangement, Arrangement::Grid);
        assert!(!tab.needs_reflow);
    }

    #[test]
    fn cycling_the_arrangement_marks_the_tab_for_reflow() {
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        tab.cycle_arrangement(true);
        assert_eq!(tab.arrangement, Arrangement::Stacked);
        assert!(tab.needs_reflow);
    }

    #[test]
    fn cycling_backwards_walks_the_other_way() {
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        tab.cycle_arrangement(false);
        assert_eq!(tab.arrangement, Arrangement::Horizontal);
    }

    #[test]
    fn reflow_rebuilds_the_tree_and_clears_the_flag() {
        let area = Rect::new(0, 0, 80, 20);
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        let first = tab.layout.focused();
        let second = tab.layout.split_focused(Direction::Horizontal);
        tab.arrangement = Arrangement::Stacked;
        tab.needs_reflow = true;

        tab.reflow(area, None);

        assert!(!tab.needs_reflow);
        assert_eq!(tab.layout.pane_ids(), vec![first, second]);
        assert!(matches!(tab.layout.root(), Node::Stack { .. }));
    }

    #[test]
    fn reflow_keeps_focus_on_the_same_pane() {
        let area = Rect::new(0, 0, 80, 20);
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        let second = tab.layout.split_focused(Direction::Horizontal);
        tab.arrangement = Arrangement::Horizontal;
        tab.needs_reflow = true;

        tab.reflow(area, None);

        assert_eq!(tab.layout.focused(), second);
    }

    #[test]
    fn reflow_is_a_no_op_when_the_flag_is_clear() {
        let area = Rect::new(0, 0, 80, 20);
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        tab.layout.split_focused(Direction::Horizontal);
        tab.arrangement = Arrangement::Stacked;

        tab.reflow(area, None);

        assert!(!matches!(tab.layout.root(), Node::Stack { .. }));
    }

    #[test]
    fn reflow_rebuilds_the_float_layer_under_its_arrangement() {
        let area = Rect::new(0, 0, 80, 20);
        let region = Rect::new(10, 5, 40, 10);
        let mut tab = test_tab();
        let ids: Vec<_> = (0..3).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        tab.float_arrangement = Arrangement::Stacked;
        tab.float_needs_reflow = true;

        tab.reflow(area, Some(region));

        assert!(!tab.float_needs_reflow);
        assert_eq!(tab.floats(), ids, "float order survives a re-flow");
        let root = tab.float_layout.as_ref().expect("a float layout").root();
        assert!(matches!(root, Node::Stack { .. }));
    }

    #[test]
    fn reflow_keeps_float_focus_on_the_same_pane() {
        let area = Rect::new(0, 0, 80, 20);
        let region = Rect::new(10, 5, 40, 10);
        let mut tab = test_tab();
        let ids: Vec<_> = (0..3).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        let focused = ids[1];
        tab.float_layout
            .as_mut()
            .expect("a float layout")
            .focus_pane(focused);
        tab.float_arrangement = Arrangement::Grid;
        tab.float_needs_reflow = true;

        tab.reflow(area, Some(region));

        assert_eq!(tab.focused_float(), Some(focused));
    }

    #[test]
    fn reflow_without_a_float_region_leaves_the_float_layer_alone() {
        let area = Rect::new(0, 0, 80, 20);
        let mut tab = test_tab();
        let ids: Vec<_> = (0..2).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        tab.float_needs_reflow = true;

        tab.reflow(area, None);

        assert_eq!(
            tab.floats(),
            ids,
            "no region means no float geometry to apply"
        );
    }

    #[test]
    fn closing_a_pane_marks_the_tab_for_reflow() {
        let mut workspace = Workspace::test_new("arrangements");
        workspace.test_split(Direction::Horizontal);
        let tab = &mut workspace.tabs[0];
        assert!(!tab.needs_reflow);

        assert!(tab.close_focused().is_some());

        assert!(tab.needs_reflow);
    }

    #[test]
    fn splitting_a_stacked_tab_keeps_the_new_pane_in_the_tree() {
        let area = Rect::new(0, 0, 80, 20);
        let mut workspace = Workspace::test_new("arrangements");
        workspace.test_split(Direction::Horizontal);
        let tab = &mut workspace.tabs[0];
        tab.arrangement = Arrangement::Stacked;
        tab.needs_reflow = true;
        tab.reflow(area, None);

        let new_pane = workspace.test_split(Direction::Horizontal);

        let tab = &mut workspace.tabs[0];
        assert!(tab.layout.pane_ids().contains(&new_pane));
        assert_eq!(tab.layout.pane_ids().len(), tab.panes.len());
        assert!(tab.layout.pane_ids().contains(&tab.layout.focused()));
        assert!(tab.close_focused().is_some());
    }

    #[test]
    fn inserting_an_existing_pane_marks_the_tab_for_reflow() {
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        let target = tab.layout.focused();
        let moved = MovedPane {
            pane_id: PaneId::alloc(),
            pane_state: PaneState::new(TerminalId::alloc()),
        };
        assert!(!tab.needs_reflow);

        assert!(tab
            .insert_existing_pane(target, moved, Direction::Horizontal, 0.5, true)
            .is_ok());

        assert!(tab.needs_reflow);
    }

    #[test]
    fn taking_a_pane_for_move_marks_the_tab_for_reflow() {
        let mut workspace = Workspace::test_new("arrangements");
        let second = workspace.test_split(Direction::Horizontal);
        let tab = &mut workspace.tabs[0];
        assert!(!tab.needs_reflow);

        assert!(tab.take_pane_for_move(second).is_some());

        assert!(tab.needs_reflow);
    }

    /// The ratio of the tiled root split, or None when the root is not a split.
    fn root_ratio(layout: &TileLayout) -> Option<f32> {
        match layout.root() {
            Node::Split { ratio, .. } => Some(*ratio),
            _ => None,
        }
    }

    #[test]
    fn opening_a_float_leaves_manual_tiled_sizing_alone() {
        let area = Rect::new(0, 0, 80, 20);
        let region = Rect::new(10, 5, 40, 10);
        let mut workspace = Workspace::test_new("layers");
        workspace.test_split(Direction::Horizontal);
        let tab = &mut workspace.tabs[0];
        tab.needs_reflow = false;
        assert!(tab.layout.set_ratio_at(&[], 0.8));

        tab.push_float(PaneId::alloc(), PaneState::new(TerminalId::alloc()));
        tab.reflow(area, Some(region));

        assert_eq!(
            root_ratio(&tab.layout),
            Some(0.8),
            "a float mutation must not re-flow the tiled layer"
        );
    }

    #[test]
    fn creating_a_tiled_pane_leaves_an_applied_float_shape_alone() {
        let area = Rect::new(0, 0, 80, 20);
        let region = Rect::new(10, 5, 40, 10);
        let mut workspace = Workspace::test_new("layers");
        let tab = &mut workspace.tabs[0];
        let floats: Vec<_> = (0..2).map(|_| PaneId::alloc()).collect();
        for id in &floats {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        // Stands in for `layout.apply float_root`: a custom float tree with a
        // ratio no arrangement would produce.
        tab.float_layout = Some(TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.25,
                first: Box::new(Node::Pane(floats[0])),
                second: Box::new(Node::Pane(floats[1])),
            },
            floats[0],
        ));
        tab.needs_reflow = false;
        tab.float_needs_reflow = false;

        let moved = MovedPane {
            pane_id: PaneId::alloc(),
            pane_state: PaneState::new(TerminalId::alloc()),
        };
        let target = tab.layout.focused();
        assert!(tab
            .insert_existing_pane(target, moved, Direction::Horizontal, 0.5, true)
            .is_ok());
        tab.reflow(area, Some(region));

        let float_layout = tab.float_layout.as_ref().expect("a float layout");
        assert_eq!(
            root_ratio(float_layout),
            Some(0.25),
            "a tiled mutation must not re-flow the float layer"
        );
    }

    #[test]
    fn an_unresolvable_float_region_keeps_the_layer_dirty() {
        let area = Rect::new(0, 0, 80, 20);
        let region = Rect::new(10, 5, 40, 10);
        let mut tab = test_tab();
        let floats: Vec<_> = (0..2).map(|_| PaneId::alloc()).collect();
        for id in &floats {
            tab.push_float(*id, PaneState::new(TerminalId::alloc()));
        }
        tab.float_arrangement = Arrangement::Vertical;

        // The terminal is too small to hold a float region, so nothing was
        // re-flowed and the layer must stay dirty.
        tab.reflow(area, None);
        assert!(tab.float_needs_reflow);

        tab.reflow(area, Some(region));
        assert!(!tab.float_needs_reflow);
        let root = tab.float_layout.as_ref().expect("a float layout").root();
        assert!(matches!(root, Node::Split { .. }));
    }
}
