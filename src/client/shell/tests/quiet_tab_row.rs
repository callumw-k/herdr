use super::*;
use ratatui::style::Color;

fn three_tab_snapshot() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    for (id, number, label) in [("tab_2", 2, "shell"), ("tab_3", 3, "tests")] {
        snapshot.tabs.push(ClientShellTab {
            tab_id: id.into(),
            number,
            label: label.into(),
            focused: false,
            ..snapshot.tabs[0].clone()
        });
    }
    snapshot.tabs[0].label = "nvim".into();
    snapshot
}

fn extra_pane(tab_id: &str, pane_id: &str) -> ClientShellPane {
    ClientShellPane {
        pane_id: pane_id.into(),
        tab_id: tab_id.into(),
        focused: false,
        ..snapshot().panes[0].clone()
    }
}

fn config(style: TabBarStyleConfig, palette: Palette) -> ClientShellConfig {
    let mut config = ClientShellConfig::from_config(&crate::config::Config::default());
    config.tab_bar_style = style;
    config.mouse_capture = false;
    config.palette = palette;
    config
}

fn render(snapshot: &ClientShellSnapshot, config: &ClientShellConfig) -> (Buffer, ShellHitMap) {
    let area = Rect::new(0, 0, 80, 1);
    let mut buffer = Buffer::empty(area);
    let mut hits = ShellHitMap::default();
    render::render_tab_bar(
        &mut buffer,
        area,
        snapshot,
        config,
        0,
        None,
        &mut 0,
        &mut false,
        None,
        &mut hits,
    );
    (buffer, hits)
}

fn row_text(buffer: &Buffer) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, 0)].symbol().to_string())
        .collect()
}

#[test]
fn quiet_tabs_pad_three_cells_and_gap_one() {
    let (_, hits) = render(
        &three_tab_snapshot(),
        &config(TabBarStyleConfig::Quiet, Palette::catppuccin()),
    );
    assert_eq!(hits.tabs[0].0.width, 4 + 6);
    assert_eq!(hits.tabs[1].0.x, hits.tabs[0].0.right() + 1);
}

#[test]
fn quiet_tab_shows_tiled_pane_count() {
    let mut snapshot = three_tab_snapshot();
    snapshot.panes.push(extra_pane("tab_1", "pane_2"));
    let (buffer, _) = render(
        &snapshot,
        &config(TabBarStyleConfig::Quiet, Palette::catppuccin()),
    );
    let text = row_text(&buffer);
    assert!(text.contains("nvim ·2"), "{text}");
    assert_eq!(text.matches('·').count(), 1, "{text}");
}

#[test]
fn quiet_tab_count_ignores_other_tabs_panes() {
    let mut snapshot = three_tab_snapshot();
    snapshot.panes.push(extra_pane("tab_2", "pane_2"));
    snapshot.panes.push(extra_pane("tab_2", "pane_3"));
    let (buffer, _) = render(
        &snapshot,
        &config(TabBarStyleConfig::Quiet, Palette::catppuccin()),
    );
    let text = row_text(&buffer);
    assert!(text.contains("shell ·2"), "{text}");
    assert!(!text.contains("nvim ·"), "{text}");
}

#[test]
fn quiet_zoomed_tab_puts_the_count_before_the_marker() {
    let mut snapshot = three_tab_snapshot();
    snapshot.tabs[0].zoomed = true;
    snapshot.panes.push(extra_pane("tab_1", "pane_2"));
    let (buffer, _) = render(
        &snapshot,
        &config(TabBarStyleConfig::Quiet, Palette::catppuccin()),
    );
    assert!(row_text(&buffer).contains("nvim ·2 Z"));
}

#[test]
fn quiet_unselected_tab_falls_back_when_surface0_is_reset() {
    let snapshot = three_tab_snapshot();
    let terminal = Palette::terminal();
    let (buffer, hits) = render(
        &snapshot,
        &config(TabBarStyleConfig::Quiet, terminal.clone()),
    );
    let rect = hits.tabs[1].0;
    assert_eq!(buffer[(rect.x, 0)].bg, terminal.active_row_bg);
    assert_eq!(buffer[(rect.right() - 1, 0)].bg, terminal.active_row_bg);

    let catppuccin = Palette::catppuccin();
    let (buffer, hits) = render(
        &snapshot,
        &config(TabBarStyleConfig::Quiet, catppuccin.clone()),
    );
    assert_eq!(buffer[(hits.tabs[1].0.x, 0)].bg, catppuccin.surface0);
}

#[test]
fn classic_tab_row_is_unchanged() {
    let mut snapshot = three_tab_snapshot();
    snapshot.tabs[0].label = "editor".into();
    snapshot.panes.push(extra_pane("tab_1", "pane_2"));
    let (buffer, hits) = render(
        &snapshot,
        &config(TabBarStyleConfig::Classic, Palette::terminal()),
    );
    assert_eq!(hits.tabs[0].0.width, 6 + 4);
    assert_eq!(hits.tabs[1].0.x, hits.tabs[0].0.right() + 1);
    assert!(!row_text(&buffer).contains('·'));
    assert_eq!(buffer[(hits.tabs[1].0.x, 0)].bg, Color::Reset);
}
