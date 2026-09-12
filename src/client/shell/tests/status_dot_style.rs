use super::*;
use ratatui::style::Color;

#[test]
fn blocked_cycles_through_four_styles_on_an_rgb_palette() {
    let palette = Palette::catppuccin();
    let phases = (0..PULSE_PHASES)
        .map(|phase| status_dot_style(AgentStatus::Blocked, &palette, phase))
        .collect::<Vec<_>>();
    assert_eq!(phases[0].fg, Some(palette.red), "phase 0 is full red");
    assert_eq!(phases[1], phases[3], "the cycle is symmetric");
    assert_ne!(phases[0], phases[1]);
    assert_ne!(phases[1], phases[2]);
    assert!(matches!(phases[2].fg, Some(Color::Rgb(..))));
    assert_eq!(
        status_dot_style(AgentStatus::Blocked, &palette, PULSE_PHASES),
        phases[0],
        "phase wraps"
    );
}

#[test]
fn blocked_steps_through_weight_on_a_named_palette() {
    let palette = Palette::terminal();
    let s = |phase| status_dot_style(AgentStatus::Blocked, &palette, phase);
    assert_eq!(
        s(0),
        Style::default()
            .fg(palette.red)
            .add_modifier(Modifier::BOLD)
    );
    assert_eq!(s(1), Style::default().fg(Color::Red));
    assert_eq!(
        s(2),
        Style::default().fg(Color::Red).add_modifier(Modifier::DIM)
    );
    assert_eq!(s(3), s(1));
}

#[test]
fn other_states_ignore_the_phase() {
    for palette in [Palette::catppuccin(), Palette::terminal()] {
        for status in [
            AgentStatus::Working,
            AgentStatus::Done,
            AgentStatus::Idle,
            AgentStatus::Unknown,
        ] {
            let steady = Style::default().fg(status_color(status, &palette));
            for phase in 0..PULSE_PHASES {
                assert_eq!(status_dot_style(status, &palette, phase), steady);
            }
        }
    }
}
