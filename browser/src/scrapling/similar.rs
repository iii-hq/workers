//! `find_similar` — parser.py's structural auto-match (`Selector.find_similar`
//! together with `__are_alike`) — and the `ratio`/`round2` numeric primitives
//! it's built on (difflib `SequenceMatcher.ratio()` + CPython `round(x, 2)`).

use std::collections::HashMap;

use crate::scrapling::{dom, dom::ElementRef, text};

/// `find_similar`'s `ignore_attributes` default — fixed here, not exposed as
/// a parameter (ledger: attrs minus href/src, "not exposed").
const IGNORE_ATTRS: [&str; 2] = ["href", "src"];

/// A matching block in the same coordinate order as Python's
/// `difflib.Match(a, b, size)`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Match {
    first_start: usize,
    second_start: usize,
    size: usize,
}

impl Match {
    fn new(first_start: usize, second_start: usize, size: usize) -> Self {
        Self {
            first_start,
            second_start,
            size,
        }
    }
}

/// The character-sequence subset of CPython 3.12's `SequenceMatcher` used by
/// Scrapling. There is no junk predicate in the wrapper call, but the default
/// autojunk heuristic is observable for inputs of 200 characters or more.
struct SequenceMatcher<'a> {
    first: &'a [char],
    second: &'a [char],
    second_indices: HashMap<char, Vec<usize>>,
}

impl<'a> SequenceMatcher<'a> {
    fn new(first: &'a [char], second: &'a [char]) -> Self {
        Self::with_autojunk(first, second, true)
    }

    fn with_autojunk(first: &'a [char], second: &'a [char], autojunk: bool) -> Self {
        let mut second_indices: HashMap<char, Vec<usize>> = HashMap::new();
        for (index, &value) in second.iter().enumerate() {
            second_indices.entry(value).or_default().push(index);
        }

        // CPython's heuristic removes values occurring more than 1% + 1
        // times, but only once the second sequence reaches 200 items.
        if autojunk && second.len() >= 200 {
            let popularity_cutoff = second.len() / 100 + 1;
            second_indices.retain(|_, indices| indices.len() <= popularity_cutoff);
        }

        Self {
            first,
            second,
            second_indices,
        }
    }

    fn find_longest_match(
        &self,
        first_start: usize,
        first_end: usize,
        second_start: usize,
        second_end: usize,
    ) -> Match {
        let (mut best_first, mut best_second, mut best_size) = (first_start, second_start, 0usize);
        let mut previous_lengths = HashMap::<usize, usize>::new();

        for first_index in first_start..first_end {
            let mut current_lengths = HashMap::<usize, usize>::new();
            if let Some(second_positions) = self.second_indices.get(&self.first[first_index]) {
                for &second_index in second_positions {
                    if second_index < second_start {
                        continue;
                    }
                    if second_index >= second_end {
                        break;
                    }

                    let size = second_index
                        .checked_sub(1)
                        .and_then(|previous| previous_lengths.get(&previous).copied())
                        .unwrap_or(0)
                        + 1;
                    current_lengths.insert(second_index, size);
                    // Strictly greater preserves CPython's earliest-first tie
                    // breaking as both sequences are scanned in order.
                    if size > best_size {
                        best_first = first_index + 1 - size;
                        best_second = second_index + 1 - size;
                        best_size = size;
                    }
                }
            }
            previous_lengths = current_lengths;
        }

        // Popular elements are absent from second_indices but are not junk,
        // so CPython extends a seeded match across them in either direction.
        while best_first > first_start
            && best_second > second_start
            && self.first[best_first - 1] == self.second[best_second - 1]
        {
            best_first -= 1;
            best_second -= 1;
            best_size += 1;
        }
        while best_first + best_size < first_end
            && best_second + best_size < second_end
            && self.first[best_first + best_size] == self.second[best_second + best_size]
        {
            best_size += 1;
        }

        Match::new(best_first, best_second, best_size)
    }

    fn get_matching_blocks(&self) -> Vec<Match> {
        let mut queue = vec![(0, self.first.len(), 0, self.second.len())];
        let mut blocks = Vec::new();

        while let Some((first_start, first_end, second_start, second_end)) = queue.pop() {
            let matched = self.find_longest_match(first_start, first_end, second_start, second_end);
            if matched.size == 0 {
                continue;
            }

            if first_start < matched.first_start && second_start < matched.second_start {
                queue.push((
                    first_start,
                    matched.first_start,
                    second_start,
                    matched.second_start,
                ));
            }
            if matched.first_start + matched.size < first_end
                && matched.second_start + matched.size < second_end
            {
                queue.push((
                    matched.first_start + matched.size,
                    first_end,
                    matched.second_start + matched.size,
                    second_end,
                ));
            }
            blocks.push(matched);
        }

        blocks.sort();

        // Collapse adjacent blocks exactly as CPython does, then append its
        // terminal zero-sized sentinel.
        let mut collapsed = Vec::with_capacity(blocks.len() + 1);
        let (mut first_start, mut second_start, mut size) = (0, 0, 0);
        for block in blocks {
            if first_start + size == block.first_start && second_start + size == block.second_start
            {
                size += block.size;
            } else {
                if size != 0 {
                    collapsed.push(Match::new(first_start, second_start, size));
                }
                first_start = block.first_start;
                second_start = block.second_start;
                size = block.size;
            }
        }
        if size != 0 {
            collapsed.push(Match::new(first_start, second_start, size));
        }
        collapsed.push(Match::new(self.first.len(), self.second.len(), 0));
        collapsed
    }

