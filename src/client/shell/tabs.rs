use super::*;
use ratatui::style::Color;

const TAB_SCROLL_BUTTON_WIDTH: u16 = 3;
const MIN_TAB_STRIP_WIDTH: u16 =
    MIN_TAB_WIDTH + NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2);
const MAX_TAB_BAR_WORKSPACE_WIDTH: usize = 24;

fn render_tab_bar_workspace(
    buffer: &mut Buffer,
    content: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    flash_phase: Option<u8>,
    hits: &mut ShellHitMap,
) -> Rect {
    if !config.tab_bar_workspace {
        return content;
    }
    let Some(workspace) = snapshot.focused_workspace_id.as_deref().and_then(|id| {
        snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == id)
    }) else {
        return content;
    };
    let label = crate::ui::truncate_end(&workspace.label, MAX_TAB_BAR_WORKSPACE_WIDTH);
    let label_width = display_width(&label);
    let segment_width = label_width.saturating_add(4);
    if content.width.saturating_sub(segment_width) < MIN_TAB_STRIP_WIDTH {
        return content;
    }
    let palette = &config.palette;
    let label_style = match flash_phase {
        Some(0) => Style::default()
            .fg(panel_contrast_fg(palette))
            .bg(palette.accent),
        Some(_) => Style::default().fg(palette.accent).bg(palette.surface0),
        None => Style::default().fg(palette.accent).bg(palette.panel_bg),
    }
    .add_modifier(Modifier::BOLD);
    put_text(
        buffer,
        content.x,
        content.y,
        label_width.saturating_add(2),
        &format!(" {label} "),
        label_style,
    );
    put_text(
        buffer,
        content.x.saturating_add(label_width).saturating_add(2),
        content.y,
        2,
        "│ ",
        Style::default()
            .fg(palette.surface_dim)
            .bg(palette.panel_bg),
    );
    hits.tab_bar_workspace = Rect::new(content.x, content.y, label_width.saturating_add(2), 1);
    Rect {
        x: content.x.saturating_add(segment_width),
        width: content.width.saturating_sub(segment_width),
        ..content
    }
}

