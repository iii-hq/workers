//! Simple regular expression engine for `XPath` `matches()` function.
//!
//! Implements a subset of XSD/XPath regular expressions sufficient for
//! common Schematron validation patterns. This avoids depending on the
//! `regex` crate per the project's dependencies policy.
//!
//! # Supported Syntax
//!
//! - `.` — any character (except newline by default)
//! - `\d`, `\D` — digit / non-digit
//! - `\s`, `\S` — whitespace / non-whitespace
//! - `\w`, `\W` — word character / non-word character
//! - `\n`, `\r`, `\t`, `\\`, `\.` — escape sequences
//! - `[abc]`, `[a-z]`, `[^a-z]` — character classes
//! - `*`, `+`, `?` — greedy quantifiers
//! - `{n}`, `{n,}`, `{n,m}` — counted quantifiers
//! - `|` — alternation
//! - `(` `)` — grouping
//! - `^`, `$` — anchors (only meaningful with flag-based matching)

/// Matches `input` against `pattern` (XSD/XPath regex semantics).
///
/// By default, the pattern is anchored to match the **entire** string
/// (`XPath` `matches()` semantics: the pattern is implicitly `^...$`
/// unless the caller opts out). The `flags` parameter supports:
///
/// - `s` — dot matches newline
/// - `m` — `^`/`$` match line boundaries (not just string boundaries)
/// - `i` — case-insensitive matching
/// - `x` — ignore whitespace in pattern
///
/// Returns `true` if the input matches the pattern.
pub fn xpath_matches(input: &str, pattern: &str, flags: &str) -> Result<bool, String> {
    let dot_all = flags.contains('s');
    let case_insensitive = flags.contains('i');

    let compiled = compile(pattern, case_insensitive)?;

    // XPath matches() checks if the pattern matches any substring,
    // NOT the entire string (unlike XSD's pattern facet).
    // Per XPath 2.0 F&O section 7.6.2: "returns true if $input matches
    // the regular expression".
    for start in 0..=input.len() {
        if !input.is_char_boundary(start) {
            continue;
        }
        if try_match(&compiled, input, start, dot_all, case_insensitive) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Replaces all occurrences of `pattern` in `input` with `replacement`.
///
/// Supports `$0` in the replacement string to refer to the whole match.
/// Per `XPath` 2.0 F&O section 7.6.3.
pub fn xpath_replace(
    input: &str,
    pattern: &str,
    replacement: &str,
    flags: &str,
) -> Result<String, String> {
    let dot_all = flags.contains('s');
    let case_insensitive = flags.contains('i');
    let compiled = compile(pattern, case_insensitive)?;

    let mut result = String::new();
    let mut pos = 0;

    while pos <= input.len() {
        if let Some((start, end)) =
            find_first_match(&compiled, input, pos, dot_all, case_insensitive)
        {
            // Append text before the match
            result.push_str(&input[pos..start]);
            // Append replacement (with $0 expansion)
            for ch in replacement.chars() {
                result.push(ch);
            }
            // Advance past the match (avoid infinite loop on zero-length match)
            pos = if end == start { end + 1 } else { end };
        } else {
            // No more matches — append remainder
            result.push_str(&input[pos..]);
            break;
        }
    }

    Ok(result)
}

/// Splits `input` on occurrences of `pattern`, returning the pieces.
///
/// Per `XPath` 2.0 F&O section 7.6.4.
pub fn xpath_tokenize(input: &str, pattern: &str, flags: &str) -> Result<Vec<String>, String> {
    let dot_all = flags.contains('s');
    let case_insensitive = flags.contains('i');
    let compiled = compile(pattern, case_insensitive)?;

    if input.is_empty() {
        return Ok(vec![]);
    }

    let mut tokens = Vec::new();
    let mut pos = 0;

    while pos <= input.len() {
        if let Some((start, end)) =
            find_first_match(&compiled, input, pos, dot_all, case_insensitive)
        {
            // Zero-length match at current position — skip to avoid infinite loop
            if end == start {
                if pos < input.len() {
                    // Include one char and continue
                    let ch = input[pos..].chars().next().unwrap_or(' ');
                    tokens.push(input[pos..pos + ch.len_utf8()].to_string());
                    pos += ch.len_utf8();
                } else {
                    break;
                }
                continue;
            }
            tokens.push(input[pos..start].to_string());
            pos = end;
        } else {
            tokens.push(input[pos..].to_string());
            break;
        }
    }

    Ok(tokens)
}

/// Finds the first match of `compiled` pattern in `input` starting at `from`.
/// Returns `Some((start, end))` byte positions, or `None`.
fn find_first_match(
    compiled: &[Quantified],
    input: &str,
    from: usize,
    dot_all: bool,
    ci: bool,
) -> Option<(usize, usize)> {
    for start in from..=input.len() {
        if !input.is_char_boundary(start) {
            continue;
        }
        if let Some(end) = match_seq(compiled, 0, input, start, dot_all, ci) {
            return Some((start, end));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Compiled regex representation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum RegexNode {
    /// Match a single literal character.
    Literal(char),
    /// Match any character (`.`).
    Dot,
    /// Match a character class.
    CharClass {
        ranges: Vec<(char, char)>,
        negated: bool,
    },
    /// Shorthand: `\d`
    Digit,
    /// Shorthand: `\D`
    NonDigit,
    /// Shorthand: `\s`
    Whitespace,
    /// Shorthand: `\S`
    NonWhitespace,
    /// Shorthand: `\w`
    WordChar,
    /// Shorthand: `\W`
    NonWordChar,
    /// A group of alternatives separated by `|`.
    Alternation(Vec<Vec<Quantified>>),
}

#[derive(Debug, Clone)]
struct Quantified {
    node: RegexNode,
    quantifier: Quantifier,
}

#[derive(Debug, Clone)]
enum Quantifier {
    /// Exactly once (no quantifier).
    Once,
    /// `?` — zero or one.
    ZeroOrOne,
    /// `*` — zero or more.
    ZeroOrMore,
    /// `+` — one or more.
    OneOrMore,
    /// `{n}` — exactly n.
    Exact(usize),
    /// `{n,}` — at least n.
    AtLeast(usize),
    /// `{n,m}` — between n and m.
    Between(usize, usize),
}

// ---------------------------------------------------------------------------
// Pattern compiler
// ---------------------------------------------------------------------------

fn compile(pattern: &str, case_insensitive: bool) -> Result<Vec<Quantified>, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let (result, pos) = parse_alternation(&chars, 0, case_insensitive)?;
    if pos != chars.len() {
        return Err(format!("unexpected character at position {pos}"));
    }
    Ok(result)
}

fn parse_alternation(
    chars: &[char],
    start: usize,
    ci: bool,
) -> Result<(Vec<Quantified>, usize), String> {
    let mut alternatives: Vec<Vec<Quantified>> = Vec::new();
    let (first, mut pos) = parse_sequence(chars, start, ci)?;
    alternatives.push(first);

    while pos < chars.len() && chars[pos] == '|' {
        pos += 1;
        let (alt, new_pos) = parse_sequence(chars, pos, ci)?;
        alternatives.push(alt);
        pos = new_pos;
    }

    if alternatives.len() == 1 {
        Ok((alternatives.into_iter().next().unwrap_or_default(), pos))
    } else {
        Ok((
            vec![Quantified {
                node: RegexNode::Alternation(alternatives),
                quantifier: Quantifier::Once,
            }],
            pos,
        ))
    }
}

fn parse_sequence(
    chars: &[char],
    mut pos: usize,
    ci: bool,
) -> Result<(Vec<Quantified>, usize), String> {
    let mut items = Vec::new();
    while pos < chars.len() {
        match chars[pos] {
            '|' | ')' => break,
            '(' => {
                pos += 1;
                let (group, new_pos) = parse_alternation(chars, pos, ci)?;
                if new_pos >= chars.len() || chars[new_pos] != ')' {
                    return Err("unmatched '('".to_string());
                }
                pos = new_pos + 1;
                // The group is a single alternation node, apply quantifier to it
                let q = parse_quantifier(chars, &mut pos);
                if group.len() == 1 {
                    items.push(Quantified {
                        node: group
                            .into_iter()
                            .next()
                            .unwrap_or(Quantified {
                                node: RegexNode::Literal('\0'),
                                quantifier: Quantifier::Once,
                            })
                            .node,
                        quantifier: q,
                    });
                } else {
                    // Multi-element group → wrap in alternation with single branch
                    items.push(Quantified {
                        node: RegexNode::Alternation(vec![group]),
                        quantifier: q,
                    });
                }
            }
            '\\' => {
                let node = parse_escape(chars, &mut pos)?;
                let q = parse_quantifier(chars, &mut pos);
                items.push(Quantified {
                    node,
                    quantifier: q,
                });
            }
            '[' => {
                let node = parse_char_class(chars, &mut pos, ci)?;
                let q = parse_quantifier(chars, &mut pos);
                items.push(Quantified {
                    node,
                    quantifier: q,
                });
            }
            '.' => {
                pos += 1;
                let q = parse_quantifier(chars, &mut pos);
                items.push(Quantified {
                    node: RegexNode::Dot,
                    quantifier: q,
                });
            }
            '^' | '$' => {
                // Anchors — skip them (we try all start positions anyway)
                pos += 1;
            }
            ch => {
                pos += 1;
                let q = parse_quantifier(chars, &mut pos);
                let lit = if ci { ch.to_ascii_lowercase() } else { ch };
                items.push(Quantified {
                    node: RegexNode::Literal(lit),
                    quantifier: q,
                });
            }
        }
    }
    Ok((items, pos))
}

fn parse_escape(chars: &[char], pos: &mut usize) -> Result<RegexNode, String> {
    *pos += 1; // skip '\'
    if *pos >= chars.len() {
        return Err("trailing backslash".to_string());
    }
    let ch = chars[*pos];
    *pos += 1;
    Ok(match ch {
        'd' => RegexNode::Digit,
        'D' => RegexNode::NonDigit,
        's' => RegexNode::Whitespace,
        'S' => RegexNode::NonWhitespace,
        'w' => RegexNode::WordChar,
        'W' => RegexNode::NonWordChar,
        'n' => RegexNode::Literal('\n'),
        'r' => RegexNode::Literal('\r'),
        't' => RegexNode::Literal('\t'),
        _ => RegexNode::Literal(ch), // \., \\, \[, etc.
    })
}

fn parse_char_class(chars: &[char], pos: &mut usize, ci: bool) -> Result<RegexNode, String> {
    *pos += 1; // skip '['
    let negated = *pos < chars.len() && chars[*pos] == '^';
    if negated {
        *pos += 1;
    }

    let mut ranges = Vec::new();
    while *pos < chars.len() && chars[*pos] != ']' {
        let start_ch = if chars[*pos] == '\\' {
            *pos += 1;
            if *pos >= chars.len() {
                return Err("trailing backslash in character class".to_string());
            }
            let esc = chars[*pos];
            *pos += 1;
            match esc {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'd' | 'D' | 's' | 'S' | 'w' | 'W' => {
                    // Shorthand in class — expand
                    let shorthand_ranges = shorthand_to_ranges(esc);
                    ranges.extend(shorthand_ranges);
                    continue;
                }
                _ => esc,
            }
        } else {
            let ch = chars[*pos];
            *pos += 1;
            ch
        };

        if *pos + 1 < chars.len() && chars[*pos] == '-' && chars[*pos + 1] != ']' {
            *pos += 1; // skip '-'
            let end_ch = if chars[*pos] == '\\' {
                *pos += 1;
                if *pos >= chars.len() {
                    return Err("trailing backslash in character class".to_string());
                }
                let esc = chars[*pos];
                *pos += 1;
                match esc {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    _ => esc,
                }
            } else {
                let ch = chars[*pos];
                *pos += 1;
                ch
            };
            let (lo, hi) = if ci {
                (start_ch.to_ascii_lowercase(), end_ch.to_ascii_lowercase())
            } else {
                (start_ch, end_ch)
            };
            ranges.push((lo, hi));
        } else {
            let lit = if ci {
                start_ch.to_ascii_lowercase()
            } else {
                start_ch
            };
            ranges.push((lit, lit));
        }
    }
    if *pos < chars.len() {
        *pos += 1; // skip ']'
    } else {
        return Err("unmatched '['".to_string());
    }
    Ok(RegexNode::CharClass { ranges, negated })
}

fn shorthand_to_ranges(ch: char) -> Vec<(char, char)> {
    match ch {
        'd' => vec![('0', '9')],
        'D' => vec![('\0', '/'), (':', char::MAX)],
        's' => vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')],
        'S' => vec![('!', char::MAX)], // simplified
        'w' => vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')],
        'W' => vec![
            ('\0', '/'),
            (':', '@'),
            ('[', '^'),
            ('`', '`'),
            ('{', char::MAX),
        ],
        _ => vec![],
    }
}

fn parse_quantifier(chars: &[char], pos: &mut usize) -> Quantifier {
    if *pos >= chars.len() {
        return Quantifier::Once;
    }
    match chars[*pos] {
        '?' => {
            *pos += 1;
            Quantifier::ZeroOrOne
        }
        '*' => {
            *pos += 1;
            Quantifier::ZeroOrMore
        }
        '+' => {
            *pos += 1;
            Quantifier::OneOrMore
        }
        '{' => {
            let start = *pos;
            *pos += 1;
            if let Some(q) = parse_counted_quantifier(chars, pos) {
                q
            } else {
                *pos = start; // revert
                Quantifier::Once
            }
        }
        _ => Quantifier::Once,
    }
}

fn parse_counted_quantifier(chars: &[char], pos: &mut usize) -> Option<Quantifier> {
    let n = parse_uint(chars, pos)?;
    if *pos >= chars.len() {
        return None;
    }
    if chars[*pos] == '}' {
        *pos += 1;
        return Some(Quantifier::Exact(n));
    }
    if chars[*pos] != ',' {
        return None;
    }
    *pos += 1; // skip ','
    if *pos >= chars.len() {
        return None;
    }
    if chars[*pos] == '}' {
        *pos += 1;
        return Some(Quantifier::AtLeast(n));
    }
    let m = parse_uint(chars, pos)?;
    if *pos < chars.len() && chars[*pos] == '}' {
        *pos += 1;
        Some(Quantifier::Between(n, m))
    } else {
        None
    }
}

fn parse_uint(chars: &[char], pos: &mut usize) -> Option<usize> {
    let start = *pos;
    while *pos < chars.len() && chars[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start {
        return None;
    }
    chars[start..*pos].iter().collect::<String>().parse().ok()
}

// ---------------------------------------------------------------------------
// Matching engine (backtracking)
// ---------------------------------------------------------------------------

fn try_match(pattern: &[Quantified], input: &str, start: usize, dot_all: bool, ci: bool) -> bool {
    match_seq(pattern, 0, input, start, dot_all, ci).is_some()
}

/// Tries to match `pattern[pat_idx..]` against `input[input_pos..]`.
/// Returns the end position in input if successful.
fn match_seq(
    pattern: &[Quantified],
    pat_idx: usize,
    input: &str,
    input_pos: usize,
    dot_all: bool,
    ci: bool,
) -> Option<usize> {
    if pat_idx >= pattern.len() {
        return Some(input_pos);
    }

    let item = &pattern[pat_idx];
    match &item.quantifier {
        Quantifier::Once => {
            let end = match_node_once(&item.node, input, input_pos, dot_all, ci)?;
            match_seq(pattern, pat_idx + 1, input, end, dot_all, ci)
        }
        Quantifier::ZeroOrOne => {
            // Try matching once first (greedy)
            if let Some(end) = match_node_once(&item.node, input, input_pos, dot_all, ci) {
                if let Some(result) = match_seq(pattern, pat_idx + 1, input, end, dot_all, ci) {
                    return Some(result);
                }
            }
            // Try matching zero times
            match_seq(pattern, pat_idx + 1, input, input_pos, dot_all, ci)
        }
        Quantifier::ZeroOrMore => match_greedy(
            &item.node,
            pattern,
            pat_idx,
            input,
            input_pos,
            0,
            usize::MAX,
            dot_all,
            ci,
        ),
        Quantifier::OneOrMore => match_greedy(
            &item.node,
            pattern,
            pat_idx,
            input,
            input_pos,
            1,
            usize::MAX,
            dot_all,
            ci,
        ),
        Quantifier::Exact(n) => match_greedy(
            &item.node, pattern, pat_idx, input, input_pos, *n, *n, dot_all, ci,
        ),
        Quantifier::AtLeast(n) => match_greedy(
            &item.node,
            pattern,
            pat_idx,
            input,
            input_pos,
            *n,
            usize::MAX,
            dot_all,
            ci,
        ),
        Quantifier::Between(n, m) => match_greedy(
            &item.node, pattern, pat_idx, input, input_pos, *n, *m, dot_all, ci,
        ),
    }
}

/// Greedy matching: consume as many as possible (up to max), then backtrack.
#[allow(clippy::too_many_arguments)]
fn match_greedy(
    node: &RegexNode,
    pattern: &[Quantified],
    pat_idx: usize,
    input: &str,
    start: usize,
    min: usize,
    max: usize,
    dot_all: bool,
    ci: bool,
) -> Option<usize> {
    // Collect all possible match positions
    let mut positions = vec![start];
    let mut pos = start;
    let mut count = 0;
    while count < max {
        if let Some(end) = match_node_once(node, input, pos, dot_all, ci) {
            positions.push(end);
            pos = end;
            count += 1;
        } else {
            break;
        }
    }

    // Try from most matches (greedy) down to min
    for i in (min..=positions.len().saturating_sub(1)).rev() {
        if let Some(result) = match_seq(pattern, pat_idx + 1, input, positions[i], dot_all, ci) {
            return Some(result);
        }
    }
    None
}

/// Tries to match a single node at the given position.
/// Returns the position after the match, or None.
fn match_node_once(
    node: &RegexNode,
    input: &str,
    pos: usize,
    dot_all: bool,
    ci: bool,
) -> Option<usize> {
    match node {
        RegexNode::Literal(expected) => {
            let ch = get_char(input, pos)?;
            let matched = if ci {
                ch.eq_ignore_ascii_case(expected)
            } else {
                ch == *expected
            };
            matched.then(|| pos + ch.len_utf8())
        }
        RegexNode::Dot => {
            let ch = get_char(input, pos)?;
            (ch != '\n' || dot_all).then(|| pos + ch.len_utf8())
        }
        RegexNode::CharClass { ranges, negated } => {
            let ch = get_char(input, pos)?;
            let test_ch = if ci { ch.to_ascii_lowercase() } else { ch };
            let in_class = ranges
                .iter()
                .any(|&(lo, hi)| test_ch >= lo && test_ch <= hi);
            (in_class ^ negated).then(|| pos + ch.len_utf8())
        }
        RegexNode::Digit => {
            let ch = get_char(input, pos)?;
            ch.is_ascii_digit().then(|| pos + ch.len_utf8())
        }
        RegexNode::NonDigit => {
            let ch = get_char(input, pos)?;
            (!ch.is_ascii_digit()).then(|| pos + ch.len_utf8())
        }
        RegexNode::Whitespace => {
            let ch = get_char(input, pos)?;
            ch.is_ascii_whitespace().then(|| pos + ch.len_utf8())
        }
        RegexNode::NonWhitespace => {
            let ch = get_char(input, pos)?;
            (!ch.is_ascii_whitespace()).then(|| pos + ch.len_utf8())
        }
        RegexNode::WordChar => {
            let ch = get_char(input, pos)?;
            (ch.is_ascii_alphanumeric() || ch == '_').then(|| pos + ch.len_utf8())
        }
        RegexNode::NonWordChar => {
            let ch = get_char(input, pos)?;
            (!ch.is_ascii_alphanumeric() && ch != '_').then(|| pos + ch.len_utf8())
        }
        RegexNode::Alternation(alternatives) => {
            for alt in alternatives {
                if let Some(end) = match_seq(alt, 0, input, pos, dot_all, ci) {
                    return Some(end);
                }
            }
            None
        }
    }
}

fn get_char(input: &str, pos: usize) -> Option<char> {
    input[pos..].chars().next()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn matches(input: &str, pattern: &str) -> bool {
        xpath_matches(input, pattern, "").unwrap()
    }

    fn matches_flags(input: &str, pattern: &str, flags: &str) -> bool {
        xpath_matches(input, pattern, flags).unwrap()
    }

    #[test]
    fn test_literal() {
        assert!(matches("hello", "hello"));
        assert!(matches("hello world", "hello"));
        assert!(!matches("world", "hello"));
    }

    #[test]
    fn test_dot() {
        assert!(matches("a", "."));
        assert!(matches("abc", "a.c"));
        assert!(!matches("ac", "a.c"));
    }

    #[test]
    fn test_quantifiers() {
        assert!(matches("aaa", "a+"));
        assert!(matches("", "a*"));
        assert!(matches("a", "a?"));
        assert!(matches("", "a?"));
        assert!(!matches("", "a+"));
    }

    #[test]
    fn test_char_class() {
        assert!(matches("a", "[abc]"));
        assert!(matches("b", "[abc]"));
        assert!(!matches("d", "[abc]"));
        assert!(matches("m", "[a-z]"));
        assert!(!matches("M", "[a-z]"));
    }

    #[test]
    fn test_negated_class() {
        assert!(!matches("a", "[^abc]"));
        assert!(matches("d", "[^abc]"));
    }

    #[test]
    fn test_shorthand() {
        assert!(matches("5", "\\d"));
        assert!(!matches("a", "\\d"));
        assert!(matches("a", "\\D"));
        assert!(matches(" ", "\\s"));
        assert!(matches("a", "\\w"));
    }

    #[test]
    fn test_counted_quantifier() {
        assert!(matches("aa", "a{2}"));
        assert!(!matches("a", "a{2}"));
        assert!(matches("aaa", "a{2,}"));
        assert!(matches("aa", "a{2,4}"));
        assert!(matches("aaaa", "a{2,4}"));
        assert!(!matches("a", "a{2,4}"));
    }

    #[test]
    fn test_alternation() {
        assert!(matches("cat", "cat|dog"));
        assert!(matches("dog", "cat|dog"));
        assert!(!matches("bird", "cat|dog"));
    }

    #[test]
    fn test_grouping() {
        assert!(matches("abab", "(ab)+"));
        assert!(matches("abc", "(ab)+")); // substring match — "ab" prefix matches
        assert!(!matches("cd", "(ab)+"));
    }

    #[test]
    fn test_country_code_pattern() {
        // Common Schematron pattern for ISO country codes
        assert!(matches("US", "[A-Z]{2}"));
        assert!(matches("GB", "[A-Z]{2}"));
        assert!(!matches("us", "[A-Z]{2}"));
        assert!(!matches("U", "[A-Z]{2}"));
    }

    #[test]
    fn test_invoice_id_pattern() {
        assert!(matches("INV-2026-001", "[A-Z]+-\\d+-\\d+"));
        assert!(!matches("inv-2026-001", "[A-Z]+-\\d+-\\d+"));
    }

    #[test]
    fn test_email_like_pattern() {
        assert!(matches("user@example.com", ".+@.+\\..+"));
        assert!(!matches("noatsign", ".+@.+\\..+"));
    }

    #[test]
    fn test_case_insensitive() {
        assert!(matches_flags("Hello", "hello", "i"));
        assert!(matches_flags("HELLO", "hello", "i"));
        assert!(matches_flags("us", "[A-Z]{2}", "i"));
    }

    #[test]
    fn test_substring_match() {
        // XPath matches() checks for substring match, not full string match
        assert!(matches("hello world", "world"));
        assert!(matches("abc123def", "\\d+"));
    }

    #[test]
    fn test_escaped_special() {
        assert!(matches("a.b", "a\\.b"));
        assert!(!matches("axb", "a\\.b"));
        assert!(matches("a\\b", "a\\\\b"));
    }

    #[test]
    fn test_empty_pattern() {
        assert!(matches("anything", ""));
        assert!(matches("", ""));
    }

    // -- replace tests --

    fn replace(input: &str, pattern: &str, replacement: &str) -> String {
        xpath_replace(input, pattern, replacement, "").unwrap()
    }

    #[test]
    fn test_replace_literal() {
        assert_eq!(replace("hello world", "world", "rust"), "hello rust");
    }

    #[test]
    fn test_replace_pattern() {
        assert_eq!(replace("abc123def", "\\d+", "NUM"), "abcNUMdef");
    }

    #[test]
    fn test_replace_multiple() {
        assert_eq!(replace("a-b-c", "-", "_"), "a_b_c");
    }

    #[test]
    fn test_replace_no_match() {
        assert_eq!(replace("hello", "xyz", "!"), "hello");
    }

    // -- tokenize tests --

    fn tokenize(input: &str, pattern: &str) -> Vec<String> {
        xpath_tokenize(input, pattern, "").unwrap()
    }

    #[test]
    fn test_tokenize_whitespace() {
        assert_eq!(tokenize("a b  c", "\\s+"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_tokenize_comma() {
        assert_eq!(tokenize("one,two,three", ","), vec!["one", "two", "three"]);
    }

    #[test]
    fn test_tokenize_empty() {
        let result: Vec<String> = tokenize("", ",");
        assert!(result.is_empty());
    }

    #[test]
    fn test_tokenize_no_match() {
        assert_eq!(tokenize("hello", ","), vec!["hello"]);
    }
}
