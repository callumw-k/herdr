const CONSECUTIVE_BONUS: i32 = 8;
const BOUNDARY_BONUS: i32 = 12;
const PREFIX_BONUS: i32 = 16;
const GAP_PENALTY: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FuzzyMatch {
    pub(crate) score: i32,
    pub(crate) positions: Vec<usize>,
}

/// Scores each whitespace-separated term of `query` against `text`
/// independently, in any order. Returns `None` when any term is not a
/// subsequence of `text`, otherwise the summed score where higher is better.
pub(crate) fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    query
        .split_whitespace()
        .try_fold(0, |score, term| Some(score + fuzzy_term_score(term, text)?))
}

/// Lowercases a char only when the result is a single char, so folded text
/// keeps the same length and indices as the original.
fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// Optimal subsequence match of one term.
///
/// Dynamic programme over (term char, text index) that maximises the summed
/// prefix, boundary and consecutive bonuses less the gap penalty. The best
/// predecessor for a non-adjacent match is kept as a running maximum, so each
/// term char costs O(len(text)).
pub(crate) fn fuzzy_term_match(term: &str, text: &str) -> Option<FuzzyMatch> {
    if term.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }
    if !is_subsequence(term, text) {
        return None;
    }
    let needles: Vec<char> = term.chars().map(fold).collect();
    let haystack: Vec<char> = text.chars().map(fold).collect();

    let bonus = |j: usize| {
        if j == 0 {
            PREFIX_BONUS
        } else if is_boundary(&haystack, j) {
            BOUNDARY_BONUS
        } else {
            0
        }
    };

    let mut best: Vec<Vec<Option<i32>>> = Vec::with_capacity(needles.len());
    let mut back: Vec<Vec<usize>> = Vec::with_capacity(needles.len());

    best.push(
        (0..haystack.len())
            .map(|j| (haystack[j] == needles[0]).then(|| bonus(j) - GAP_PENALTY * j as i32))
            .collect(),
    );
    back.push(vec![0; haystack.len()]);

    for k in 1..needles.len() {
        let previous = &best[k - 1];
        let mut row: Vec<Option<i32>> = vec![None; haystack.len()];
        let mut row_back = vec![0; haystack.len()];
        let mut running: Option<(i32, usize)> = None;
        for j in 1..haystack.len() {
            if j >= 2 {
                let i = j - 2;
                if let Some(value) = previous[i] {
                    let candidate = value + GAP_PENALTY * i as i32;
                    if running.is_none_or(|(current, _)| candidate > current) {
                        running = Some((candidate, i));
                    }
                }
            }
            if haystack[j] != needles[k] {
                continue;
            }
            let mut choice: Option<(i32, usize)> =
                previous[j - 1].map(|value| (value + CONSECUTIVE_BONUS, j - 1));
            if let Some((value, i)) = running {
                let candidate = value - GAP_PENALTY * (j - 1) as i32;
                if choice.is_none_or(|(current, _)| candidate > current) {
                    choice = Some((candidate, i));
                }
            }
            if let Some((value, i)) = choice {
                row[j] = Some(bonus(j) + value);
                row_back[j] = i;
            }
        }
        best.push(row);
        back.push(row_back);
    }

    let last = needles.len() - 1;
    let mut end: Option<(i32, usize)> = None;
    for (j, value) in best[last].iter().enumerate() {
        if let Some(value) = *value {
            if end.is_none_or(|(current, _)| value > current) {
                end = Some((value, j));
            }
        }
    }
    let (score, mut j) = end?;

    let mut positions = vec![0; needles.len()];
    for k in (0..needles.len()).rev() {
        positions[k] = j;
        j = back[k][j];
    }
    Some(FuzzyMatch { score, positions })
}

/// The score [`fuzzy_term_match`] would return, computed with two rolling DP
/// rows and no back-pointers or positions, for ranking fields that are not
/// drawn.
pub(crate) fn fuzzy_term_score(term: &str, text: &str) -> Option<i32> {
    let len = subsequence_text_len(term, text)?;
    let mut needles = term.chars().map(fold);
    let Some(first) = needles.next() else {
        return Some(0);
    };
    let mut previous: Vec<Option<i32>> = vec![None; len];
    let mut row: Vec<Option<i32>> = vec![None; len];
    let mut prev_char = None;
    for (j, ch) in text.chars().map(fold).enumerate() {
        previous[j] = (ch == first).then(|| char_bonus(j, prev_char) - GAP_PENALTY * j as i32);
        prev_char = Some(ch);
    }
    for needle in needles {
        row.fill(None);
        let mut running: Option<i32> = None;
        let mut prev_char = None;
        for (j, ch) in text.chars().map(fold).enumerate() {
            if j >= 2 {
                let i = j - 2;
                if let Some(value) = previous[i] {
                    let candidate = value + GAP_PENALTY * i as i32;
                    if running.is_none_or(|current| candidate > current) {
                        running = Some(candidate);
                    }
                }
            }
            if j >= 1 && ch == needle {
                let consecutive = previous[j - 1].map(|value| value + CONSECUTIVE_BONUS);
                let gapped = running.map(|value| value - GAP_PENALTY * (j - 1) as i32);
                row[j] = consecutive
                    .max(gapped)
                    .map(|value| char_bonus(j, prev_char) + value);
            }
            prev_char = Some(ch);
        }
        std::mem::swap(&mut previous, &mut row);
    }
    previous.into_iter().flatten().max()
}

