//! Python-string semantics: scrapling's TextHandler.clean()/.re() ports.

use rustpython_sre_engine::{compiler, Request, SearchIter, State, StrDrive};

pub fn clean(s: &str) -> String {
    let translated: String = s
        .chars()
        .map(|c| {
            if matches!(c, '\t' | '\r' | '\n') {
                ' '
            } else {
                c
            }
        })
        .collect();
    collapse_spaces(&translated).trim().to_string()
}

/// scrapling `clean_spaces` (`scrapling/core/utils/_utils.py`:
/// `__CLEANING_TABLE__ = str.maketrans({"\t": " ", "\n": None, "\r": None})`
/// then `__CONSECUTIVE_SPACES_REGEX__.sub(" ", string)`). Unlike `clean`
/// above: `\n`/`\r` are DELETED outright (not turned into a space — so they
/// can glue two words together with no separator at all), and there is no
/// trim. Used by `similar`'s `match_text` scoring (`__are_alike`); `clean`
/// stays the one `find_by_text`/`find_by_regex` use — this is a genuinely
/// different function in scrapling, not a duplicate.
pub fn clean_spaces(s: &str) -> String {
    let translated: String = s
        .chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            '\n' | '\r' => None,
            other => Some(other),
        })
        .collect();
    collapse_spaces(&translated)
}

/// Runs of the ASCII space character collapse to one (shared by `clean` and
/// `clean_spaces`; the only difference between them is the translate step
/// and whether the result gets trimmed afterward).
fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// XPath normalize-space: trim + collapse all whitespace runs to single spaces.
pub fn normalize_space(s: &str) -> String {
    s.split([' ', '\t', '\r', '\n'])
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct PyRegex {
    codes: Vec<u32>,
    groups: usize,
}

pub fn compile(pattern: &str, case_insensitive: bool) -> Result<PyRegex, String> {
    let compiled =
        compiler::compile(pattern, case_insensitive).map_err(|error| error.to_string())?;
    Ok(PyRegex {
        codes: compiled.codes,
        groups: compiled.groups,
    })
}

impl PyRegex {
    pub fn findall(&self, text: &str) -> Vec<String> {
        let request = Request::new(text, 0, text.count(), &self.codes, false);
        let mut matches = SearchIter {
            req: request,
            state: State::default(),
        };
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        while matches.next().is_some() {
            if self.groups == 0 {
                out.push(
                    chars[matches.state.start..matches.state.cursor.position]
                        .iter()
                        .collect(),
                );
            } else {
                for group in 0..self.groups {
                    let (start, end) = matches.state.marks.get(group);
                    out.push(match (start.into_option(), end.into_option()) {
                        (Some(start), Some(end)) => chars[start..end].iter().collect(),
                        _ => String::new(),
                    });
                }
            }
        }
        out.into_iter()
            .map(|match_| replace_entities(&match_))
            .collect()
    }

    pub fn find_first(&self, text: &str) -> Option<String> {
        self.findall(text).into_iter().next()
    }

    pub fn check_match(&self, text: &str) -> bool {
        let request = Request::new(text, 0, text.count(), &self.codes, false);
        State::default().search(request)
    }
}

fn replace_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(relative) = text[position..].find('&') {
        let amp = position + relative;
        out.push_str(&text[position..amp]);
        let Some((consumed, entity, semicolon)) = parse_entity(&text[amp + 1..]) else {
            out.push('&');
            position = amp + 1;
            continue;
        };
        let whole_end = amp + 1 + consumed;
        let replacement: Option<String> = match entity {
            Entity::Named(name) if name != "apos" => xmloxide::html::entities::lookup_entity(name)
                .or_else(|| {
                    let lower = name.to_lowercase();
                    (lower != "apos")
                        .then(|| xmloxide::html::entities::lookup_entity(&lower))
                        .flatten()
                })
                .map(str::to_string),
            Entity::Number(number) => codepoint(number).map(|ch| ch.to_string()),
            _ => None,
        };
        if let Some(replacement) = replacement {
            out.push_str(&replacement);
        } else if !semicolon {
            out.push_str(&text[amp..whole_end]);
        }
        position = whole_end;
    }
    out.push_str(&text[position..]);
    out
}

