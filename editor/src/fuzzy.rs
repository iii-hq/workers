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

    #[test]
    fn ranking_is_stable_for_equal_scores() {
        let candidates = ["b/x.rs", "a/x.rs"];
        let first = rank("x", &candidates, 10);
        let second = rank("x", &candidates, 10);
        assert_eq!(
            first.iter().map(|r| r.0).collect::<Vec<_>>(),
            second.iter().map(|r| r.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn limit_is_respected() {
        let candidates = ["a.rs", "ab.rs", "abc.rs", "abcd.rs"];
        assert_eq!(rank("a", &candidates, 2).len(), 2);
    }
}