fn is_subsequence(term: &str, text: &str) -> bool {
    subsequence_text_len(term, text).is_some()
}

/// Checks that the folded `term` is a subsequence of the folded `text` without
/// allocating, returning the char length of `text` when it is.
fn subsequence_text_len(term: &str, text: &str) -> Option<usize> {
    let mut needles = term.chars().map(fold).peekable();
    let mut len = 0;
    for ch in text.chars().map(fold) {
        if needles.peek() == Some(&ch) {
            needles.next();
        }
        len += 1;
    }
    needles.peek().is_none().then_some(len)
}

fn char_bonus(j: usize, previous: Option<char>) -> i32 {
    if j == 0 {
        PREFIX_BONUS
    } else if previous.is_none_or(is_separator) {
        BOUNDARY_BONUS
    } else {
        0
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '/' | '-' | '_' | '.' | ':')
}

/// A match starts a word when the character before it is a separator.
fn is_boundary(haystack: &[char], idx: usize) -> bool {
    let Some(previous) = idx.checked_sub(1).and_then(|i| haystack.get(i)) else {
        return true;
    };
    is_separator(*previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_subsequence() {
        assert!(fuzzy_score("cld", "claude").is_some());
    }

    #[test]
    fn rejects_a_non_subsequence() {
        assert_eq!(fuzzy_score("cldx", "claude"), None);
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(fuzzy_score("", "anything").is_some());
    }

    #[test]
    fn matches_terms_independently_in_any_order() {
        assert!(fuzzy_score("gamma alpha", "alpha beta gamma").is_some());
        assert!(fuzzy_score("  ALP\tGAM  ", "alpha beta gamma").is_some());
        assert_eq!(fuzzy_score("alpha missing", "alpha beta gamma"), None);
    }

    #[test]
    fn ignores_case() {
        assert!(fuzzy_score("CLD", "claude").is_some());
    }

    #[test]
    fn consecutive_runs_beat_scattered_matches() {
        let consecutive = fuzzy_score("cla", "claude").expect("match");
        let scattered = fuzzy_score("cla", "cxlxaude").expect("match");

        assert!(
            consecutive > scattered,
            "consecutive {consecutive} should beat scattered {scattered}"
        );
    }

    #[test]
    fn word_boundary_matches_beat_mid_word_matches() {
        let boundary = fuzzy_score("hs", "herdr src").expect("match");
        let mid_word = fuzzy_score("hs", "theshed").expect("match");

        assert!(
            boundary > mid_word,
            "boundary {boundary} should beat mid-word {mid_word}"
        );
    }

    #[test]
    fn a_prefix_match_beats_a_later_match() {
        let prefix = fuzzy_score("cl", "claude").expect("match");
        let later = fuzzy_score("cl", "xxclaude").expect("match");

        assert!(prefix > later, "prefix {prefix} should beat later {later}");
    }

    #[test]
    fn optimal_placement_beats_the_first_occurrence() {
        let found = fuzzy_term_match("ab", "xaxb ab").expect("match");
        assert_eq!(found.positions, vec![5, 6]);
        assert_eq!(found.score, 15);
    }

    #[test]
    fn positions_are_char_indices_in_the_original_text() {
        let found = fuzzy_term_match("fi", "naïve fix").expect("match");
        assert_eq!(found.positions, vec![6, 7]);
    }

    #[test]
    fn a_char_with_a_multi_char_lowercase_does_not_shift_positions() {
        assert_eq!(
            fuzzy_term_match("ab", "İx ab").expect("match").positions,
            vec![3, 4]
        );
        assert_eq!(fuzzy_term_match("i", "İ"), None);
    }

    #[test]
    fn the_score_only_matcher_agrees_with_the_positions_matcher() {
        let cases = [
            ("cld", "claude"),
            ("cldx", "claude"),
            ("", "anything"),
            ("alp", "alpha beta gamma"),
            ("gam", "alpha beta gamma"),
            ("missing", "alpha beta gamma"),
            ("CLD", "claude"),
            ("cla", "claude"),
            ("cla", "cxlxaude"),
            ("hs", "herdr src"),
            ("hs", "theshed"),
            ("cl", "claude"),
            ("cl", "xxclaude"),
            ("ab", "xaxb ab"),
            ("fi", "naïve fix"),
            ("ab", "İx ab"),
            ("i", "İ"),
            ("ab", "axxab"),
            ("worker", "wxxoxxrxxkxxexxr"),
            ("toolong", "short"),
            ("abc", "a-b-c xabc abc"),
            ("gap", "Gap between panes"),
        ];
        for (term, text) in cases {
            assert_eq!(
                fuzzy_term_score(term, text),
                fuzzy_term_match(term, text).map(|found| found.score),
                "{term:?} against {text:?}"
            );
        }
    }

    #[test]
    fn a_gapped_prefix_placement_beats_a_later_consecutive_run() {
        let found = fuzzy_term_match("ab", "axxab").expect("match");
        assert_eq!(found.positions, vec![0, 4]);
        assert_eq!(found.score, 13);
    }
}