    fn ratio(&self) -> f64 {
        let total = self.first.len() + self.second.len();
        if total == 0 {
            return 1.0;
        }
        let matches: usize = self
            .get_matching_blocks()
            .into_iter()
            .map(|block| block.size)
            .sum();
        2.0 * matches as f64 / total as f64
    }
}

/// `difflib.SequenceMatcher(None, a, b).ratio()` over chars, computed directly
/// in f64 as `2.0 * matches / (len_a + len_b)`.
pub fn ratio(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    SequenceMatcher::new(&a_chars, &b_chars).ratio()
}

/// CPython `round(x, 2)`. Rust's `{:.2}` formatter and CPython's `round` are
/// both correctly-rounded half-even decimal conversions of the same
/// underlying binary64 value, so this agrees with the oracle bit-for-bit.
pub fn round2(x: f64) -> f64 {
    format!("{:.2}", x).parse().unwrap()
}

/// Number of ancestor elements (lxml `len(list(el.iterancestors()))`).
fn depth(el: ElementRef) -> usize {
    let mut n = 0;
    let mut cur = el;
    while let Some(parent) = dom::parent_element(cur) {
        n += 1;
        cur = parent;
    }
    n
}

/// Attrs minus the fixed ignore list, source order preserved — order matters
/// here because `are_alike` sums `ratio()` calls over these in iteration
/// order, and that sum must match Python's dict-iteration order bit-for-bit.
fn filtered_attrs<'a>(el: ElementRef<'a>) -> Vec<(&'a str, &'a str)> {
    el.attrs()
        .filter(|(name, _)| !IGNORE_ATTRS.contains(name))
        .collect()
}

/// Same tag + parent-tag + grandparent-tag chain as the anchor's precomputed
/// `parent_tag`/`grandparent_tag` (parser.py builds `//{grandparent}/{parent}/{tag}`
/// and evaluates it as an absolute, whole-document XPath; walked directly
/// here instead of through the xpath engine, per the ledger).
fn chain_matches(el: ElementRef, parent_tag: Option<&str>, grandparent_tag: Option<&str>) -> bool {
    let Some(want_parent) = parent_tag else {
        return true;
    };
    let Some(parent) = dom::parent_element(el) else {
        return false;
    };
    if parent.name() != want_parent {
        return false;
    }
    let Some(want_grandparent) = grandparent_tag else {
        return true;
    };
    dom::parent_element(parent).is_some_and(|gp| gp.name() == want_grandparent)
}

/// `__are_alike`: accept iff `checks > 0 && round2(score / checks) >= threshold`.
fn are_alike(
    anchor: ElementRef,
    target_attrs: &[(&str, &str)],
    candidate: ElementRef,
    threshold: f64,
    match_text: bool,
) -> bool {
    let candidate_attrs = filtered_attrs(candidate);
    let mut score = 0.0_f64;
    let mut checks = 0usize;

    if !target_attrs.is_empty() {
        for &(k, v) in target_attrs {
            let cv = candidate_attrs
                .iter()
                .copied()
                .find(|&(ck, _)| ck == k)
                .map_or("", |(_, cv)| cv);
            score += ratio(v, cv);
        }
        // `max`, not `min`: extra candidate attrs are penalized, and fewer
        // candidate attrs don't inflate the score via a smaller denominator.
        checks += target_attrs.len().max(candidate_attrs.len());
    } else if candidate_attrs.is_empty() {
        // Both attribute-free: "this must mean something" (parser.py comment).
        score += 1.0;
        checks += 1;
    }

    if match_text {
        // `__are_alike` uses `clean_spaces` (core/utils/_utils.py), NOT
        // `TextHandler.clean` — a different cleaner: `\n`/`\r` are deleted
        // outright (no space inserted) and the result is never trimmed.
        let a_text = text::clean_spaces(&dom::leading_text(anchor));
        let b_text = text::clean_spaces(&dom::leading_text(candidate));
        score += ratio(&a_text, &b_text);
        checks += 1;
    }

    checks > 0 && round2(score / checks as f64) >= threshold
}

