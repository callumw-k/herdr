use super::*;

mod settings_overlay;
mod worktree_overlays;

#[derive(Default)]
pub(crate) struct OverlayRender {
    pub(crate) area: Rect,
    pub(crate) menu_rows: Vec<(Rect, usize)>,
    pub(crate) primary: Rect,
    pub(crate) clear: Rect,
    pub(crate) cancel: Rect,
    pub(crate) rename_fields: Vec<(Rect, ClientRenameField)>,
    pub(crate) navigator_popup: Rect,
    pub(crate) navigator_search: Rect,
    pub(crate) navigator_rows: Vec<(Rect, ClientNavigatorTarget)>,
    pub(crate) navigator_preview: Rect,
    pub(crate) navigator_scrollbar: Rect,
    pub(crate) navigator_scroll_metrics: Option<crate::pane::ScrollMetrics>,
    pub(crate) worktree_search: Rect,
    pub(crate) worktree_rows: Vec<(Rect, usize)>,
    pub(crate) help_popup: Rect,
    pub(crate) help_scrollbar: Rect,
    pub(crate) help_scroll_metrics: Option<crate::pane::ScrollMetrics>,
    pub(crate) help_max_scroll: usize,
    pub(crate) settings_popup: Rect,
    pub(crate) settings_tabs: Vec<(Rect, ClientSettingsSection)>,
    pub(crate) settings_choices: Vec<(Rect, usize)>,
    pub(crate) product_announcement_scrollbar: Rect,
    pub(crate) product_announcement_scroll_metrics: Option<crate::pane::ScrollMetrics>,
    pub(crate) product_announcement_max_scroll: usize,
    pub(crate) release_notes_scrollbar: Rect,
    pub(crate) release_notes_scroll_metrics: Option<crate::pane::ScrollMetrics>,
    pub(crate) release_notes_max_scroll: usize,
    pub(crate) cursor: Option<crate::protocol::CursorState>,
}

pub(crate) fn render_client_overlay(
    b: &mut Buffer,
    o: &ClientShellOverlay,
    s: &ClientShellSnapshot,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    k: &LiveKeybindConfig,
    p: &Palette,
    pulse_phase: u8,
) -> Option<OverlayRender> {
    if !matches!(
        o,
        ClientShellOverlay::Navigator(_)
            | ClientShellOverlay::ContextMenu(_)
            | ClientShellOverlay::GlobalMenu(_)
    ) {
        for y in b.area.y..b.area.bottom() {
            for x in b.area.x..b.area.right() {
                let c = &mut b[(x, y)];
                c.set_style(c.style().add_modifier(Modifier::DIM));
            }
        }
    }
    match o {
        ClientShellOverlay::Onboarding => render_onboarding_overlay(b, k, p),
        ClientShellOverlay::ProductAnnouncement(v) => render_product_announcement_overlay(b, v, p),
        ClientShellOverlay::ReleaseNotes(v) => {
            render_release_notes_overlay(b, v, &s.update_install_command, p)
        }
        ClientShellOverlay::Rename(v) => render_rename_overlay(b, v, p),
        ClientShellOverlay::ConfirmClose(v) => render_confirm_close_overlay(b, v, p),
        ClientShellOverlay::Help(v) => render_help_overlay(b, v, k, p),
        ClientShellOverlay::Navigator(v) => {
            render_navigator_overlay(b, v, endpoints, active_endpoint_id, p, pulse_phase)
        }
        ClientShellOverlay::Settings(v) => {
            settings_overlay::render_settings_overlay(b, v, s.integration_updates_available, p)
        }
        ClientShellOverlay::WorktreeCreate(v) => {
            worktree_overlays::render_worktree_create_overlay(b, v, p)
        }
        ClientShellOverlay::WorktreeOpen(v) => {
            worktree_overlays::render_worktree_open_overlay(b, v, p)
        }
        ClientShellOverlay::WorktreeRemove(v) => {
            worktree_overlays::render_worktree_remove_overlay(b, v, p)
        }
        ClientShellOverlay::ContextMenu(_) | ClientShellOverlay::GlobalMenu(_) => None,
    }
}

pub(crate) fn render_global_menu(
    buffer: &mut Buffer,
    launcher: Rect,
    menu: &ClientGlobalMenuOverlay,
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) -> Option<OverlayRender> {
    let items = super::super::global_menu::global_menu_items(snapshot);
    let screen = buffer.area;
    let width = items
        .iter()
        .map(|(label, action)| {
            display_width(label)
                + u16::from(super::super::global_menu::global_menu_item_has_badge(
                    snapshot, *action,
                )) * 2
        })
        .max()
        .unwrap_or(8)
        .saturating_add(4)
        .min(screen.width.max(1));
    let height = (items.len() as u16)
        .saturating_add(2)
        .min(screen.height.max(1));
    let x = launcher
        .right()
        .saturating_sub(width)
        .min(screen.right().saturating_sub(width));
    let y = launcher.y.saturating_sub(height).max(screen.y);
    let rect = Rect::new(x, y, width, height);
    let inner = panel(buffer, rect, palette.accent, palette.panel_bg)?;
    let mut rows = Vec::new();
    for (index, (label, action)) in items.iter().enumerate() {
        let row_y = inner.y.saturating_add(index as u16);
        if row_y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, row_y, inner.width, 1);
        let highlighted = index == menu.highlighted;
        let style = if highlighted {
            Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.text).bg(palette.panel_bg)
        };
        buffer.set_style(row, style);
        let has_badge = super::super::global_menu::global_menu_item_has_badge(snapshot, *action);
        if has_badge {
            let badge_style = if highlighted {
                style
            } else {
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD)
            };
            put_text(buffer, row.x, row.y, row.width.min(2), " ●", badge_style);
            put_text(
                buffer,
                row.x.saturating_add(2),
                row.y,
                row.width.saturating_sub(2),
                &format!(" {label}"),
                style,
            );
        } else {
            put_text(buffer, row.x, row.y, row.width, &format!(" {label}"), style);
        }
        rows.push((row, index));
    }
    Some(OverlayRender {
        area: rect,
        menu_rows: rows,
        ..OverlayRender::default()
    })
}

pub(crate) fn render_context_menu(
    buffer: &mut Buffer,
    menu: &ClientContextMenuOverlay,
    palette: &Palette,
) -> Option<OverlayRender> {
    let items = menu.items();
    let screen = buffer.area;
    let max_item_width = items
        .iter()
        .map(|item| display_width(item.label))
        .max()
        .unwrap_or(0);
    let width = max_item_width
        .saturating_add(4)
        .max(14)
        .min(screen.width.max(1));
    let height = (items.len() as u16)
        .saturating_add(2)
        .min(screen.height.max(1));
    let x = menu
        .x
        .min(screen.x.saturating_add(screen.width.saturating_sub(width)));
    let y = menu.y.min(
        screen
            .y
            .saturating_add(screen.height.saturating_sub(height)),
    );
    let rect = Rect::new(x, y, width, height);
    let inner = panel(buffer, rect, palette.accent, palette.panel_bg)?;
    let mut rows = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let row_y = inner.y.saturating_add(index as u16);
        if row_y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, row_y, inner.width, 1);
        let highlighted = index == menu.highlighted;
        let style = if highlighted {
            Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.text).bg(palette.panel_bg)
        };
        buffer.set_style(row, style);
        put_text(buffer, row.x, row.y, row.width, item.label, style);
        rows.push((row, index));
    }
    Some(OverlayRender {
        area: rect,
        menu_rows: rows,
        ..OverlayRender::default()
    })
}

