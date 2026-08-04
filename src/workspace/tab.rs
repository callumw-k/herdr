use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::layout::Direction;
use tokio::sync::{mpsc, Notify};

use crate::events::AppEvent;
use crate::layout::{Node, PaneId, TileLayout};
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
    /// Floating panes over the tiled layer, back to front. Last is topmost.
    pub floats: Vec<PaneId>,
    /// Hide the whole floating layer without closing anything.
    pub floats_hidden: bool,
    /// When true, keyboard focus is the topmost visible float.
    pub float_focused: bool,
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
                floats: Vec::new(),
                floats_hidden: false,
                float_focused: false,
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

    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn is_float(&self, pane_id: PaneId) -> bool {
        self.floats.contains(&pane_id)
    }

    /// Every pane in this tab, tiled then floating. Use this over `layout.pane_ids()`
    /// wherever "every pane" is meant rather than "every tiled pane".
    pub fn all_pane_ids(&self) -> impl Iterator<Item = PaneId> + '_ {
        self.layout
            .pane_ids()
            .into_iter()
            .chain(self.floats.iter().copied())
    }

    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn top_float(&self) -> Option<PaneId> {
        if self.floats_hidden {
            return None;
        }
        self.floats.last().copied()
    }

    /// Focus resolves to the topmost visible float when the floating layer holds
    /// focus, otherwise to the tiled layer. `layout.focused()` keeps tracking the
    /// tiled focus independently, so returning to it needs no saved-focus field.
    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn focused_pane(&self) -> PaneId {
        self.float_focused
            .then(|| self.top_float())
            .flatten()
            .unwrap_or_else(|| self.layout.focused())
    }

    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn push_float(&mut self, pane_id: PaneId, pane_state: PaneState) {
        if !self.floats.contains(&pane_id) {
            self.floats.push(pane_id);
        }
        self.panes.insert(pane_id, pane_state);
        self.floats_hidden = false;
        self.float_focused = true;
    }

    /// Unlike `detach_pane`, there is no last-pane guard: closing the final float
    /// is valid because the tiled layer still exists.
    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn close_float(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        let position = self.floats.iter().position(|id| *id == pane_id)?;
        self.floats.remove(position);
        let pane = self.panes.remove(&pane_id)?;
        if self.floats.is_empty() {
            self.float_focused = false;
        }
        Some((pane_id, pane.attached_terminal_id))
    }

    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn cycle_floats(&mut self, forward: bool) -> bool {
        if self.floats.len() < 2 && !self.floats_hidden {
            return false;
        }
        if self.floats.is_empty() {
            return false;
        }
        self.floats_hidden = false;
        if forward {
            self.floats.rotate_right(1);
        } else {
            self.floats.rotate_left(1);
        }
        self.float_focused = true;
        true
    }

    /// Bring the floating layer into focus without creating or closing anything.
    /// Returns false when there is nothing to do: no floats exist, or the
    /// layer is already focused and visible.
    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn focus_floats(&mut self) -> bool {
        if self.floats.is_empty() || (self.float_focused && !self.floats_hidden) {
            return false;
        }
        self.floats_hidden = false;
        self.float_focused = true;
        true
    }

    #[allow(dead_code)] // consumed by later floating-panes tasks (input/render layers)
    pub fn set_floats_hidden(&mut self, hidden: bool) -> bool {
        if self.floats_hidden == hidden {
            return false;
        }
        self.floats_hidden = hidden;
        if hidden {
            self.float_focused = false;
        } else if !self.floats.is_empty() {
            self.float_focused = true;
        }
        true
    }

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
        self.split_focused_with_runtime(
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

    pub fn split_focused_with_ratio(
        &mut self,
        direction: Direction,
        ratio: f32,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
        shell_config: crate::pane::PaneShellConfig<'_>,
        launch_env: &PaneLaunchEnv,
    ) -> std::io::Result<NewPane> {
        self.split_focused_with_runtime(
            direction,
            Some(ratio),
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
        self.split_focused_with_runtime(
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

    pub fn split_focused_argv_command(
        &mut self,
        direction: Direction,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        argv: &[String],
        launch_env: &PaneLaunchEnv,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
    ) -> std::io::Result<NewPane> {
        self.split_focused_with_runtime(
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
            Some(SplitCommand::Argv { argv, launch_env }),
        )
    }

    pub fn split_focused_argv_command_with_ratio(
        &mut self,
        direction: Direction,
        ratio: f32,
        rows: u16,
        cols: u16,
        cwd: Option<PathBuf>,
        argv: &[String],
        launch_env: &PaneLaunchEnv,
        scrollback_limit_bytes: usize,
        host_terminal_theme: crate::terminal_theme::TerminalTheme,
        host_terminal_appearance: Option<crate::terminal_theme::HostAppearance>,
    ) -> std::io::Result<NewPane> {
        self.split_focused_with_runtime(
            direction,
            Some(ratio),
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
    fn split_focused_with_runtime(
        &mut self,
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
        let previous_focus = self.layout.focused();
        let new_id = match ratio {
            Some(ratio) => self.layout.split_focused_with_ratio(direction, ratio),
            None => self.layout.split_focused(direction),
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
                self.layout.close_focused();
                self.layout.focus_pane(previous_focus);
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
        self.panes.insert(new_id, PaneState::new(terminal_id));
        self.zoomed = false;
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
            floats: Vec::new(),
            floats_hidden: false,
            float_focused: false,
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
            if self.layout.focused() == pane_id {
                self.layout.close_focused();
            } else {
                let prev_focus = self.layout.focused();
                self.layout.focus_pane(pane_id);
                self.layout.close_focused();
                self.layout.focus_pane(prev_focus);
            }
            if let Some(next_root) = next_root {
                self.root_pane = next_root;
            }
        }

        let pane_state = self.panes.remove(&pane_id)?;
        self.zoomed = false;
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
    ) -> Result<PaneId, MovedPane> {
        if !self
            .layout
            .insert_pane_near(target_pane_id, moved.pane_id, direction, ratio)
        {
            return Err(moved);
        }
        let pane_id = moved.pane_id;
        self.panes.insert(pane_id, moved.pane_state);
        self.zoomed = false;
        Ok(pane_id)
    }

    fn detach_pane(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        if self.layout.pane_count() <= 1 {
            return None;
        }

        let next_root = self.promoted_root_if_needed(pane_id);

        if self.layout.focused() == pane_id {
            self.layout.close_focused();
        } else {
            let prev_focus = self.layout.focused();
            self.layout.focus_pane(pane_id);
            self.layout.close_focused();
            self.layout.focus_pane(prev_focus);
        }

        let pane = self.panes.remove(&pane_id)?;
        let terminal_id = pane.attached_terminal_id;
        self.zoomed = false;
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

    fn test_tab() -> Tab {
        let ws = crate::workspace::Workspace::test_new("float-test");
        ws.tabs
            .into_iter()
            .next()
            .expect("test workspace has a tab")
    }

    #[test]
    fn focused_pane_prefers_top_float_when_float_focused() {
        let mut tab = test_tab();
        let tiled = tab.layout.focused();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));

        assert_eq!(tab.focused_pane(), float);
        assert_eq!(tab.layout.focused(), tiled, "tiled focus is remembered");

        tab.set_floats_hidden(true);
        assert_eq!(tab.focused_pane(), tiled);
    }

    #[test]
    fn top_float_is_last_pushed_and_hidden_layer_has_none() {
        let mut tab = test_tab();
        let first = PaneId::alloc();
        let second = PaneId::alloc();
        tab.push_float(first, PaneState::new(TerminalId::alloc()));
        tab.push_float(second, PaneState::new(TerminalId::alloc()));

        assert_eq!(tab.top_float(), Some(second));

        tab.set_floats_hidden(true);
        assert_eq!(tab.top_float(), None);
    }

    #[test]
    fn push_float_with_duplicate_id_does_not_duplicate_stack_entry() {
        let mut tab = test_tab();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));
        let updated_terminal = TerminalId::alloc();
        tab.push_float(float, PaneState::new(updated_terminal.clone()));

        assert_eq!(tab.floats, vec![float]);
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
    fn cycle_floats_rotates_stack_and_unhides() {
        let mut tab = test_tab();
        let first = PaneId::alloc();
        let second = PaneId::alloc();
        tab.push_float(first, PaneState::new(TerminalId::alloc()));
        tab.push_float(second, PaneState::new(TerminalId::alloc()));
        tab.set_floats_hidden(true);

        assert!(tab.cycle_floats(true));

        assert!(!tab.floats_hidden, "cycling brings the layer back");
        assert_eq!(tab.top_float(), Some(first));
        assert_eq!(tab.focused_pane(), first);
    }

    #[test]
    fn close_float_drops_state_and_clears_focus_when_last() {
        let mut tab = test_tab();
        let tiled = tab.layout.focused();
        let float = PaneId::alloc();
        tab.push_float(float, PaneState::new(TerminalId::alloc()));

        assert!(tab.close_float(float).is_some());

        assert!(tab.floats.is_empty());
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
}