/// Elements at the anchor's own ancestor depth, sharing its tag/parent-tag/
/// grandparent-tag chain, scored alike by `__are_alike` — document order,
/// anchor excluded, no cap.
pub fn find_similar<'a>(
    doc: &'a dom::Doc,
    anchor: ElementRef<'a>,
    threshold: f64,
    match_text: bool,
) -> Vec<ElementRef<'a>> {
    let anchor_depth = depth(anchor);
    let anchor_tag = anchor.name();
    let anchor_parent = dom::parent_element(anchor);
    let parent_tag = anchor_parent.map(ElementRef::name);
    let grandparent_tag = anchor_parent
        .and_then(dom::parent_element)
        .map(ElementRef::name);
    let target_attrs = filtered_attrs(anchor);

    dom::descendant_elements(doc.root())
        .into_iter()
        .filter(|&el| el.id() != anchor.id())
        // ponytail: depth() walk runs before the cheaper tag check; reorder
        // if find-similar ever profiles hot.
        .filter(|&el| depth(el) == anchor_depth)
        .filter(|&el| el.name() == anchor_tag)
        .filter(|&el| chain_matches(el, parent_tag, grandparent_tag))
        .filter(|&el| are_alike(anchor, &target_attrs, el, threshold, match_text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_matches_cpython_examples() {
        // difflib.SequenceMatcher(None,'card wide','card slim').ratio() == 0.6666666666666666
        assert_eq!(ratio("card wide", "card slim"), 0.666_666_666_666_666_6);
        assert_eq!(ratio("", ""), 1.0);
        assert_eq!(ratio("abc", "abc"), 1.0);
    }

    #[test]
    fn ratio_ignores_popular_elements_at_cpython_autojunk_threshold() {
        // Frozen CPython 3.12.13: with len(b) == 200, the 199 `x` values
        // are popular and removed from b2j. Since the leading values differ,
        // there is no non-popular match from which the run can be extended.
        let a = "x".repeat(199);
        let b = format!("y{}", "x".repeat(199));
        assert_eq!(ratio(&a, &b), 0.0);
    }

    #[test]
    fn autojunk_stays_disabled_below_two_hundred_items() {
        // Frozen CPython 3.12.13: len(b) == 199 does not activate autojunk.
        let a = "x".repeat(198);
        let b = format!("y{}", "x".repeat(198));
        assert_eq!(ratio(&a, &b), 0.997_481_108_312_342_6);
    }

    #[test]
    fn matching_blocks_follow_cpython_order_and_include_terminal_sentinel() {
        // Frozen CPython 3.12.13:
        // SequenceMatcher(None, "abxcd", "abcd").get_matching_blocks()
        let a: Vec<char> = "abxcd".chars().collect();
        let b: Vec<char> = "abcd".chars().collect();
        let matcher = SequenceMatcher::new(&a, &b);
        let blocks: Vec<_> = matcher
            .get_matching_blocks()
            .iter()
            .map(|m| (m.first_start, m.second_start, m.size))
            .collect();
        assert_eq!(blocks, [(0, 0, 2), (3, 2, 2), (5, 4, 0)]);
    }

    #[test]
    fn equal_length_ties_prefer_the_earliest_first_sequence_position() {
        // Frozen CPython 3.12.13: the competing one-character matches are
        // resolved to a[0] before considering the earlier position in b.
        let a: Vec<char> = "ab".chars().collect();
        let b: Vec<char> = "ba".chars().collect();
        let matcher = SequenceMatcher::new(&a, &b);
        assert_eq!(
            matcher.get_matching_blocks(),
            [Match::new(0, 1, 1), Match::new(2, 2, 0)]
        );
    }

    #[test]
    fn popular_elements_extend_a_match_seeded_by_a_rare_element() {
        // Frozen CPython 3.12.13: `x` is popular, but it remains eligible for
        // extension around the non-popular `z` seed.
        let value = format!("{}z{}", "x".repeat(100), "x".repeat(100));
        let chars: Vec<char> = value.chars().collect();
        let matcher = SequenceMatcher::new(&chars, &chars);
        assert_eq!(
            matcher.get_matching_blocks(),
            [Match::new(0, 0, 201), Match::new(201, 201, 0)]
        );
        assert_eq!(matcher.ratio(), 1.0);
    }

    #[test]
    fn disabling_autojunk_retains_repeated_matches() {
        // Frozen CPython 3.12.13 with autojunk=False.
        let a: Vec<char> = "x".repeat(199).chars().collect();
        let b: Vec<char> = format!("y{}", "x".repeat(199)).chars().collect();
        let matcher = SequenceMatcher::with_autojunk(&a, &b, false);
        let blocks: Vec<_> = matcher
            .get_matching_blocks()
            .iter()
            .map(|m| (m.first_start, m.second_start, m.size))
            .collect();
        assert_eq!(blocks, [(0, 1, 199), (199, 200, 0)]);
        assert_eq!(matcher.ratio(), 0.997_493_734_335_839_5);
    }

    #[test]
    fn round2_is_half_even_decimal() {
        assert_eq!(round2(0.665), 0.67); // 0.665 is 0.66500000000000003xx in binary
        assert_eq!(round2(0.5), 0.5);
        assert_eq!(round2(2.0 / 3.0), 0.67);
        assert_eq!(round2(0.625), 0.62);
        assert_eq!(round2(0.635), 0.64);
    }
}