fn panel(
    b: &mut Buffer,
    a: Rect,
    c: ratatui::style::Color,
    bg: ratatui::style::Color,
) -> Option<Rect> {
    if a.width < 2 || a.height < 2 {
        return None;
    }
    let background = Style::default().bg(bg).remove_modifier(Modifier::DIM);
    let border = Style::default().fg(c).bg(bg).remove_modifier(Modifier::DIM);
    for y in a.y..a.bottom() {
        for x in a.x..a.right() {
            b[(x, y)].set_symbol(" ").set_style(background);
        }
    }
    for x in a.x..a.right() {
        b[(x, a.y)]
            .set_symbol(if x == a.x {
                "┌"
            } else if x + 1 == a.right() {
                "┐"
            } else {
                "─"
            })
            .set_style(border);
        let y = a.bottom() - 1;
        b[(x, y)]
            .set_symbol(if x == a.x {
                "└"
            } else if x + 1 == a.right() {
                "┘"
            } else {
                "─"
            })
            .set_style(border);
    }
    for y in a.y + 1..a.bottom() - 1 {
        b[(a.x, y)].set_symbol("│").set_style(border);
        b[(a.right() - 1, y)].set_symbol("│").set_style(border);
    }
    Some(Rect::new(a.x + 1, a.y + 1, a.width - 2, a.height - 2))
}
fn popup(a: Rect, w: u16, h: u16) -> Option<Rect> {
    popup_with_width_cap(a, w, h, 4)
}

fn popup_with_width_cap(a: Rect, w: u16, h: u16, width_cap: u16) -> Option<Rect> {
    let w = w.min(a.width.saturating_sub(width_cap));
    let h = h.min(a.height.saturating_sub(2));
    if w < 4 || h < 4 {
        return None;
    }
    Some(Rect::new(
        a.x + (a.width - w) / 2,
        a.y + (a.height - h) / 2,
        w,
        h,
    ))
}
fn button(b: &mut Buffer, r: Rect, t: &str, s: Style) {
    b.set_style(r, s);
    let w = display_width(t).min(r.width);
    put_text(b, r.x + (r.width - w) / 2, r.y, w, t, s)
}
fn row(i: Rect, ws: &[u16], gap: u16, off: u16) -> Vec<Rect> {
    let total = ws.iter().sum::<u16>() + gap * (ws.len().saturating_sub(1) as u16);
    let mut x = i.x + i.width.saturating_sub(total) / 2;
    ws.iter()
        .map(|w| {
            let r = Rect::new(
                x,
                i.y + off.min(i.height.saturating_sub(1)),
                (*w).min(i.width.saturating_sub(x - i.x)),
                1,
            );
            x += *w + gap;
            r
        })
        .collect()
}
fn contrast(p: &Palette) -> ratatui::style::Color {
    match p.panel_bg {
        ratatui::style::Color::Reset => p.surface_dim,
        c => c,
    }
}
fn render_release_notes_overlay(
    b: &mut Buffer,
    notes: &crate::app::state::ReleaseNotesState,
    install_command: &str,
    p: &Palette,
) -> Option<OverlayRender> {
    let outer = popup(
        b.area,
        crate::ui::RELEASE_NOTES_MODAL_SIZE.0,
        crate::ui::RELEASE_NOTES_MODAL_SIZE.1,
    )?;
    let inner = panel(b, outer, p.accent, p.panel_bg)?;
    if inner.height < 8 || inner.width < 20 {
        return Some(OverlayRender {
            area: outer,
            ..OverlayRender::default()
        });
    }

    let stack = crate::ui::modal_stack_areas(inner, 2, 1, 0, 1);
    let title_area = Rect::new(
        stack.header.x.saturating_add(1),
        stack.header.y,
        stack.header.width.saturating_sub(2),
        1,
    );
    let subtitle_area = Rect::new(
        stack.header.x.saturating_add(1),
        stack.header.y.saturating_add(1),
        stack.header.width.saturating_sub(2),
        1,
    );
    let base = Style::default()
        .bg(p.panel_bg)
        .remove_modifier(Modifier::DIM);
    put_text(
        b,
        title_area.x,
        title_area.y,
        title_area.width,
        &format!("v{}", notes.version),
        base.fg(p.text).add_modifier(Modifier::BOLD),
    );
    put_text(
        b,
        subtitle_area.x,
        subtitle_area.y,
        subtitle_area.width,
        if notes.preview {
            "update ready"
        } else {
            "what's new in this release"
        },
        base.fg(p.overlay1),
    );
    let close = crate::ui::release_notes_close_button_rect(Rect::new(
        stack.header.x,
        stack.header.y,
        stack.header.width,
        1,
    ));
    button(
        b,
        close,
        " esc close ",
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
            .remove_modifier(Modifier::DIM),
    );

    let body = stack.content;
    let lines = crate::ui::release_notes_display_lines(notes, install_command, p);
    let metrics = crate::ui::release_notes_scroll_metrics(notes, install_command, body, p);
    let max_scroll = metrics.max_offset_from_bottom;
    let scroll = usize::from(notes.scroll).min(max_scroll);
    let track = crate::ui::release_notes_scrollbar_rect(body, metrics);
    let text_area = track
        .map(|_| Rect::new(body.x, body.y, body.width.saturating_sub(1), body.height))
        .unwrap_or(body);
    let paragraph = ratatui::widgets::Paragraph::new(
        lines.into_iter().map(|(_, line)| line).collect::<Vec<_>>(),
    )
    .wrap(ratatui::widgets::Wrap { trim: false })
    .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
    ratatui::widgets::Widget::render(paragraph, text_area, b);
    if let Some(track) = track {
        crate::ui::render_scrollbar_buffer(b, metrics, track, p.overlay0, p.overlay1, "▐");
    }

    if let Some(footer_area) = stack.footer {
        let footer_line = ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(" scroll ", base.fg(p.overlay0)),
            ratatui::text::Span::styled("wheel ↑↓", base.fg(p.text)),
            ratatui::text::Span::styled("  ·  ", base.fg(p.overlay0)),
            ratatui::text::Span::styled("close", base.fg(p.overlay0)),
            ratatui::text::Span::styled(" esc / enter ", base.fg(p.text)),
        ]);
        ratatui::widgets::Widget::render(
            ratatui::widgets::Paragraph::new(footer_line),
            footer_area,
            b,
        );
    }

    Some(OverlayRender {
        area: outer,
        primary: close,
        release_notes_scrollbar: track.unwrap_or_default(),
        release_notes_scroll_metrics: Some(metrics),
        release_notes_max_scroll: max_scroll,
        ..OverlayRender::default()
    })
}