enum Entity<'a> {
    Named(&'a str),
    Number(Option<u32>),
}

fn parse_entity(input: &str) -> Option<(usize, Entity<'_>, bool)> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    let (body_start, radix, named) = if first == '#' {
        match chars.next() {
            Some((_, 'x' | 'X')) => (2, 16, false),
            Some(_) => (1, 10, false),
            None => return None,
        }
    } else {
        (0, 10, true)
    };
    let body = &input[body_start..];
    let mut body_end = 0;
    for (offset, ch) in body.char_indices() {
        let valid = if named {
            ch.is_ascii_alphabetic()
                || decimal_digit(ch).is_some()
                || matches!(ch, '\u{130}' | '\u{131}' | '\u{17f}' | '\u{212a}')
        } else if radix == 16 {
            ch.is_ascii_hexdigit() || decimal_digit(ch).is_some()
        } else {
            decimal_digit(ch).is_some()
        };
        if !valid {
            break;
        }
        body_end = offset + ch.len_utf8();
    }
    if body_end == 0 {
        return None;
    }
    let semicolon = body[body_end..].starts_with(';');
    let consumed = body_start + body_end + usize::from(semicolon);
    let value = &body[..body_end];
    Some((
        consumed,
        if named {
            Entity::Named(value)
        } else {
            Entity::Number(parse_number(value, radix))
        },
        semicolon,
    ))
}

fn parse_number(value: &str, radix: u32) -> Option<u32> {
    value.chars().try_fold(0u32, |number, ch| {
        let digit = decimal_digit(ch).or_else(|| ch.to_digit(radix))?;
        (digit < radix)
            .then(|| number.checked_mul(radix)?.checked_add(digit))
            .flatten()
    })
}

fn decimal_digit(ch: char) -> Option<u32> {
    const ZEROES: &[u32] = &[
        0x0030, 0x0660, 0x06f0, 0x07c0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66,
        0x0ce6, 0x0d66, 0x0de6, 0x0e50, 0x0ed0, 0x0f20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946,
        0x19d0, 0x1a80, 0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0,
        0xa9f0, 0xaa50, 0xabf0, 0xff10, 0x104a0, 0x10d30, 0x11066, 0x110f0, 0x11136, 0x111d0,
        0x112f0, 0x11450, 0x114d0, 0x11650, 0x116c0, 0x11730, 0x118e0, 0x11950, 0x11c50, 0x11d50,
        0x11da0, 0x11f50, 0x16a60, 0x16ac0, 0x16b50, 0x1d7ce, 0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6,
        0x1e140, 0x1e2f0, 0x1e4f0, 0x1e950, 0x1fbf0,
    ];
    let codepoint = ch as u32;
    let index = ZEROES
        .partition_point(|candidate| *candidate <= codepoint)
        .checked_sub(1)?;
    let zero = ZEROES[index];
    (codepoint - zero < 10).then_some(codepoint - zero)
}

