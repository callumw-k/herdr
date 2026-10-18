use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::layout::{Direction, Rect};
use tokio::sync::{mpsc, Notify};

use crate::events::AppEvent;
use crate::layout::{arrange, Arrangement, Node, PaneId, TileLayout};
use crate::pane::{PaneLaunchEnv, PaneState};
use crate::render_signal::RenderSignal;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};

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
    pub parent: Option<usize>,
    /// Identity source for this tab's pane tree.
    pub root_pane: PaneId,
    pub layout: TileLayout,
    /// Pane viewport state — always present, testable without PTYs.
    pub panes: HashMap<PaneId, PaneState>,
    #[cfg(test)]
    pub runtimes: HashMap<PaneId, TerminalRuntime>,
    pub zoomed: bool,
    /// The arrangement this tab's tiled panes are laid out under.
    pub arrangement: Arrangement,
    /// Set by tiled pane creation, closure and arrangement cycling.
    /// `compute_view` is the only place that knows the tab's real rect, so it
    /// performs the re-flow.
    pub needs_reflow: bool,
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
                parent: None,
                root_pane: root_id,
                layout,
                panes,
                #[cfg(test)]
                runtimes: HashMap::new(),
                zoomed: false,
                arrangement: Arrangement::default(),
                needs_reflow: false,
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
            parent: None,
            root_pane: pane_id,
            layout: TileLayout::from_saved(Node::Pane(pane_id), pane_id),
            panes,
            #[cfg(test)]
            runtimes: HashMap::new(),
            zoomed: false,
            arrangement: Arrangement::default(),
            needs_reflow: false,
            events,
            render_notify,
            render_dirty,
        }
    }

    pub(crate) fn take_pane_for_move(&mut self, pane_id: PaneId) -> Option<MovedPane> {
        if !self.panes.contains_key(&pane_id) {
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

    /// Regenerate the tiled layer under its arrangement when it is dirty,
    /// preserving pane order and focus.
    pub fn reflow(&mut self, area: Rect) {
        if self.needs_reflow {
            self.needs_reflow = false;
            let panes = self.layout.pane_ids();
            let focus = self.layout.focused();
            if let Some(root) = arrange(self.arrangement, &panes, focus, area) {
                self.layout = TileLayout::from_saved(root, focus);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Arrangement, Node};
    use crate::workspace::Workspace;
    use ratatui::layout::Direction;

    #[test]
    fn a_new_tab_starts_in_the_stacked_arrangement() {
        let workspace = Workspace::test_new("arrangements");
        let tab = &workspace.tabs[0];
        assert_eq!(tab.arrangement, Arrangement::Stacked);
        assert!(!tab.needs_reflow);
    }

    #[test]
    fn cycling_the_arrangement_marks_the_tab_for_reflow() {
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        tab.cycle_arrangement(true);
        assert_eq!(tab.arrangement, Arrangement::Vertical);
        assert!(tab.needs_reflow);
    }

    #[test]
    fn cycling_backwards_walks_the_other_way() {
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        tab.cycle_arrangement(false);
        assert_eq!(tab.arrangement, Arrangement::Grid);
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

        tab.reflow(area);

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

        tab.reflow(area);

        assert_eq!(tab.layout.focused(), second);
    }

    #[test]
    fn reflow_is_a_no_op_when_the_flag_is_clear() {
        let area = Rect::new(0, 0, 80, 20);
        let mut workspace = Workspace::test_new("arrangements");
        let tab = &mut workspace.tabs[0];
        tab.layout.split_focused(Direction::Horizontal);
        tab.arrangement = Arrangement::Stacked;

        tab.reflow(area);

        assert!(!matches!(tab.layout.root(), Node::Stack { .. }));
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
        tab.reflow(area);

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
        workspace.tabs[0].needs_reflow = false;
        assert!(workspace.tabs[0].layout.set_ratio_at(&[], 0.8));

        workspace
            .floats
            .push(PaneId::alloc(), PaneState::new(TerminalId::alloc()));
        workspace.tabs[0].reflow(area);
        workspace.floats.reflow(Some(region));
        let tab = &workspace.tabs[0];

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
        let floats: Vec<_> = (0..2).map(|_| PaneId::alloc()).collect();
        for id in &floats {
            workspace
                .floats
                .push(*id, PaneState::new(TerminalId::alloc()));
        }
        // Stands in for `layout.apply float_root`: a custom float tree with a
        // ratio no arrangement would produce.
        workspace.floats.layout = Some(TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.25,
                first: Box::new(Node::Pane(floats[0])),
                second: Box::new(Node::Pane(floats[1])),
            },
            floats[0],
        ));
        workspace.floats.needs_reflow = false;
        let tab = &mut workspace.tabs[0];
        tab.needs_reflow = false;

        let moved = MovedPane {
            pane_id: PaneId::alloc(),
            pane_state: PaneState::new(TerminalId::alloc()),
        };
        let target = tab.layout.focused();
        assert!(tab
            .insert_existing_pane(target, moved, Direction::Horizontal, 0.5, true)
            .is_ok());
        tab.reflow(area);
        workspace.floats.reflow(Some(region));

        let float_layout = workspace.floats.layout.as_ref().expect("a float layout");
        assert_eq!(
            root_ratio(float_layout),
            Some(0.25),
            "a tiled mutation must not re-flow the float layer"
        );
    }
}
