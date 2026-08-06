//! Fuzzy path matching for `editor::find`.
//!
//! Path-aware rather than generic: a query is matched as a subsequence, but
//! the score is dominated by *where* the characters landed. Matching the
//! basename beats matching a directory, matching after a separator beats
//! matching mid-word, and runs of adjacent characters beat scattered hits.
//! Without that bias, `mod` in a large repo ranks a hundred `src/models/…`
//! directories above the `mod.rs` you meant.
//!
//! Matching is case-insensitive, with a bonus when the case matched exactly —
//! so `App` prefers `App.tsx` over `app.config.js` without ever hiding it.

/// One scored candidate. `positions` are byte indices into the haystack, in
/// order, so a UI can highlight exactly the characters that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

const BONUS_CONSECUTIVE: i32 = 8;
const BONUS_SEPARATOR: i32 = 10;
const BONUS_BASENAME: i32 = 12;
const BONUS_EXACT_CASE: i32 = 2;
const PENALTY_LEADING: i32 = -1;
const PENALTY_LEADING_MAX: i32 = -12;
const PENALTY_UNMATCHED_TAIL: i32 = -1;

fn is_separator(c: char) -> bool {
    matches!(c, '/' | '\\' | '_' | '-' | '.' | ' ')
}

/// Score `query` against `haystack`, or `None` when `query` is not a
/// subsequence of it.
///
/// Greedy left-to-right: the first viable match for each query character is
/// taken. That is not the globally optimal alignment, but it is linear and the
/// separator/basename bonuses recover the cases an optimal matcher would win —
/// worth it when the candidate list is every tracked file in a repo.
pub fn score(query: &str, haystack: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }

    let basename_start = haystack.rfind('/').map(|i| i + 1).unwrap_or(0);

    let hay: Vec<(usize, char)> = haystack.char_indices().collect();
    let mut positions = Vec::with_capacity(query.chars().count());
    let mut total = 0i32;
    let mut hay_idx = 0usize;
    let mut last_match: Option<usize> = None;

    for qc in query.chars() {
        let qc_lower = qc.to_ascii_lowercase();
        let mut found = None;

        while hay_idx < hay.len() {
            let (byte_idx, hc) = hay[hay_idx];
            if hc.to_ascii_lowercase() == qc_lower {
                found = Some((hay_idx, byte_idx, hc));
                break;
            }
            hay_idx += 1;
        }

        let (idx, byte_idx, hc) = found?;

        let mut points = 1;
        if hc == qc {
            points += BONUS_EXACT_CASE;
        }
        if byte_idx >= basename_start {
            points += BONUS_BASENAME;
        }
        if last_match == Some(idx.wrapping_sub(1)) {
            points += BONUS_CONSECUTIVE;
        } else if idx == 0 || hay.get(idx - 1).is_some_and(|(_, p)| is_separator(*p)) {
            points += BONUS_SEPARATOR;
        }

        total += points;
        positions.push(byte_idx);
        last_match = Some(idx);
        hay_idx = idx + 1;
    }

    // Characters skipped before the first match, and everything trailing the
    // last one, both make the match less about this path. Bounded so a long
    // path is not disqualified outright.
    let leading = positions.first().copied().unwrap_or(0) as i32;
    total += (leading * PENALTY_LEADING).max(PENALTY_LEADING_MAX);
    let trailing = haystack
        .len()
        .saturating_sub(positions.last().copied().unwrap_or(0)) as i32;
    total += (trailing * PENALTY_UNMATCHED_TAIL).max(PENALTY_LEADING_MAX);

    Some(Match {
        score: total,
        positions,
    })
}