pub(crate) fn render_tab_bar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    pulse_phase: u8,
    workspace_flash_phase: Option<u8>,
    tab_scroll: &mut usize,
    reveal_focused_tab: &mut bool,
    tab_drag_insert_index: Option<usize>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect::<Vec<_>>();
    let bar_style = config.tab_bar_style;
    let unselected_bg = unselected_tab_bg(palette, bar_style);
    let pane_counts = if bar_style == TabBarStyleConfig::Quiet {
        tiled_pane_counts(snapshot)
    } else {
        HashMap::new()
    };
    let pane_count_of = |tab: &ClientShellTab| {
        pane_counts
            .get(tab.tab_id.as_str())
            .copied()
            .unwrap_or_default()
    };
    let desired_widths = tabs
        .iter()
        .map(|tab| {
            let label = tab_label(tab, &snapshot.tabs, config, pane_count_of(tab));
            display_width(&label)
                .saturating_add(tab_padding(bar_style).saturating_mul(2))
                .max(MIN_TAB_WIDTH)
        })
        .collect::<Vec<_>>();
    let content = render_tab_bar_workspace(
        buffer,
        tab_bar_content_area(snapshot, area),
        snapshot,
        config,
        workspace_flash_phase,
        hits,
    );
    let mouse_chrome = config.mouse_capture;
    let new_tab_width = if mouse_chrome { NEW_TAB_WIDTH } else { 0 };
    let desired_total = desired_widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add)
        .saturating_add(tabs.len().saturating_sub(1).min(u16::MAX as usize) as u16)
        .saturating_add(new_tab_width);
    let overflow =
        desired_total > content.width && (!mouse_chrome || content.width >= MIN_TAB_STRIP_WIDTH);
    let available = if overflow && mouse_chrome {
        content
            .width
            .saturating_sub(NEW_TAB_WIDTH)
            .saturating_sub(TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2))
    } else {
        content.width.saturating_sub(new_tab_width)
    };
    let max_scroll = max_tab_scroll(&desired_widths, available);
    if !overflow {
        *tab_scroll = 0;
    } else if *reveal_focused_tab {
        if let Some(focused) = tabs.iter().position(|tab| tab.focused) {
            *tab_scroll = centered_tab_scroll(focused, &desired_widths, available).min(max_scroll);
        }
    } else {
        *tab_scroll = (*tab_scroll).min(max_scroll);
    }
    *reveal_focused_tab = false;

    let mut x = content.x;
    let tab_right = if overflow && mouse_chrome {
        hits.tab_scroll_left = Rect::new(
            content.x,
            content.y,
            TAB_SCROLL_BUTTON_WIDTH.min(content.width),
            1,
        );
        put_text(
            buffer,
            hits.tab_scroll_left.x,
            content.y,
            hits.tab_scroll_left.width,
            " < ",
            Style::default()
                .fg(if *tab_scroll > 0 {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        x = hits.tab_scroll_left.right();
        content
            .right()
            .saturating_sub(NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH)
    } else {
        content.right().saturating_sub(new_tab_width)
    };

    let mut first_visible = None;
    let mut last_visible = None;
    for (index, tab) in tabs.iter().enumerate().skip(*tab_scroll) {
        let name = tab_label(tab, &snapshot.tabs, config, pane_count_of(tab));
        let desired = desired_widths[index];
        let remaining = tab_right.saturating_sub(x);
        let width = desired.min(remaining);
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let style = if tab.focused {
            Style::default()
                .fg(palette.accent)
                .bg(unselected_bg)
                .add_modifier(Modifier::BOLD)
        } else if tab.custom_label {
            Style::default().fg(palette.overlay1).bg(unselected_bg)
        } else {
            Style::default().fg(palette.overlay0).bg(unselected_bg)
        };
        let padding = width.saturating_sub(display_width(&name));
        let left = padding / 2;
        let text = format!(
            "{empty:left$}{name}{empty:right_padding$}",
            empty = "",
            left = left as usize,
            right_padding = padding.saturating_sub(left) as usize,
        );
        put_text(buffer, rect.x, rect.y, rect.width, &text, style);
        if let Some(icon) = tab_status_icon(tab, config) {
            let icon_x = rect.x.saturating_add(left);
            let icon_width = display_width(icon).min(rect.right().saturating_sub(icon_x));
            buffer.set_style(
                Rect::new(icon_x, rect.y, icon_width, 1),
                status_dot_style(tab.agent_status, palette, pulse_phase),
            );
        }
        hits.tabs.push((rect, tab.tab_id.clone()));
        first_visible.get_or_insert(index);
        last_visible = Some(index);
        x = x.saturating_add(width + 1);
        if width < desired {
            break;
        }
    }

    if overflow && mouse_chrome {
        hits.tab_scroll_right = Rect::new(tab_right, area.y, TAB_SCROLL_BUTTON_WIDTH, 1);
        let can_scroll_right = *tab_scroll < max_scroll;
        put_text(
            buffer,
            hits.tab_scroll_right.x,
            area.y,
            hits.tab_scroll_right.width,
            " > ",
            Style::default()
                .fg(if can_scroll_right {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        hits.new_tab = Rect::new(
            hits.tab_scroll_right.right(),
            area.y,
            content
                .right()
                .saturating_sub(hits.tab_scroll_right.right())
                .min(NEW_TAB_WIDTH),
            1,
        );
    } else if mouse_chrome {
        hits.new_tab = Rect::new(
            x.min(content.right()),
            area.y,
            content.right().saturating_sub(x).min(NEW_TAB_WIDTH),
            1,
        );
    }
    if mouse_chrome {
        put_text(
            buffer,
            hits.new_tab.x,
            area.y,
            hits.new_tab.width,
            " + ",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }

    if first_visible.is_some_and(|index| index > 0) {
        let ellipsis_x = if hits.tab_scroll_left.width > 0 {
            hits.tab_scroll_left.right()
        } else {
            content.x
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }
    if last_visible.is_some_and(|index| index + 1 < tabs.len()) {
        let ellipsis_x = if hits.tab_scroll_right.width > 0 {
            hits.tab_scroll_right.x.saturating_sub(1)
        } else {
            content.right().saturating_sub(1)
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x >= content.x && ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }

    if let Some(insert_index) = tab_drag_insert_index {
        if let Some(indicator_x) = tab_drop_indicator_x(hits, &tabs, insert_index) {
            put_text(
                buffer,
                indicator_x.min(content.right().saturating_sub(1)),
                area.y,
                1,
                "│",
                Style::default().fg(palette.accent),
            );
        }
    }
    render_tab_bar_status(buffer, area, snapshot, palette);
}

pub(crate) fn tab_bar_status_width(snapshot: &ClientShellSnapshot) -> u16 {
    let content = snapshot.tab_bar_right.iter().fold(0u16, |width, segment| {
        width.saturating_add(display_width(&segment.text))
    });
    let separators = snapshot.tab_bar_right.len().saturating_sub(1);
    content.saturating_add(
        display_width(&snapshot.tab_bar_right_separator)
            .saturating_mul(separators.min(u16::MAX as usize) as u16),
    )
}

fn tab_bar_status_area(snapshot: &ClientShellSnapshot, area: Rect) -> Option<Rect> {
    let width = tab_bar_status_width(snapshot);
    if width == 0 {
        return None;
    }
    let reserved = width.saturating_add(1);
    (area.width.saturating_sub(reserved) >= MIN_TAB_STRIP_WIDTH)
        .then(|| Rect::new(area.right().saturating_sub(width), area.y, width, 1))
}

fn tab_bar_content_area(snapshot: &ClientShellSnapshot, area: Rect) -> Rect {
    let reserved = tab_bar_status_area(snapshot, area)
        .map(|status| status.width.saturating_add(1))
        .unwrap_or(0);
    Rect {
        width: area.width.saturating_sub(reserved),
        ..area
    }
}

fn render_tab_bar_status(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) {
    let Some(status) = tab_bar_status_area(snapshot, area) else {
        return;
    };
    let separator_width = display_width(&snapshot.tab_bar_right_separator);
    let mut x = status.x;
    for (index, segment) in snapshot.tab_bar_right.iter().enumerate() {
        if index > 0 && separator_width > 0 {
            put_text(
                buffer,
                x,
                area.y,
                separator_width,
                &snapshot.tab_bar_right_separator,
                Style::default().fg(palette.overlay0).bg(palette.panel_bg),
            );
            x = x.saturating_add(separator_width);
        }
        let width = display_width(&segment.text);
        let style = if segment.accent {
            Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay1).bg(palette.panel_bg)
        };
        put_text(buffer, x, area.y, width, &segment.text, style);
        x = x.saturating_add(width);
    }
}

fn tab_drop_indicator_x(
    hits: &ShellHitMap,
    tabs: &[&ClientShellTab],
    insert_index: usize,
) -> Option<u16> {
    let visible = hits
        .tabs
        .iter()
        .filter_map(|(rect, tab_id)| {
            tabs.iter()
                .position(|tab| tab.tab_id == *tab_id)
                .map(|index| (index, *rect))
        })
        .collect::<Vec<_>>();
    let (first_index, first_rect) = *visible.first()?;
    let (last_index, last_rect) = *visible.last()?;
    if insert_index == 0 {
        return Some(if first_index == 0 {
            first_rect.x
        } else {
            hits.tab_scroll_left.right()
        });
    }
    if let Some((_, rect)) = visible.iter().find(|(index, _)| *index == insert_index) {
        return Some(rect.x.saturating_sub(1));
    }
    if insert_index >= tabs.len() {
        return Some(if last_index + 1 >= tabs.len() {
            last_rect.right()
        } else {
            hits.tab_scroll_right.x.saturating_sub(1)
        });
    }
    None
}

fn tab_padding(style: TabBarStyleConfig) -> u16 {
    match style {
        TabBarStyleConfig::Classic => 2,
        TabBarStyleConfig::Quiet => 3,
    }
}

fn unselected_tab_bg(palette: &Palette, style: TabBarStyleConfig) -> Color {
    if style == TabBarStyleConfig::Quiet && palette.surface0 == Color::Reset {
        palette.active_row_bg
    } else {
        palette.surface0
    }
}

fn tiled_pane_counts(snapshot: &ClientShellSnapshot) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for pane in &snapshot.panes {
        *counts.entry(pane.tab_id.as_str()).or_insert(0) += 1;
    }
    counts
}

fn centered_tab_scroll(focused: usize, widths: &[u16], available: u16) -> usize {
    let mut best = focused;
    let mut best_distance = u16::MAX;
    for start in 0..=focused {
        let before = widths
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .take(focused.saturating_sub(start))
            .fold(0u16, |width, (_, tab)| width.saturating_add(tab + 1));
        if before >= available {
            continue;
        }
        let focused_width = widths[focused].min(available.saturating_sub(before));
        let center = before.saturating_mul(2).saturating_add(focused_width);
        let distance = center.abs_diff(available);
        if distance <= best_distance {
            best_distance = distance;
            best = start;
        }
    }
    best
}

fn max_tab_scroll(widths: &[u16], available: u16) -> usize {
    let Some((&last, preceding)) = widths.split_last() else {
        return 0;
    };
    let mut start = preceding.len();
    let mut used = u32::from(last);
    // Keep the longest fully visible suffix, not merely a sliver of the last tab.
    // An oversized last tab must still be reachable at the start of the strip.
    for width in preceding.iter().rev() {
        let required = used + 1 + u32::from(*width);
        if required > u32::from(available) {
            break;
        }
        used = required;
        start -= 1;
    }
    start
}

pub(in crate::client::shell) fn tab_display_label(
    tab: &ClientShellTab,
    tabs: &[ClientShellTab],
) -> String {
    match tab
        .parent_tab_id
        .as_deref()
        .and_then(|parent_id| tabs.iter().find(|candidate| candidate.tab_id == parent_id))
    {
        Some(parent) => format!("{} › {}", parent.label, tab.label),
        None => tab.label.clone(),
    }
}

fn tab_label(
    tab: &ClientShellTab,
    tabs: &[ClientShellTab],
    config: &ClientShellConfig,
    pane_count: usize,
) -> String {
    let label = tab_display_label(tab, tabs);
    let label = if config.tab_bar_style == TabBarStyleConfig::Quiet && pane_count >= 2 {
        format!("{label} ·{pane_count}")
    } else {
        label
    };
    let label = if tab.zoomed {
        format!("{label} Z")
    } else {
        label
    };
    match tab_status_icon(tab, config) {
        Some(icon) => format!("{icon} {label}"),
        None => label,
    }
}

fn tab_status_icon(tab: &ClientShellTab, config: &ClientShellConfig) -> Option<&'static str> {
    (config.tab_bar_status && tab.agent_status != crate::api::schema::AgentStatus::Unknown)
        .then(|| status_icon(tab.agent_status, config.status_indicators))
}

#[cfg(test)]
mod tests {
    use super::max_tab_scroll;

    #[test]
    fn trailing_scroll_limit_accounts_for_full_widths_and_separators() {
        for (widths, available, expected) in [
            (&[][..], 0, 0),
            (&[8, 13][..], 0, 1),
            (&[8, 13][..], 1, 1),
            (&[8, 13][..], 12, 1),
            (&[8, 13][..], 21, 1),
            (&[8, 13][..], 22, 0),
            (&[8, 13][..], 30, 0),
            (&[8, u16::MAX][..], u16::MAX, 1),
        ] {
            assert_eq!(
                max_tab_scroll(widths, available),
                expected,
                "widths={widths:?}, available={available}"
            );
        }
    }
}
