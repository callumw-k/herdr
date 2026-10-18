use std::collections::{HashMap, HashSet};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

pub(super) struct AgentRow {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    /// Workspace label drawn above the first agent of each workspace in grouped sort.
    pub(super) group: Option<String>,
    /// Blank line above the group header; every group after the first gets one.
    pub(super) group_gap: bool,
    pub(super) depth: usize,
    /// One entry per tree column: whether a later row at that column's depth
    /// follows within the same branch and workspace group.
    pub(super) continues: Vec<bool>,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

impl AgentRow {
    pub(super) fn lines(&self) -> usize {
        self.rows.len().max(1) + usize::from(self.group.is_some()) + usize::from(self.group_gap)
    }
}

#[derive(Clone, Copy)]
pub(super) struct AgentListOrder {
    pub(super) sort: crate::config::AgentPanelSortConfig,
    pub(super) follows_workspace: bool,
}

impl ClientShellConfig {
    pub(super) fn agent_list_order(&self) -> AgentListOrder {
        AgentListOrder {
            sort: self.agent_panel_sort,
            follows_workspace: self.agent_panel_follows_workspace,
        }
    }
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    order: AgentListOrder,
) -> Vec<String> {
    let mut pane_ids = unscoped_agent_pane_ids(snapshot, order.sort);
    if order.follows_workspace {
        let focused = snapshot.focused_workspace_id.as_deref();
        let scoped = snapshot
            .agents
            .iter()
            .filter(|agent| Some(agent.workspace_id.as_str()) == focused)
            .map(|agent| agent.pane_id.as_str())
            .collect::<HashSet<_>>();
        pane_ids.retain(|pane_id| scoped.contains(pane_id.as_str()));
    }
    pane_ids
}

pub(super) fn unscoped_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    match sort {
        crate::config::AgentPanelSortConfig::Priority => {
            agents.sort_by_key(|agent| {
                (
                    std::cmp::Reverse(status_priority(agent.agent_status)),
                    std::cmp::Reverse(agent.state_change_seq),
                )
            });
        }
        crate::config::AgentPanelSortConfig::Spaces => {
            sort_agents_by_tab_tree(&mut agents, &snapshot.tabs);
        }
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn sort_agents_by_tab_tree(
    agents: &mut [&crate::protocol::ClientShellAgent],
    tabs: &[ClientShellTab],
) {
    let rank = tab_tree_rank(tabs);
    agents.sort_by_key(|agent| {
        rank.get(agent.tab_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

fn same_workspace_parent<'a>(
    tab: &ClientShellTab,
    by_id: &HashMap<&str, &'a ClientShellTab>,
) -> Option<&'a ClientShellTab> {
    tab.parent_tab_id
        .as_deref()
        .and_then(|parent_id| by_id.get(parent_id).copied())
        .filter(|parent| parent.workspace_id == tab.workspace_id)
}

fn tab_tree_rank(tabs: &[ClientShellTab]) -> HashMap<&str, usize> {
    let by_id: HashMap<&str, &ClientShellTab> =
        tabs.iter().map(|tab| (tab.tab_id.as_str(), tab)).collect();
    let mut roots = Vec::new();
    let mut children: HashMap<&str, Vec<&ClientShellTab>> = HashMap::new();
    for tab in tabs {
        match same_workspace_parent(tab, &by_id) {
            Some(parent) => children
                .entry(parent.tab_id.as_str())
                .or_default()
                .push(tab),
            None => roots.push(tab),
        }
    }
    let mut rank = HashMap::with_capacity(tabs.len());
    for root in roots {
        let mut stack = vec![root];
        while let Some(tab) = stack.pop() {
            if rank.contains_key(tab.tab_id.as_str()) {
                continue;
            }
            rank.insert(tab.tab_id.as_str(), rank.len());
            if let Some(tab_children) = children.get(tab.tab_id.as_str()) {
                stack.extend(tab_children.iter().rev());
            }
        }
    }
    for tab in tabs {
        let next = rank.len();
        rank.entry(tab.tab_id.as_str()).or_insert(next);
    }
    rank
}

fn agent_tab_depths(snapshot: &ClientShellSnapshot) -> HashMap<&str, usize> {
    let by_id: HashMap<&str, &ClientShellTab> = snapshot
        .tabs
        .iter()
        .map(|tab| (tab.tab_id.as_str(), tab))
        .collect();
    let with_agents: HashSet<&str> = snapshot
        .agents
        .iter()
        .map(|agent| agent.tab_id.as_str())
        .collect();
    snapshot
        .tabs
        .iter()
        .map(|tab| {
            let mut depth = 0;
            let mut current = tab;
            for _ in 0..snapshot.tabs.len() {
                let Some(parent) = same_workspace_parent(current, &by_id) else {
                    break;
                };
                if with_agents.contains(parent.tab_id.as_str()) {
                    depth += 1;
                }
                current = parent;
            }
            (tab.tab_id.as_str(), depth)
        })
        .collect()
}

pub(super) fn tree_prefix(continues: &[bool], first_line: bool) -> String {
    let Some((own, ancestors)) = continues.split_last() else {
        return "  ".into();
    };
    let mut prefix = String::from("  ");
    for continuing in ancestors {
        prefix.push_str(if *continuing { "│  " } else { "   " });
    }
    prefix.push_str(match (first_line, own) {
        (true, true) => "├─ ",
        (true, false) => "└─ ",
        (false, true) => "│  ",
        (false, false) => "   ",
    });
    prefix
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    pulse_phase: u8,
    hits: &mut ShellHitMap,
) {
    let title = agent_panel_title(Some(snapshot), config);
    if !render_agent_panel_header(
        buffer,
        area,
        &title,
        snapshot.agent_view_label.as_deref(),
        config,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    render_agent_list(
        buffer,
        area,
        &rows,
        agent_panel_empty_message(snapshot.agent_view_label.as_deref(), config),
        config,
        agent_scroll,
        hits,
        |row| row.lines(),
        |buffer, rect, row, hits| {
            hits.agents.push((rect, row.pane_id.clone()));
            render_agent_row(buffer, rect, row, config, pulse_phase);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let title_width = area.width.saturating_sub(sort_width.saturating_add(1));
    put_text(
        buffer,
        area.x,
        area.y + 1,
        title_width,
        &crate::ui::truncate_end(title, title_width as usize),
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn agent_panel_title(
    snapshot: Option<&ClientShellSnapshot>,
    config: &ClientShellConfig,
) -> String {
    let followed = config
        .agent_panel_follows_workspace
        .then_some(snapshot)
        .flatten()
        .and_then(|snapshot| {
            let workspace_id = snapshot.focused_workspace_id.as_deref()?;
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == workspace_id)
        });
    match followed {
        Some(workspace) => format!(" {}", workspace.label),
        None => " agents".to_owned(),
    }
}

pub(super) fn agent_panel_empty_message(
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
) -> Option<&'static str> {
    if agent_view_label.is_some() {
        Some(" no matching agents")
    } else {
        config
            .agent_panel_follows_workspace
            .then_some(" no agents here")
    }
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[T],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
    row_lines: impl Fn(&T) -> usize,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, &mut ShellHitMap),
) {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows
        .iter()
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    let metrics =
        super::scroll::list_scroll_metrics(&row_heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, row) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        render_row(buffer, rect, row, hits);
        y = y
            .saturating_add(height)
            .saturating_add(if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            });
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    // Headers only make sense while entries run in workspace order; a view
    // sort would scatter them.
    let grouped = config.agent_panel_sort == crate::config::AgentPanelSortConfig::Spaces
        && snapshot.agent_view_label.is_none();
    let headers = grouped && !config.agent_panel_follows_workspace;
    let depths = if grouped {
        agent_tab_depths(snapshot)
    } else {
        HashMap::new()
    };
    let mut previous_workspace: Option<&str> = None;
    let mut rows: Vec<AgentRow> = ordered_agent_pane_ids(snapshot, config.agent_list_order())
        .into_iter()
        .enumerate()
        .filter_map(|(position, pane_id)| {
            let mut row = agent_row(snapshot, &pane_id, config, machine, Some(position + 1))?;
            let agent = snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)?;
            let workspace = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
            row.group = (headers && previous_workspace != Some(workspace.workspace_id.as_str()))
                .then(|| workspace.label.clone());
            row.group_gap = row.group.is_some() && previous_workspace.is_some();
            row.depth = depths.get(agent.tab_id.as_str()).copied().unwrap_or(0);
            previous_workspace = Some(workspace.workspace_id.as_str());
            Some(row)
        })
        .collect();
    for index in 0..rows.len() {
        rows[index].continues = (1..=rows[index].depth)
            .map(|level| {
                rows[index + 1..]
                    .iter()
                    .take_while(|row| row.depth >= level && row.group.is_none())
                    .any(|row| row.depth == level)
            })
            .collect();
    }
    rows
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
    index: Option<usize>,
) -> Option<AgentRow> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id);
    let tab_count = snapshot
        .tabs
        .iter()
        .filter(|candidate| candidate.workspace_id == agent.workspace_id)
        .count();
    let tab_label = tab
        .filter(|tab| tab_count > 1 || tab.custom_label)
        .map(|tab| super::render::tab_display_label(tab, &snapshot.tabs));
    let agent_label = agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref());
    let labels = agent
        .state_labels
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
    let state_text = labels
        .get(status_text(agent.agent_status))
        .map(String::as_str)
        .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
    let canonical_agent = agent
        .agent
        .as_deref()
        .and_then(crate::detect::parse_agent_label);
    let rows = crate::ui::sidebar_agent_rows(
        &config.agents,
        crate::ui::AgentTokenContext {
            machine,
            workspace: &workspace.label,
            tab: tab_label.as_deref(),
            pane: agent
                .title
                .as_deref()
                .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
            agent_label,
            terminal_title: agent.terminal_title.as_deref(),
            terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
            canonical_agent,
            tokens: &tokens,
            index,
        },
        state_text,
    );
    Some(AgentRow {
        pane_id: agent.pane_id.clone(),
        status: agent.agent_status,
        focused: agent.focused,
        group: None,
        group_gap: false,
        depth: 0,
        continues: Vec::new(),
        rows,
    })
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
    pulse_phase: u8,
) {
    let palette = &config.palette;
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(status_color(row.status, palette));
    let secondary = Style::default().fg(palette.overlay0);
    // Context tokens on the entry's first row are the label when a layout has
    // no workspace token there, so they read at full strength; later rows stay
    // dim as supporting detail. Regular weight in `text` keeps the label in a
    // different register from the bold `subtext0` group header.
    let leading = Style::default().fg(palette.text);
    let icon = (
        status_icon(row.status, config.status_indicators),
        status_dot_style(row.status, palette, pulse_phase),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows.clone()
    };
    let mut rect = rect;
    if row.group_gap {
        rect.y = rect.y.saturating_add(1);
        rect.height = rect.height.saturating_sub(1);
    }
    if let Some(group) = &row.group {
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(" {group}"),
            Style::default()
                .fg(palette.subtext0)
                .add_modifier(Modifier::BOLD),
        );
        rect.y = rect.y.saturating_add(1);
        rect.height = rect.height.saturating_sub(1);
    }
    // Column 0 belongs to the focus marker. Every row of one entry keeps the
    // same indent so labels stay in one column instead of stepping right on
    // continuation rows; nested entries shift by their tree prefix.
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let prefix = tree_prefix(&row.continues, index == 0);
        let indent = display_width(&prefix);
        let context = if index == 0 { leading } else { secondary };
        let mut spans = vec![ratatui::text::Span::styled(
            prefix,
            Style::default().fg(palette.overlay0),
        )];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            context,
            context,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
        ));
        Paragraph::new(Line::from(spans)).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }

    let ribbon_height = (rows.len().min(u16::MAX as usize) as u16).min(rect.height);
    if ribbon_height > 0 {
        super::sidebar::render_status_ribbon(
            buffer,
            rect.x,
            rect.y,
            ribbon_height,
            super::sidebar::workspace_ribbon(palette, false, row.focused),
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}