fn codepoint(number: Option<u32>) -> Option<char> {
    const WINDOWS_1252: [Option<char>; 32] = [
        Some('\u{20ac}'),
        None,
        Some('\u{201a}'),
        Some('\u{0192}'),
        Some('\u{201e}'),
        Some('\u{2026}'),
        Some('\u{2020}'),
        Some('\u{2021}'),
        Some('\u{02c6}'),
        Some('\u{2030}'),
        Some('\u{0160}'),
        Some('\u{2039}'),
        Some('\u{0152}'),
        None,
        Some('\u{017d}'),
        None,
        None,
        Some('\u{2018}'),
        Some('\u{2019}'),
        Some('\u{201c}'),
        Some('\u{201d}'),
        Some('\u{2022}'),
        Some('\u{2013}'),
        Some('\u{2014}'),
        Some('\u{02dc}'),
        Some('\u{2122}'),
        Some('\u{0161}'),
        Some('\u{203a}'),
        Some('\u{0153}'),
        None,
        Some('\u{017e}'),
        Some('\u{0178}'),
    ];
    let number = number?;
    if (0x80..=0x9f).contains(&number) {
        WINDOWS_1252[(number - 0x80) as usize]
    } else {
        char::from_u32(number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_translates_and_collapses_spaces_only() {
        // \t\r\n each become a space, then RUNS OF SPACES collapse, then trim.
        assert_eq!(clean(" a\t\tb\n\nc  d "), "a b c d");
    }

    #[test]
    fn clean_spaces_deletes_newlines_gluing_words_and_never_trims() {
        // Oracle (`scrapling.core.utils.clean_spaces`), reviewer-verified
        // counterexamples: \n/\r are DELETED (no space inserted), unlike
        // `clean`'s \t/\r/\n -> ' ' translation, and there is no trim at all.
        assert_eq!(clean_spaces("a\nb"), "ab");
        assert_eq!(clean_spaces(" a "), " a "); // no trim
        assert_eq!(clean_spaces("a\tb"), "a b"); // \t still becomes a space
        assert_eq!(clean_spaces("a\r\nb"), "ab"); // \r and \n both deleted
        assert_eq!(clean_spaces("a    b"), "a b"); // space runs still collapse
        assert_eq!(clean_spaces(""), "");
    }

    #[test]
    fn normalize_space_collapses_all_whitespace() {
        assert_eq!(normalize_space("  a \t b \n "), "a b");
        assert_eq!(normalize_space(" \n\t "), "");
    }

    #[test]
    fn findall_without_groups_returns_whole_matches() {
        let re = compile(r"\d+", false).unwrap();
        assert_eq!(re.findall("price 42 usd then 99"), vec!["42", "99"]);
    }

    #[test]
    fn findall_with_one_group_returns_group_values() {
        let re = compile(r"price (\d+)", false).unwrap();
        assert_eq!(re.findall("price 42 usd then price 99"), vec!["42", "99"]);
    }

    #[test]
    fn findall_with_two_groups_flattens_in_order() {
        let re = compile(r"(\w+)=(\d+)", false).unwrap();
        assert_eq!(re.findall("a=1 b=2"), vec!["a", "1", "b", "2"]);
    }

    #[test]
    fn findall_unmatched_group_is_empty_string() {
        let re = compile(r"(a)|(b)", false).unwrap();
        assert_eq!(re.findall("ab"), vec!["a", "", "", "b"]);
    }

    #[test]
    fn results_are_entity_decoded() {
        // scrapling passes replace_entities=True: "&amp;co" decodes to "&co"
        let re = compile(r"&amp;\w+", false).unwrap();
        assert_eq!(re.findall("Smith &amp;co"), vec!["&co"]);
        assert_eq!(
            replace_entities("&Copy; &apos; &unknown; &unknown"),
            "©   &unknown"
        );
        assert_eq!(replace_entities("&#128; &#129; &#x80 &#x81"), "€  € &#x81");
        assert_eq!(replace_entities("&#०; &#x９;"), "\0 \t");
    }

    #[test]
    fn case_insensitive_flag_and_lookahead_work() {
        let re = compile(r"apple(?= pie)", true).unwrap();
        assert!(re.check_match("APPLE PIE".to_lowercase().as_str()));
        assert_eq!(
            compile(r"HeLLo", true).unwrap().findall("hello"),
            vec!["hello"]
        );
    }

    #[test]
    fn invalid_pattern_is_an_error() {
        assert!(compile(r"(", false).is_err());
    }
}