fn render_product_announcement_overlay(
    b: &mut Buffer,
    announcement: &crate::app::state::ProductAnnouncementState,
    p: &Palette,
) -> Option<OverlayRender> {
    let outer = popup(
        b.area,
        crate::ui::PRODUCT_ANNOUNCEMENT_MODAL_SIZE.0,
        crate::ui::PRODUCT_ANNOUNCEMENT_MODAL_SIZE.1,
    )?;
    let inner = panel(b, outer, p.accent, p.panel_bg)?;
    if inner.height < 8 || inner.width < 20 {
        return Some(OverlayRender {
            area: outer,
            ..OverlayRender::default()
        });
    }

    let stack = crate::ui::modal_stack_areas(inner, 2, 1, 0, 1);
    let title_area = Rect::new(
        stack.header.x.saturating_add(1),
        stack.header.y,
        stack.header.width.saturating_sub(2),
        1,
    );
    let subtitle_area = Rect::new(
        stack.header.x.saturating_add(1),
        stack.header.y.saturating_add(1),
        stack.header.width.saturating_sub(2),
        1,
    );
    let base = Style::default()
        .bg(p.panel_bg)
        .remove_modifier(Modifier::DIM);
    put_text(
        b,
        title_area.x,
        title_area.y,
        title_area.width,
        &announcement.title,
        base.fg(p.text).add_modifier(Modifier::BOLD),
    );
    let subtitle = if announcement.preview {
        "product announcement preview"
    } else {
        "product announcement"
    };
    put_text(
        b,
        subtitle_area.x,
        subtitle_area.y,
        subtitle_area.width,
        &format!("{subtitle} · v{}", announcement.version),
        base.fg(p.overlay1),
    );
    let close = crate::ui::release_notes_close_button_rect(Rect::new(
        stack.header.x,
        stack.header.y,
        stack.header.width,
        1,
    ));
    button(
        b,
        close,
        " esc close ",
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
            .remove_modifier(Modifier::DIM),
    );

    let body = stack.content;
    let lines = crate::ui::product_announcement_display_lines(announcement, p);
    let metrics = crate::ui::product_announcement_scroll_metrics(announcement, body, p);
    let max_scroll = metrics.max_offset_from_bottom;
    let scroll = usize::from(announcement.scroll).min(max_scroll);
    let track = crate::ui::release_notes_scrollbar_rect(body, metrics);
    let text_area = track
        .map(|_| Rect::new(body.x, body.y, body.width.saturating_sub(1), body.height))
        .unwrap_or(body);
    let paragraph = ratatui::widgets::Paragraph::new(
        lines.into_iter().map(|(_, line)| line).collect::<Vec<_>>(),
    )
    .wrap(ratatui::widgets::Wrap { trim: false })
    .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
    ratatui::widgets::Widget::render(paragraph, text_area, b);
    if let Some(track) = track {
        crate::ui::render_scrollbar_buffer(b, metrics, track, p.overlay0, p.overlay1, "▐");
    }

    if let Some(footer_area) = stack.footer {
        let footer_line = ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(" scroll ", base.fg(p.overlay0)),
            ratatui::text::Span::styled("wheel ↑↓", base.fg(p.text)),
            ratatui::text::Span::styled("  ·  ", base.fg(p.overlay0)),
            ratatui::text::Span::styled("close", base.fg(p.overlay0)),
            ratatui::text::Span::styled(" esc / enter ", base.fg(p.text)),
        ]);
        ratatui::widgets::Widget::render(
            ratatui::widgets::Paragraph::new(footer_line),
            footer_area,
            b,
        );
    }

    Some(OverlayRender {
        area: outer,
        primary: close,
        product_announcement_scrollbar: track.unwrap_or_default(),
        product_announcement_scroll_metrics: Some(metrics),
        product_announcement_max_scroll: max_scroll,
        ..OverlayRender::default()
    })
}

fn render_onboarding_overlay(
    b: &mut Buffer,
    k: &LiveKeybindConfig,
    p: &Palette,
) -> Option<OverlayRender> {
    let prefix = k.primary_prefix_label();
    let hint_width = display_width("  ")
        + display_width(&prefix)
        + display_width(crate::ui::ONBOARDING_PREFIX_SUFFIX)
        + display_width(crate::ui::ONBOARDING_HELP_LABEL)
        + display_width(crate::ui::ONBOARDING_HELP_SUFFIX);
    let outer = popup_with_width_cap(b.area, (hint_width + 5).max(64), 16, 3)?;
    let inner = panel(b, outer, p.accent, p.panel_bg)?;
    if inner.height < 11 {
        return Some(OverlayRender {
            area: outer,
            ..OverlayRender::default()
        });
    }
    let stack = crate::ui::modal_stack_areas(inner, 2, 0, 1, 1);
    let base = Style::default()
        .bg(p.panel_bg)
        .remove_modifier(Modifier::DIM);
    let title = base.fg(p.text).add_modifier(Modifier::BOLD);
    let muted = base.fg(p.overlay0);
    let text = base.fg(p.overlay1);
    let accent = base.fg(p.accent).add_modifier(Modifier::BOLD);

    put_text(
        b,
        stack.header.x,
        stack.header.y,
        stack.header.width,
        crate::ui::ONBOARDING_TITLE,
        title,
    );
    put_text(
        b,
        stack.header.x,
        stack.header.y.saturating_add(1),
        stack.header.width,
        crate::ui::ONBOARDING_SUBTITLE,
        muted,
    );

    let content = stack.content;
    for (offset, line) in crate::ui::ONBOARDING_DESCRIPTION.iter().enumerate() {
        put_text(
            b,
            content.x,
            content.y.saturating_add(offset as u16),
            content.width,
            line,
            text,
        );
    }

    let key_y = content.y.saturating_add(4);
    let mut key_x = content.x;
    for (value, style) in [
        ("  ", base),
        (prefix.as_str(), accent),
        (crate::ui::ONBOARDING_PREFIX_SUFFIX, text),
        (crate::ui::ONBOARDING_HELP_LABEL, accent),
        (crate::ui::ONBOARDING_HELP_SUFFIX, text),
    ] {
        let width = display_width(value);
        put_text(
            b,
            key_x,
            key_y,
            content.right().saturating_sub(key_x),
            value,
            style,
        );
        key_x = key_x.saturating_add(width);
    }
    put_text(
        b,
        content.x,
        content.y.saturating_add(5),
        content.width,
        crate::ui::ONBOARDING_NEXT,
        text,
    );

    let primary = crate::ui::onboarding_welcome_continue_rect(stack.actions.unwrap_or_default());
    button(
        b,
        primary,
        " ↵ continue ",
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
            .remove_modifier(Modifier::DIM),
    );
    Some(OverlayRender {
        area: outer,
        primary,
        ..OverlayRender::default()
    })
}

