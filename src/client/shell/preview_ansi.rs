use ratatui::style::{Color, Modifier, Style};

pub(super) type StyledLine = Vec<(String, Style)>;

pub(super) fn parse_lines(text: &str) -> Vec<StyledLine> {
    let mut lines = Vec::new();
    let mut line: StyledLine = Vec::new();
    let mut run = String::new();
    let mut style = Style::default();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                if chars.peek() != Some(&'[') {
                    continue;
                }
                chars.next();
                let mut params = String::new();
                let mut terminator = None;
                for next in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&next) {
                        terminator = Some(next);
                        break;
                    }
                    params.push(next);
                }
                if terminator == Some('m') {
                    let next = apply_sgr(style, &params);
                    if next != style {
                        flush(&mut run, style, &mut line);
                        style = next;
                    }
                }
            }
            '\n' => {
                flush(&mut run, style, &mut line);
                lines.push(std::mem::take(&mut line));
            }
            '\r' => {}
            _ => run.push(ch),
        }
    }
    flush(&mut run, style, &mut line);
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn flush(run: &mut String, style: Style, line: &mut StyledLine) {
    if !run.is_empty() {
        line.push((std::mem::take(run), style));
    }
}

fn apply_sgr(mut style: Style, params: &str) -> Style {
    let codes = params
        .split(';')
        .map(|code| code.parse::<u16>().unwrap_or(0))
        .collect::<Vec<_>>();
    if codes.is_empty() {
        return Style::default();
    }
    let mut index = 0;
    while index < codes.len() {
        match codes[index] {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            7 => style = style.add_modifier(Modifier::REVERSED),
            9 => style = style.add_modifier(Modifier::CROSSED_OUT),
            22 => style.add_modifier.remove(Modifier::BOLD | Modifier::DIM),
            23 => style.add_modifier.remove(Modifier::ITALIC),
            24 => style.add_modifier.remove(Modifier::UNDERLINED),
            27 => style.add_modifier.remove(Modifier::REVERSED),
            29 => style.add_modifier.remove(Modifier::CROSSED_OUT),
            30..=37 => style = style.fg(basic(codes[index] - 30)),
            90..=97 => style = style.fg(basic(codes[index] - 90 + 8)),
            40..=47 => style = style.bg(basic(codes[index] - 40)),
            100..=107 => style = style.bg(basic(codes[index] - 100 + 8)),
            39 => style.fg = None,
            49 => style.bg = None,
            38 | 48 => {
                let (color, consumed) = extended(&codes[index + 1..]);
                if let Some(color) = color {
                    style = if codes[index] == 38 {
                        style.fg(color)
                    } else {
                        style.bg(color)
                    };
                }
                index += consumed;
            }
            _ => {}
        }
        index += 1;
    }
    style
}

fn extended(codes: &[u16]) -> (Option<Color>, usize) {
    match codes {
        [5, n, ..] => (Some(Color::Indexed(u8::try_from(*n).unwrap_or(0))), 2),
        [2, r, g, b, ..] => (
            Some(Color::Rgb(
                u8::try_from(*r).unwrap_or(0),
                u8::try_from(*g).unwrap_or(0),
                u8::try_from(*b).unwrap_or(0),
            )),
            4,
        ),
        _ => (None, 0),
    }
}

fn basic(index: u16) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<Vec<(String, Style)>> {
        parse_lines(text)
    }

    #[test]
    fn plain_text_is_one_default_span_per_line() {
        assert_eq!(
            spans("one\ntwo"),
            vec![
                vec![("one".to_owned(), Style::default())],
                vec![("two".to_owned(), Style::default())],
            ]
        );
    }

    #[test]
    fn a_colour_change_mid_line_splits_the_line_and_reset_returns_to_default() {
        let lines = spans("a\x1b[31mb\x1b[0mc");
        assert_eq!(
            lines,
            vec![vec![
                ("a".to_owned(), Style::default()),
                ("b".to_owned(), Style::default().fg(Color::Red)),
                ("c".to_owned(), Style::default()),
            ]]
        );
    }

    #[test]
    fn attributes_stack_and_colours_survive_a_bold_toggle() {
        let lines = spans("\x1b[1;32mx\x1b[22my");
        assert_eq!(
            lines[0][0].1,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(lines[0][1].1, Style::default().fg(Color::Green));
    }

    #[test]
    fn extended_colours_parse_and_leave_following_codes_intact() {
        let lines = spans("\x1b[38;5;208;1mx\x1b[48;2;10;20;30my");
        assert_eq!(
            lines[0][0].1,
            Style::default()
                .fg(Color::Indexed(208))
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            lines[0][1].1,
            Style::default()
                .fg(Color::Indexed(208))
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(10, 20, 30))
        );
    }

    #[test]
    fn non_sgr_sequences_and_carriage_returns_are_dropped() {
        assert_eq!(
            spans("a\x1b[2Kb\r\x1b[?25lc"),
            vec![vec![("abc".to_owned(), Style::default())]]
        );
    }

    #[test]
    fn a_trailing_newline_does_not_add_an_empty_line() {
        assert_eq!(spans("a\n").len(), 1);
        assert_eq!(spans("a\n\nb").len(), 3);
    }
}
