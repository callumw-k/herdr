const CONSECUTIVE_BONUS: i32 = 8;
const BOUNDARY_BONUS: i32 = 12;
const PREFIX_BONUS: i32 = 16;
const GAP_PENALTY: i32 = 1;

/// Greedy left-to-right subsequence match. Returns `None` when `query` is not
/// a subsequence of `text`, otherwise a score where higher is better.
///
/// Greedy rather than optimal: an optimal matcher would backtrack to find the
/// best alignment, which costs O(n*m) for a picker that never sees more than a
/// few hundred rows. If ranking ever feels wrong on real data, that is the
/// upgrade path.
pub(crate) fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let needles: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();

    let mut score = 0;
    let mut haystack_idx = 0usize;
    let mut previous_match: Option<usize> = None;

    for needle in needles {
        let found = haystack[haystack_idx..]
            .iter()
            .position(|candidate| *candidate == needle)
            .map(|offset| haystack_idx + offset)?;

        if previous_match == Some(found.wrapping_sub(1)) {
            score += CONSECUTIVE_BONUS;
        }
        if found == 0 {
            score += PREFIX_BONUS;
        } else if is_boundary(&haystack, found) {
            score += BOUNDARY_BONUS;
        }
        if let Some(previous) = previous_match {
            score -= GAP_PENALTY * (found - previous - 1) as i32;
        } else {
            score -= GAP_PENALTY * found as i32;
        }

        previous_match = Some(found);
        haystack_idx = found + 1;
    }

    Some(score)
}

/// A match starts a word when the character before it is a separator.
fn is_boundary(haystack: &[char], idx: usize) -> bool {
    let Some(previous) = idx.checked_sub(1).and_then(|i| haystack.get(i)) else {
        return true;
    };
    matches!(previous, ' ' | '/' | '-' | '_' | '.' | ':')
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
}