fn render_rename_overlay(
    b: &mut Buffer,
    v: &ClientRenameOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let two_fields = matches!(
        v.target,
        ClientRenameTarget::Workspace { .. } | ClientRenameTarget::NewWorkspace { .. }
    );
    let q = popup(b.area, 56, if two_fields { 9 } else { 7 })?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(
        b,
        i.x,
        i.y,
        i.width,
        v.title,
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let mut fields = vec![(2, "name", &v.input, ClientRenameField::Name)];
    if two_fields {
        fields.push((4, "path", &v.path_input, ClientRenameField::Path));
    }
    let mut rename_fields = Vec::new();
    let mut cursor = None;
    for (offset, label, editor, field) in fields {
        let rect = Rect::new(i.x, i.y + offset, i.width, 1);
        let focused = v.field == field || !two_fields;
        let style = Style::default()
            .fg(p.text)
            .bg(if focused { p.surface0 } else { p.surface1 });
        b.set_style(rect, style);
        let mut value = Rect::new(rect.x + 1, rect.y, rect.width.saturating_sub(1), 1);
        if two_fields {
            let prefix = format!("{label:<5} ");
            put_text(b, value.x, value.y, value.width, &prefix, style);
            let used = display_width(&prefix).min(value.width);
            value = Rect::new(value.x + used, value.y, value.width - used, 1);
        }
        let field_cursor = if field == ClientRenameField::Path && !v.path_loaded {
            put_text(b, value.x, value.y, value.width, "…", style);
            None
        } else {
            text_editor::render(b, value, editor, style)
        };
        rename_fields.push((rect, field));
        if focused {
            cursor = field_cursor;
        }
    }
    let rs = row(i, &[8, 10, 12], 2, if two_fields { 6 } else { 3 });
    let [save, clear, cancel] = rs.as_slice() else {
        return None;
    };
    button(
        b,
        *save,
        " ↵ save ",
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    let n = Style::default()
        .fg(p.text)
        .bg(p.surface0)
        .add_modifier(Modifier::BOLD);
    button(b, *clear, " ^c clear ", n);
    button(b, *cancel, " esc cancel ", n);
    Some(OverlayRender {
        area: q,
        primary: *save,
        clear: *clear,
        cancel: *cancel,
        rename_fields,
        navigator_popup: Rect::default(),
        navigator_search: Rect::default(),
        navigator_rows: Vec::new(),
        worktree_search: Rect::default(),
        worktree_rows: Vec::new(),
        cursor,
        ..OverlayRender::default()
    })
}

pub(in crate::client::shell) const NAVIGATOR_PREVIEW_HEADER_ROWS: u16 = 4;
const NAVIGATOR_SPLIT_MIN_WIDTH: u16 = 100;
const NAVIGATOR_LABEL_SHARE_PERCENT: usize = 60;
const NAVIGATOR_LABEL_MIN_WIDTH: usize = 12;
const NAVIGATOR_POPUP_PERCENT: u16 = 80;
const NAVIGATOR_POPUP_MIN_COLS: u16 = 60;
const NAVIGATOR_POPUP_MIN_ROWS: u16 = 16;
const NAVIGATOR_PAD_X: u16 = 2;
const NAVIGATOR_INDENT: u16 = 2;
const NAVIGATOR_CARET_GUTTER: u16 = 2;

pub(in crate::client::shell) struct NavigatorGeometry {
    pub(in crate::client::shell) popup: Rect,
    pub(in crate::client::shell) inner: Rect,
    pub(in crate::client::shell) search: Rect,
    pub(in crate::client::shell) tree: Rect,
    pub(in crate::client::shell) preview: Option<Rect>,
    pub(in crate::client::shell) detail: Option<Rect>,
    pub(in crate::client::shell) footer: Rect,
}

pub(in crate::client::shell) fn navigator_geometry(area: Rect) -> Option<NavigatorGeometry> {
    let fit = |extent: u16, minimum: u16| {
        let scaled = (u32::from(extent) * u32::from(NAVIGATOR_POPUP_PERCENT) / 100) as u16;
        scaled.max(minimum.min(extent.saturating_sub(2)))
    };
    let width = fit(area.width, NAVIGATOR_POPUP_MIN_COLS);
    let height = fit(area.height, NAVIGATOR_POPUP_MIN_ROWS);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    if popup.width < 4 || popup.height < 6 {
        return None;
    }
    let inner = Rect::new(popup.x + 1, popup.y + 1, popup.width - 2, popup.height - 2);
    let pad = NAVIGATOR_PAD_X.min(inner.width / 2);
    let content = Rect::new(
        inner.x + pad,
        inner.y,
        inner.width.saturating_sub(pad * 2),
        inner.height,
    );
    let search = Rect::new(content.x, inner.y + 1, content.width, 1);
    let footer = Rect::new(
        content.x,
        inner.bottom().saturating_sub(2),
        content.width,
        1,
    );
    let split = content.width >= NAVIGATOR_SPLIT_MIN_WIDTH;
    let body_y = inner.y + 3;
    let body_bottom = inner.bottom().saturating_sub(if split { 3 } else { 4 });
    let body_height = body_bottom.saturating_sub(body_y);
    if !split {
        let detail_y = inner.bottom().saturating_sub(3);
        return Some(NavigatorGeometry {
            popup,
            inner,
            search,
            tree: Rect::new(content.x, body_y, content.width, body_height),
            preview: None,
            detail: (body_height > 0).then(|| Rect::new(content.x, detail_y, content.width, 1)),
            footer,
        });
    }
    let tree_width = (u32::from(content.width) * 45 / 100).clamp(40, 80) as u16;
    let tree = Rect::new(content.x, body_y, tree_width, body_height);
    let preview_x = tree.right() + 3;
    let preview = Rect::new(
        preview_x,
        body_y,
        content.right().saturating_sub(preview_x),
        body_height,
    );
    Some(NavigatorGeometry {
        popup,
        inner,
        search,
        tree,
        preview: Some(preview),
        detail: None,
        footer,
    })
}

pub(in crate::client::shell) fn navigator_preview_capacity(cols: u16, rows: u16) -> Option<u16> {
    navigator_geometry(Rect::new(0, 0, cols, rows))?
        .preview
        .map(|preview| preview.height.saturating_sub(NAVIGATOR_PREVIEW_HEADER_ROWS))
        .filter(|capacity| *capacity > 0)
}

/// Paints `text` truncated to `cap` columns and returns the width drawn. `hits`
/// are char indices into `text`; the ellipsis added by truncation is never lit.
pub(in crate::client::shell) fn paint_navigator_text(
    b: &mut Buffer,
    x: u16,
    y: u16,
    cap: usize,
    text: &str,
    hits: &[usize],
    style: Style,
    highlight: Style,
) -> u16 {
    let shown = crate::ui::truncate_end(text, cap);
    let drawn = display_width(&shown);
    put_text(b, x, y, drawn, &shown, style);
    let chars = shown.chars().count();
    let lit_chars = if shown == text {
        chars
    } else {
        chars.saturating_sub(1)
    };
    let mut column = 0u16;
    let mut next_hit = hits.iter().copied().peekable();
    for (index, ch) in shown.chars().enumerate() {
        if index >= lit_chars {
            break;
        }
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        while next_hit.next_if(|hit| *hit < index).is_some() {}
        if next_hit.next_if(|hit| *hit == index).is_some() && width > 0 {
            let mut buf = [0u8; 4];
            put_text(
                b,
                x.saturating_add(column),
                y,
                width,
                ch.encode_utf8(&mut buf),
                highlight,
            );
        }
        column = column.saturating_add(width);
    }
    drawn
}

fn render_navigator_overlay(
    b: &mut Buffer,
    n: &ClientNavigatorOverlay,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    p: &Palette,
    pulse_phase: u8,
) -> Option<OverlayRender> {
    let g = navigator_geometry(b.area)?;
    let q = g.popup;
    let i = panel(b, q, p.peach, p.panel_bg)?;
    debug_assert_eq!(i, g.inner);
    put_text(
        b,
        q.x + 3,
        q.y,
        q.width.saturating_sub(4),
        " goto ",
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let rows = super::aggregate_navigation::navigator_rows(endpoints, active_endpoint_id, n);
    let search = g.search;
    put_text(
        b,
        search.x,
        search.y,
        search.width,
        "› ",
        Style::default()
            .fg(p.accent)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    if !n.search_focused {
        let (text, color) = if n.query.is_empty() {
            ("search panes", p.overlay0)
        } else {
            (n.query.as_str(), p.text)
        };
        put_text(
            b,
            search.x + 2,
            search.y,
            search.width.saturating_sub(2),
            text,
            Style::default().fg(color).bg(p.panel_bg),
        );
    }
    let pane_count = |rows: &[ClientNavigatorRow]| {
        rows.iter()
            .filter(|row| matches!(row.target, ClientNavigatorTarget::Pane { .. }))
            .count()
    };
    let noun = if n.agents_only { "agents" } else { "panes" };
    let total = super::aggregate_navigation::navigator_pane_total(endpoints, n);
    let mut count = if n.query.trim().is_empty() {
        format!("{total} {noun}")
    } else {
        format!("{} of {total} {noun}", pane_count(&rows))
    };
    if let Some(f) = n.filter {
        count = format!(
            "{} · {count}",
            match f {
                ClientNavigatorFilter::Blocked => "blocked",
                ClientNavigatorFilter::Working => "working",
                ClientNavigatorFilter::Idle => "idle",
                ClientNavigatorFilter::Done => "done",
            }
        );
    }
    let cursor = if n.search_focused {
        text_editor::render(
            b,
            Rect::new(
                search.x + 2,
                search.y,
                search.width.saturating_sub(3 + display_width(&count)),
                1,
            ),
            &n.query,
            Style::default().fg(p.text).bg(p.panel_bg),
        )
    } else {
        None
    };
    put_right_text(
        b,
        search,
        search.y,
        &count,
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    let selected = super::aggregate_navigation::navigator_selected_index(&rows, n).unwrap_or(0);
    let lines = super::aggregate_navigation::navigator_lines(&rows);
    let mut line_of = vec![0; rows.len()];
    for (line, row) in lines.iter().enumerate() {
        if let Some(row) = row {
            line_of[*row] = line;
        }
    }
    let selected_line = line_of.get(selected).copied().unwrap_or(0);
    let max = lines.len().saturating_sub(g.tree.height as usize);
    let scroll = n
        .scroll
        .max(selected_line.saturating_sub(g.tree.height.saturating_sub(1) as usize))
        .min(selected_line)
        .min(max);
    let scrollbar = (max > 0 && !n.keys_open)
        .then(|| Rect::new(g.tree.right().saturating_sub(1), g.tree.y, 1, g.tree.height));
    let body = match scrollbar {
        Some(_) => Rect::new(
            g.tree.x,
            g.tree.y,
            g.tree.width.saturating_sub(1),
            g.tree.height,
        ),
        None => g.tree,
    };
    let scroll_metrics = crate::pane::ScrollMetrics {
        offset_from_bottom: max.saturating_sub(scroll),
        max_offset_from_bottom: max,
        viewport_rows: usize::from(g.tree.height),
    };
    let mut row_hits = Vec::new();
    if !n.keys_open {
        for (ix, r) in rows.iter().enumerate() {
            let line = line_of[ix];
            if line < scroll {
                continue;
            }
            if line >= scroll + body.height as usize {
                break;
            }
            let rect = Rect::new(body.x, body.y + (line - scroll) as u16, body.width, 1);
            let is_workspace = matches!(r.target, ClientNavigatorTarget::Workspace { .. });
            let hit = if is_workspace {
                let caret_x = rect
                    .x
                    .saturating_add(NAVIGATOR_INDENT * u16::from(r.depth))
                    .saturating_sub(NAVIGATOR_CARET_GUTTER);
                Rect::new(caret_x, rect.y, rect.right().saturating_sub(caret_x), 1)
            } else {
                rect
            };
            row_hits.push((hit, r.target.clone()));
            let selected_row = ix == selected;
            let st = if r.stale {
                Style::default()
                    .fg(p.overlay0)
                    .bg(if selected_row { p.surface0 } else { p.panel_bg })
                    .add_modifier(Modifier::DIM)
            } else if selected_row {
                Style::default()
                    .fg(contrast(p))
                    .bg(p.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(if r.current { p.text } else { p.subtext0 })
                    .bg(p.panel_bg)
            };
            b.set_style(rect, st);
            let heading = is_workspace || matches!(r.target, ClientNavigatorTarget::Machine { .. });
            let label_style = if heading {
                st.add_modifier(Modifier::BOLD)
            } else {
                st
            };
            let highlight_style = if selected_row {
                st.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                st.fg(p.peach).add_modifier(Modifier::BOLD)
            };
            if let ClientNavigatorTarget::Workspace {
                endpoint_id,
                workspace_id,
            } = &r.target
            {
                let expanded = n
                    .expanded_workspaces
                    .contains(&(endpoint_id.clone(), workspace_id.clone()));
                put_text(
                    b,
                    hit.x,
                    hit.y,
                    NAVIGATOR_CARET_GUTTER,
                    if expanded { "▾" } else { "▸" },
                    Style::default().fg(p.overlay0).bg(p.panel_bg),
                );
            }
            let machine_status = match &r.target {
                ClientNavigatorTarget::Machine { endpoint_id } if !endpoint_id.is_local() => {
                    endpoints
                        .iter()
                        .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
                        .map(|endpoint| endpoint.status)
                }
                _ => None,
            };
            let mut right_edge = rect.right();
            if let Some(status) = machine_status {
                let (glyph, state, color) = endpoint_status_presentation(status, p);
                let signal = if status == ClientEndpointStatus::Online {
                    glyph.to_owned()
                } else {
                    format!("{glyph} {state}")
                };
                let signal_style = if selected_row {
                    st
                } else {
                    Style::default()
                        .fg(color)
                        .bg(p.panel_bg)
                        .add_modifier(if r.stale {
                            Modifier::DIM
                        } else {
                            Modifier::empty()
                        })
                };
                put_right_text(b, rect, rect.y, &signal, signal_style);
                right_edge = right_edge.saturating_sub(display_width(&signal).saturating_add(1));
            }
            if let Some(status) = r.status {
                let dot_x = rect.right().saturating_sub(1);
                let status_style = if r.stale || selected_row {
                    st
                } else {
                    status_dot_style(status, p, pulse_phase).bg(p.panel_bg)
                };
                put_text(
                    b,
                    dot_x,
                    rect.y,
                    display_width(status_dot(status)),
                    status_dot(status),
                    status_style,
                );
                right_edge = dot_x.saturating_sub(1);
            }
            let indent = NAVIGATOR_INDENT * u16::from(r.depth);
            let mut label_x = rect.x.saturating_add(indent);
            if r.current {
                put_text(
                    b,
                    label_x,
                    rect.y,
                    right_edge.saturating_sub(label_x),
                    "◆ ",
                    st,
                );
                label_x = label_x.saturating_add(2);
            }
            let available = usize::from(right_edge.saturating_sub(label_x));
            let label_cap = if r.meta.is_empty() {
                available
            } else {
                let budget = usize::from(rect.width) * NAVIGATOR_LABEL_SHARE_PERCENT / 100;
                let floor = budget
                    .saturating_sub(usize::from(label_x - rect.x))
                    .max(NAVIGATOR_LABEL_MIN_WIDTH)
                    .min(available);
                available
                    .saturating_sub(usize::from(display_width(&r.meta)) + 1)
                    .max(floor)
            };
            let label_width = paint_navigator_text(
                b,
                label_x,
                rect.y,
                label_cap,
                &r.label,
                &r.label_hits,
                label_style,
                highlight_style,
            );
            if !r.meta.is_empty() {
                let meta_start = label_x.saturating_add(label_width).saturating_add(1);
                let meta_cap = usize::from(right_edge.saturating_sub(meta_start));
                let meta_width = display_width(&crate::ui::truncate_end(&r.meta, meta_cap));
                if meta_width > 0 {
                    let meta_style = if selected_row || r.stale {
                        st
                    } else {
                        st.fg(p.overlay0)
                    };
                    paint_navigator_text(
                        b,
                        right_edge - meta_width,
                        rect.y,
                        usize::from(meta_width),
                        &r.meta,
                        &r.meta_hits,
                        meta_style,
                        highlight_style,
                    );
                }
            }
        }
    }
    if let Some(track) = scrollbar {
        crate::ui::render_scrollbar_buffer(b, scroll_metrics, track, p.overlay0, p.overlay1, "▐");
    }
    if n.keys_open {
        let right = g.preview.map_or(g.tree.right(), |preview| preview.right());
        let area = Rect::new(
            g.tree.x,
            g.tree.y,
            right.saturating_sub(g.tree.x),
            g.tree.height,
        );
        render_navigator_key_sheet(b, area, n.search_focused, p);
    } else if let Some(preview) = g.preview {
        for y in preview.y..preview.bottom() {
            put_text(
                b,
                g.tree.right() + 1,
                y,
                1,
                "│",
                Style::default().fg(p.surface1).bg(p.panel_bg),
            );
        }
        render_navigator_preview(b, preview, n, &rows, endpoints, p);
    }
    if let Some(detail) = g.detail.filter(|_| !n.keys_open) {
        if let Some(r) = rows.get(selected) {
            let context = navigator_row_context(endpoints, &r.target);
            let text = match (&r.target, context) {
                (ClientNavigatorTarget::Pane { .. }, context) => {
                    let head = match context {
                        Some(context) => format!("{context} › {}", r.title),
                        None => r.title.clone(),
                    };
                    if r.cwd.is_empty() {
                        format!(" {head}")
                    } else {
                        let budget = usize::from(detail.width)
                            .saturating_sub(usize::from(display_width(&r.cwd)) + 4)
                            .max(NAVIGATOR_LABEL_MIN_WIDTH);
                        format!(" {} · {}", crate::ui::truncate_end(&head, budget), r.cwd)
                    }
                }
                (ClientNavigatorTarget::Tab { .. }, Some(context)) => {
                    format!(" {context} › {}", r.label)
                }
                _ => format!(" {}", r.label),
            };
            put_text(
                b,
                detail.x,
                detail.y,
                detail.width,
                &text,
                Style::default().fg(p.overlay0).bg(p.panel_bg),
            );
        }
    }
    let pairs: &[(&str, &str)] = if n.keys_open {
        &[("esc", "close keys")]
    } else if n.search_focused {
        &[
            ("↑↓", "move"),
            ("enter", "open"),
            ("tab", "tree"),
            ("F1", "keys"),
            ("esc", "close"),
        ]
    } else {
        &[
            ("j/k", "move"),
            ("enter", "open"),
            ("/", "search"),
            ("?", "keys"),
            ("esc", "close"),
        ]
    };
    let mut x = g.footer.x;
    for (index, (key, word)) in pairs.iter().enumerate() {
        if index > 0 {
            x = x.saturating_add(3);
        }
        for (text, color) in [(*key, p.text), (" ", p.overlay0), (*word, p.overlay0)] {
            let remaining = g.footer.right().saturating_sub(x);
            put_text(
                b,
                x,
                g.footer.y,
                remaining,
                text,
                Style::default().fg(color).bg(p.panel_bg),
            );
            x = x.saturating_add(display_width(text).min(remaining));
        }
    }
    Some(OverlayRender {
        area: q,
        primary: Rect::default(),
        clear: Rect::default(),
        cancel: Rect::default(),
        navigator_popup: q,
        navigator_search: g.search,
        navigator_rows: row_hits,
        navigator_preview: g.preview.filter(|_| !n.keys_open).unwrap_or_default(),
        navigator_scrollbar: scrollbar.unwrap_or_default(),
        navigator_scroll_metrics: scrollbar.is_some().then_some(scroll_metrics),
        worktree_search: Rect::default(),
        worktree_rows: Vec::new(),
        cursor,
        ..OverlayRender::default()
    })
}

/// The workspace label of a pane or tab target, prefixed by its machine label
/// when more than one endpoint is listed, for the narrow detail line.
fn navigator_row_context(
    endpoints: &[ClientShellEndpoint],
    target: &ClientNavigatorTarget,
) -> Option<String> {
    let (ClientNavigatorTarget::Pane { endpoint_id, .. }
    | ClientNavigatorTarget::Tab { endpoint_id, .. }) = target
    else {
        return None;
    };
    let endpoint = endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == endpoint_id)?;
    let snapshot = endpoint.snapshot.as_deref()?;
    let workspace_id = match target {
        ClientNavigatorTarget::Pane { pane_id, .. } => snapshot
            .panes
            .iter()
            .find(|pane| &pane.pane_id == pane_id)
            .map(|pane| pane.workspace_id.as_str()),
        ClientNavigatorTarget::Tab { tab_id, .. } => snapshot
            .tabs
            .iter()
            .find(|tab| &tab.tab_id == tab_id)
            .map(|tab| tab.workspace_id.as_str()),
        _ => None,
    }?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)?;
    Some(if endpoints.len() > 1 {
        format!("{} › {}", endpoint.label, workspace.label)
    } else {
        workspace.label.clone()
    })
}

const NAVIGATOR_SEARCH_KEYS: &[(&str, &str)] = &[
    ("type", "search tab, pane and agent names"),
    ("↑↓  ctrl+j/k  ctrl+n/p", "move"),
    ("enter", "open"),
    ("tab", "tree mode"),
    ("alt+a", "agents only"),
    ("alt+b/w/i/d", "filter blocked / working / idle / done"),
    ("pgup/pgdn", "scroll preview"),
    ("F1", "close this list"),
    ("esc", "clear search, then close"),
];

const NAVIGATOR_TREE_KEYS: &[(&str, &str)] = &[
    ("j/k  ↑↓  ctrl+j/k", "move"),
    ("ctrl+d/u", "move 8"),
    ("h/l  ←→", "previous / next workspace"),
    ("home  end/G", "first / last"),
    ("space", "collapse workspace"),
    ("enter", "open"),
    ("/  tab", "search"),
    ("a", "agents only"),
    ("b/w/i/d", "filter blocked / working / idle / done"),
    ("backspace", "clear filter"),
    ("p", "edit workspace path"),
    ("ctrl+o", "new workspace"),
    ("pgup/pgdn", "scroll preview"),
    ("?  F1", "close this list"),
    ("esc", "close"),
];

const NAVIGATOR_KEY_SHEET_GAP: u16 = 3;
const NAVIGATOR_KEY_SHEET_MIN_ACTION: u16 = 6;

pub(in crate::client::shell) struct NavigatorKeySheetColumn {
    pub(in crate::client::shell) x: u16,
    pub(in crate::client::shell) key_width: u16,
    pub(in crate::client::shell) action_width: u16,
    pub(in crate::client::shell) entries: Vec<(&'static str, &'static str)>,
}

/// Lays the current mode's keys out in as many columns as `width` fits with
/// `capacity` rows each; `x` is relative to the sheet. Every column gets its
/// widest key first, so keys are never clipped while the columns fit, and the
/// width left over goes to the actions, smallest need first, so an action is
/// clipped only when the columns' total does not fit. When even that runs out
/// of rows, entries are dropped from the middle so the closing keys at the end
/// stay listed.
pub(in crate::client::shell) fn navigator_key_sheet_columns(
    width: u16,
    capacity: usize,
    search_focused: bool,
) -> Vec<NavigatorKeySheetColumn> {
    let entries = if search_focused {
        NAVIGATOR_SEARCH_KEYS
    } else {
        NAVIGATOR_TREE_KEYS
    };
    if capacity == 0 || width == 0 {
        return Vec::new();
    }
    let key_width = |entries: &[(&str, &str)]| {
        entries
            .iter()
            .map(|(key, _)| display_width(key))
            .max()
            .unwrap_or(0)
            + 2
    };
    let action_width = |entries: &[(&str, &str)]| {
        entries
            .iter()
            .map(|(_, action)| display_width(action))
            .max()
            .unwrap_or(0)
    };
    let closing = entries
        .iter()
        .rev()
        .take_while(|(_, action)| action.contains("close"))
        .count();
    let select = |slots: usize| -> Vec<(&'static str, &'static str)> {
        if entries.len() <= slots {
            return entries.to_vec();
        }
        let tail = slots.div_ceil(2).max(closing.min(slots));
        entries[..slots - tail]
            .iter()
            .chain(&entries[entries.len() - tail..])
            .copied()
            .collect()
    };
    let gaps =
        |columns: usize| NAVIGATOR_KEY_SHEET_GAP.saturating_mul(columns.saturating_sub(1) as u16);
    let fits = |shown: &[(&str, &str)]| {
        let minimum: u16 = shown
            .chunks(capacity)
            .map(|chunk| key_width(chunk) + NAVIGATOR_KEY_SHEET_MIN_ACTION)
            .sum();
        minimum.saturating_add(gaps(shown.chunks(capacity).len())) <= width
    };
    let mut columns = entries.len().div_ceil(capacity).max(1);
    let mut shown = select(columns * capacity);
    while columns > 1 && !fits(&shown) {
        columns -= 1;
        shown = select(columns * capacity);
    }
    let chunks: Vec<&[(&'static str, &'static str)]> = shown.chunks(capacity).collect();
    let keys: Vec<u16> = chunks.iter().map(|chunk| key_width(chunk)).collect();
    let needs: Vec<u16> = chunks.iter().map(|chunk| action_width(chunk)).collect();
    let mut remaining = width
        .saturating_sub(gaps(chunks.len()))
        .saturating_sub(keys.iter().sum());
    let mut actions = vec![0u16; chunks.len()];
    let mut by_need: Vec<usize> = (0..chunks.len()).collect();
    by_need.sort_by_key(|&index| needs[index]);
    for (done, &index) in by_need.iter().enumerate() {
        let fair = remaining / (chunks.len() - done) as u16;
        actions[index] = needs[index].min(fair);
        remaining -= actions[index];
    }
    let mut x = 0u16;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let key_width = keys[index].min(width.saturating_sub(x));
            let column = NavigatorKeySheetColumn {
                x,
                key_width,
                action_width: actions[index],
                entries: chunk.to_vec(),
            };
            x = x
                .saturating_add(key_width + actions[index])
                .saturating_add(NAVIGATOR_KEY_SHEET_GAP);
            column
        })
        .collect()
}

fn render_navigator_key_sheet(b: &mut Buffer, area: Rect, search_focused: bool, p: &Palette) {
    if area.height == 0 {
        return;
    }
    put_text(
        b,
        area.x,
        area.y,
        area.width,
        if search_focused {
            "Search keys"
        } else {
            "Tree keys"
        },
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let capacity = usize::from(area.height - 1);
    for column in navigator_key_sheet_columns(area.width, capacity, search_focused) {
        let x = area.x + column.x;
        for (line, (key, action)) in column.entries.iter().enumerate() {
            let y = area.y + 1 + line as u16;
            put_text(
                b,
                x,
                y,
                column.key_width,
                &crate::ui::truncate_end(key, usize::from(column.key_width)),
                Style::default().fg(p.text).bg(p.panel_bg),
            );
            put_text(
                b,
                x + column.key_width,
                y,
                column.action_width,
                &crate::ui::truncate_end(action, usize::from(column.action_width)),
                Style::default().fg(p.overlay0).bg(p.panel_bg),
            );
        }
    }
}

fn render_navigator_preview(
    b: &mut Buffer,
    area: Rect,
    n: &ClientNavigatorOverlay,
    rows: &[ClientNavigatorRow],
    endpoints: &[ClientShellEndpoint],
    p: &Palette,
) {
    let muted = Style::default().fg(p.overlay0).bg(p.panel_bg);
    let text = Style::default().fg(p.text).bg(p.panel_bg);
    let selected = super::aggregate_navigation::navigator_selected_index(rows, n)
        .and_then(|index| rows.get(index));
    let Some(row) = selected else {
        put_text(b, area.x, area.y, area.width, "select a pane", muted);
        return;
    };
    let ClientNavigatorTarget::Pane {
        endpoint_id,
        pane_id,
    } = &row.target
    else {
        put_text(b, area.x, area.y, area.width, "select a pane", muted);
        return;
    };
    let endpoint = endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == endpoint_id);
    let kind = endpoint
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find(|agent| &agent.pane_id == pane_id)
        })
        .and_then(|agent| agent.agent.as_deref())
        .unwrap_or("shell");
    let status = row.status.map(status_text).unwrap_or("unknown");
    put_text(
        b,
        area.x,
        area.y,
        area.width,
        &row.title,
        text.add_modifier(Modifier::BOLD),
    );
    put_text(b, area.x, area.y + 1, area.width, &row.cwd, muted);
    put_text(
        b,
        area.x,
        area.y + 2,
        area.width,
        &format!("{kind} · {status}"),
        muted,
    );
    let body = Rect::new(
        area.x,
        area.y + NAVIGATOR_PREVIEW_HEADER_ROWS,
        area.width,
        area.height.saturating_sub(NAVIGATOR_PREVIEW_HEADER_ROWS),
    );
    if body.height == 0 {
        return;
    }
    let placeholder = if row.stale {
        Some(format!(
            "{} is offline",
            endpoint.map_or("machine", |endpoint| endpoint.label.as_str())
        ))
    } else {
        match &n.preview {
            Some(preview) if &preview.pane_id == pane_id && &preview.endpoint_id == endpoint_id => {
                match &preview.error {
                    Some(error) if error.starts_with("preview ") => Some(error.clone()),
                    Some(error) => Some(format!("preview unavailable: {error}")),
                    None if preview.lines.is_empty() && preview.received_at.is_none() => {
                        Some("loading".to_owned())
                    }
                    None => None,
                }
            }
            _ => Some("loading".to_owned()),
        }
    };
    if let Some(placeholder) = placeholder {
        put_text(b, body.x, body.y, body.width, &placeholder, muted);
        return;
    }
    let Some(preview) = n.preview.as_ref() else {
        return;
    };
    let held = preview.lines.len();
    let end = held
        .saturating_sub(preview.scroll)
        .max(held.min(body.height as usize));
    let start = end.saturating_sub(body.height as usize);
    for (offset, line) in preview.lines[start..end].iter().enumerate() {
        put_spans(b, body.x, body.y + offset as u16, body.width, line, text);
    }
    let shown_offset = held - end;
    if shown_offset > 0 {
        put_right_text(b, area, area.y, &format!("↑{shown_offset} "), muted);
    }
}