/// Rank `candidates` by [`score`], best first, keeping at most `limit`.
///
/// Ties break on the shorter path, then alphabetically, so the same query
/// always produces the same ordering — a picker that reshuffles equal-scoring
/// rows between keystrokes is unusable.
pub fn rank<'a>(query: &str, candidates: &[&'a str], limit: usize) -> Vec<(&'a str, Match)> {
    let mut scored: Vec<(&str, Match)> = candidates
        .iter()
        .filter_map(|c| score(query, c).map(|m| (*c, m)))
        .collect();

    scored.sort_by(|a, b| {
        b.1.score
            .cmp(&a.1.score)
            .then_with(|| a.0.len().cmp(&b.0.len()))
            .then_with(|| a.0.cmp(b.0))
    });
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_subsequence_does_not_match() {
        assert!(score("zzz", "src/main.rs").is_none());
    }

    #[test]
    fn empty_query_matches_everything_at_zero() {
        let m = score("", "src/main.rs").expect("empty query matches");
        assert_eq!(m.score, 0);
        assert!(m.positions.is_empty());
    }

    #[test]
    fn positions_point_at_the_matched_characters() {
        let m = score("mn", "src/main.rs").expect("subsequence");
        let chars: String = m
            .positions
            .iter()
            .map(|&i| "src/main.rs"[i..].chars().next().unwrap())
            .collect();
        assert_eq!(chars, "mn");
    }

    #[test]
    fn basename_beats_directory() {
        let ranked = rank("mod", &["src/models/user.rs", "src/tree/mod.rs"], 10);
        assert_eq!(ranked[0].0, "src/tree/mod.rs");
    }

    #[test]
    fn exact_case_outranks_folded_case() {
        let ranked = rank("App", &["src/app.config.js", "src/App.tsx"], 10);
        assert_eq!(ranked[0].0, "src/App.tsx");
    }

    #[test]
    fn consecutive_run_beats_scattered_hits() {
        let ranked = rank("editor", &["e/d/i/t/o/r.rs", "src/editor.rs"], 10);
        assert_eq!(ranked[0].0, "src/editor.rs");
    }

    /// Ranking the same slice twice proves nothing — `rank` is pure, so the
    /// two runs cannot differ. What is worth pinning is the documented
    /// tie-break itself: shorter path first, then alphabetically, and never
    /// dependent on the order the candidates arrived in.
    #[test]
    fn equal_scores_break_on_length_then_alphabetically() {
        // An empty query scores every candidate 0, which is the only way to get
        // an exact tie across paths of different lengths.
        let ordered: Vec<&str> = rank("", &["bbb.rs", "cc.rs", "a.rs"], 10)
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(ordered, vec!["a.rs", "cc.rs", "bbb.rs"]);

        let shuffled: Vec<&str> = rank("", &["a.rs", "bbb.rs", "cc.rs"], 10)
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(
            ordered, shuffled,
            "a picker that reshuffles equal rows by input order is unusable"
        );
    }

    #[test]
    fn equal_length_ties_break_alphabetically() {
        let ranked = rank("x", &["b/x.rs", "a/x.rs"], 10);
        assert_eq!(
            ranked[0].1.score, ranked[1].1.score,
            "the fixture only exercises the tie-break if it is actually a tie"
        );
        assert_eq!(
            ranked.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec!["a/x.rs", "b/x.rs"]
        );
    }

    #[test]
    fn a_candidate_that_does_not_match_is_dropped_from_the_ranking() {
        let ranked = rank("mod", &["src/mod.rs", "README"], 10);
        assert_eq!(
            ranked.len(),
            1,
            "a non-subsequence must not be ranked at all"
        );
        assert_eq!(ranked[0].0, "src/mod.rs");
    }

    /// `positions` are byte offsets a UI slices the path with. An index that is
    /// not a char boundary panics the consumer, so a path with multibyte
    /// characters has to come back with boundaries.
    #[test]
    fn positions_are_char_boundaries_on_a_multibyte_path() {
        let path = "src/café/main.rs";
        let m = score("cm", path).expect("subsequence");
        for &i in &m.positions {
            assert!(path.is_char_boundary(i), "byte {i} splits a codepoint");
        }
        let matched: String = m
            .positions
            .iter()
            .map(|&i| path[i..].chars().next().expect("a character"))
            .collect();
        assert_eq!(matched, "cm");
    }

    #[test]
    fn limit_is_respected() {
        let candidates = ["a.rs", "ab.rs", "abc.rs", "abcd.rs"];
        assert_eq!(rank("a", &candidates, 2).len(), 2);
    }

    #[test]
    fn a_limit_past_the_end_of_the_list_is_not_an_error() {
        assert_eq!(rank("a", &["a.rs"], 100).len(), 1);
    }
}