fn put_spans(
    b: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    line: &super::preview_ansi::StyledLine,
    base: Style,
) {
    let mut cursor = x;
    let right = x.saturating_add(width);
    for (text, style) in line {
        if cursor >= right {
            break;
        }
        put_text(b, cursor, y, right - cursor, text, base.patch(*style));
        cursor = cursor.saturating_add(display_width(text));
    }
}

fn help_lines(
    keybinds: &LiveKeybindConfig,
    query: &str,
    palette: &Palette,
) -> Vec<(usize, ratatui::text::Line<'static>)> {
    use ratatui::text::{Line, Span};

    let groups = crate::input::filter_keybind_help_groups(
        crate::input::keybind_help_groups(&keybinds.keybinds, &keybinds.prefix),
        query,
    );
    let key_width = groups
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|(key, _)| key.chars().count()))
        .max()
        .unwrap_or(8);
    if groups.is_empty() {
        let message = " no matching keybinds";
        return vec![(
            message.chars().count(),
            Line::from(Span::styled(
                message,
                Style::default().fg(palette.overlay1).bg(palette.panel_bg),
            )),
        )];
    }

    let mut lines = Vec::new();
    for (group, entries) in groups {
        lines.push((
            group.len() + 1,
            Line::from(Span::styled(
                format!(" {group}"),
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD),
            )),
        ));
        for (key, label) in entries {
            let padded_key = format!(" {key:<key_width$} ");
            let width = padded_key.chars().count() + label.chars().count();
            lines.push((
                width,
                Line::from(vec![
                    Span::styled(
                        padded_key,
                        Style::default()
                            .fg(palette.mauve)
                            .bg(palette.panel_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        label.into_owned(),
                        Style::default().fg(palette.text).bg(palette.panel_bg),
                    ),
                ]),
            ));
        }
        lines.push((0, Line::raw("")));
    }
    lines
}

fn render_help_overlay(
    b: &mut Buffer,
    h: &ClientHelpOverlay,
    k: &LiveKeybindConfig,
    p: &Palette,
) -> Option<OverlayRender> {
    use ratatui::widgets::{Paragraph, Widget, Wrap};

    let q = popup(b.area, 76, 22)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    if i.width < 20 || i.height < 6 {
        return None;
    }
    put_text(
        b,
        i.x,
        i.y,
        i.width,
        "keybinds",
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let close = Rect::new(i.right() - 13, i.y, 13, 1);
    button(
        b,
        close,
        if h.search_focused {
            " esc back "
        } else {
            " esc close "
        },
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    let sy = i.y + 1;
    put_text(
        b,
        i.x,
        sy,
        i.width,
        &if h.search_focused {
            " / ".to_owned()
        } else {
            " / press / to filter by command or shortcut".to_owned()
        },
        Style::default()
            .fg(if h.search_focused { p.text } else { p.overlay0 })
            .bg(p.panel_bg),
    );
    let cursor = if h.search_focused {
        text_editor::render(
            b,
            Rect::new(i.x + 3, sy, i.width.saturating_sub(3), 1),
            &h.query,
            Style::default().fg(p.text).bg(p.panel_bg),
        )
    } else {
        None
    };

    let body = Rect::new(i.x, i.y + 3, i.width, i.height.saturating_sub(5));
    let lines = help_lines(k, &h.query, p);
    let viewport_rows = usize::from(body.height.max(1));
    let wrapped_rows = |width: u16| {
        let width = usize::from(width.max(1));
        lines
            .iter()
            .map(|(line_width, _)| line_width.max(&1).div_ceil(width))
            .sum::<usize>()
    };
    let needs_scrollbar = wrapped_rows(body.width) > viewport_rows;
    let text_area = if needs_scrollbar {
        Rect::new(body.x, body.y, body.width.saturating_sub(1), body.height)
    } else {
        body
    };
    let total_rows = wrapped_rows(text_area.width);
    let max_scroll = total_rows.saturating_sub(viewport_rows);
    let scroll = h.scroll.min(max_scroll);
    let metrics = crate::pane::ScrollMetrics {
        offset_from_bottom: max_scroll.saturating_sub(scroll),
        max_offset_from_bottom: max_scroll,
        viewport_rows,
    };
    let scrollbar = needs_scrollbar.then_some(Rect::new(
        body.right().saturating_sub(1),
        body.y,
        1,
        body.height,
    ));
    Widget::render(
        Paragraph::new(lines.into_iter().map(|(_, line)| line).collect::<Vec<_>>())
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        text_area,
        b,
    );
    if let Some(track) = scrollbar {
        if let Some(thumb) = crate::ui::scrollbar_thumb(metrics, track) {
            for y in track.y..track.bottom() {
                b[(track.x, y)]
                    .set_symbol("▐")
                    .set_style(Style::default().fg(p.overlay0).bg(p.panel_bg));
            }
            for y in thumb.top..thumb.top.saturating_add(thumb.len) {
                b[(track.x, y)]
                    .set_symbol("▐")
                    .set_style(Style::default().fg(p.overlay1).bg(p.panel_bg));
            }
        }
    }

    put_text(
        b,
        i.x,
        i.bottom() - 1,
        i.width,
        if h.search_focused {
            " edit ←→/home/end · kill ^u/^k · yank ^y · scroll ↑↓ · back esc"
        } else {
            " search / · scroll j/k/↑↓/pgup/pgdn · close esc/enter"
        },
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    Some(OverlayRender {
        area: q,
        cancel: close,
        help_popup: q,
        help_scrollbar: scrollbar.unwrap_or_default(),
        help_scroll_metrics: Some(metrics),
        help_max_scroll: max_scroll,
        cursor,
        ..OverlayRender::default()
    })
}
fn render_confirm_close_overlay(
    b: &mut Buffer,
    c: &ClientConfirmCloseOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let q = popup(b.area, 64, 6)?;
    let i = panel(b, q, p.red, p.panel_bg)?;
    put_text(
        b,
        i.x,
        i.y,
        i.width,
        &format!(" {}", c.title),
        Style::default()
            .fg(p.red)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        b,
        i.x,
        i.y + 1,
        i.width,
        &format!(" {}", c.detail),
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    let rs = row(i, &[13, 12], 2, 3);
    let [ok, cancel] = rs.as_slice() else {
        return None;
    };
    button(
        b,
        *ok,
        " ↵ confirm ",
        Style::default()
            .fg(contrast(p))
            .bg(p.red)
            .add_modifier(Modifier::BOLD),
    );
    button(
        b,
        *cancel,
        " esc cancel ",
        Style::default()
            .fg(p.text)
            .bg(p.surface0)
            .add_modifier(Modifier::BOLD),
    );
    Some(OverlayRender {
        area: q,
        primary: *ok,
        clear: Rect::default(),
        cancel: *cancel,
        navigator_popup: Rect::default(),
        navigator_search: Rect::default(),
        navigator_rows: Vec::new(),
        worktree_search: Rect::default(),
        worktree_rows: Vec::new(),
        cursor: None,
        ..OverlayRender::default()
    })
}
