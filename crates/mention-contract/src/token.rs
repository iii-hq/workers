//! The mention token: `@<name>(id="<id>")`.
//!
//! - `<name>`: lowercase ASCII letters, digits and `-`, starting with a
//!   letter, at most [`MAX_NAME_LEN`] bytes, not one of
//!   [`RESERVED_NAMES`](crate::RESERVED_NAMES).
//! - `<id>`: a JSON string literal (so `"` and `\` are escaped), decoding
//!   to 1..=[`MAX_ID_LEN`] characters.
//! - The `@` must not follow a letter, digit or `_` (`me@kanban(id="x")`
//!   is not a mention).
//! - Tolerated on read, never written: a missing closing quote,
//!   `@<name>(id="<id>)`, when the id holds no quote, backslash or `)` —
//!   models drop that quote often enough that a strict reader would leave
//!   their replies full of raw tokens.
//!
//! The console's TypeScript grammar (`ade/web/src/lib/mentions/token.ts`)
//! matches this one; both run `fixtures/tokens.json`.

use crate::RESERVED_NAMES;

/// Longest token name, in bytes.
pub const MAX_NAME_LEN: usize = 40;
/// Longest id, in characters (after JSON unescaping).
pub const MAX_ID_LEN: usize = 512;

const OPEN: &str = "(id=\"";
const CLOSE: &str = "\")";

/// One mention found in text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionRef {
    pub name: String,
    pub id: String,
    /// The token exactly as written.
    pub token: String,
}

impl MentionRef {
    /// `name` + `id`, the identity two tokens share when they point at the
    /// same item however their ids were escaped.
    pub fn key(&self) -> (String, String) {
        (self.name.clone(), self.id.clone())
    }
}

/// Whether `name` can be a token name.
pub fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_NAME_LEN
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && !RESERVED_NAMES.contains(&name)
}

/// Writes the token for one item.
pub fn format_mention(name: &str, id: &str) -> String {
    let literal = serde_json::to_string(id).expect("a string always serializes");
    format!("@{name}(id={literal})")
}

/// Every mention in `text`, in order.
pub fn parse_mentions(text: &str) -> Vec<MentionRef> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('@') {
        let at = from + offset;
        from = at + 1;
        let follows_word = text[..at]
            .chars()
            .next_back()
            .is_some_and(|prev| prev.is_alphanumeric() || prev == '_');
        if follows_word {
            continue;
        }
        if let Some((mention, end)) = parse_at(text, at) {
            out.push(mention);
            from = end;
        }
    }
    out
}

/// Like [`parse_mentions`], but skips markdown code: anything between a run
/// of backticks and the next run of the same length (inline code and
/// fenced blocks alike). A chat surface never renders a token inside code
/// as a mention, so it is not one.
pub fn parse_prose_mentions(text: &str) -> Vec<MentionRef> {
    let mut out = Vec::new();
    for segment in prose_segments(text) {
        out.extend(parse_mentions(segment));
    }
    out
}

/// The parts of `text` outside backtick-delimited code.
fn prose_segments(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut segments = Vec::new();
    let mut prose_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
        }
        // An unmatched run is literal text.
        if let Some(close_end) = find_backtick_run(bytes, i, i - run_start) {
            segments.push(&text[prose_start..run_start]);
            prose_start = close_end;
            i = close_end;
        }
    }
    segments.push(&text[prose_start..]);
    segments
}

/// End of the next run of exactly `len` backticks at or after `from`.
fn find_backtick_run(bytes: &[u8], mut from: usize, len: usize) -> Option<usize> {
    while from < bytes.len() {
        if bytes[from] != b'`' {
            from += 1;
            continue;
        }
        let start = from;
        while from < bytes.len() && bytes[from] == b'`' {
            from += 1;
        }
        if from - start == len {
            return Some(from);
        }
    }
    None
}

/// Parses a token starting at the `@` at byte `at`; returns it and the byte
/// just past it.
fn parse_at(text: &str, at: usize) -> Option<(MentionRef, usize)> {
    let rest = &text[at + 1..];
    let name_len = rest
        .bytes()
        .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        .count();
    let name = &rest[..name_len];
    if !is_valid_name(name) {
        return None;
    }
    let body = rest[name_len..].strip_prefix(OPEN)?;
    let (id, consumed) = strict_id(body).or_else(|| unclosed_id(body))?;
    let id_len = id.chars().count();
    if id_len == 0 || id_len > MAX_ID_LEN {
        return None;
    }
    let end = at + 1 + name_len + OPEN.len() + consumed;
    Some((
        MentionRef {
            name: name.to_string(),
            id,
            token: text[at..end].to_string(),
        },
        end,
    ))
}

/// `<json string body>")`: the decoded id and the bytes consumed.
fn strict_id(body: &str) -> Option<(String, usize)> {
    let body_len = json_string_body_len(body)?;
    if !body[body_len..].starts_with(CLOSE) {
        return None;
    }
    let id: String = serde_json::from_str(&format!("\"{}\"", &body[..body_len])).ok()?;
    Some((id, body_len + CLOSE.len()))
}

/// The tolerated `<id>)` (closing quote dropped): an id with no quote,
/// backslash, `)` or control character.
fn unclosed_id(body: &str) -> Option<(String, usize)> {
    let len = body.find(')')?;
    let id = &body[..len];
    if id.is_empty() || id.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        return None;
    }
    Some((id.to_string(), len + 1))
}

/// Byte length of a JSON string body up to (not including) its closing
/// quote, or `None` when the body is malformed or unterminated.
fn json_string_body_len(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut i = 0;
    loop {
        match *bytes.get(i)? {
            b'"' => return Some(i),
            b'\\' => match *bytes.get(i + 1)? {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => i += 2,
                b'u' => {
                    let hex = bytes.get(i + 2..i + 6)?;
                    if !hex.iter().all(u8::is_ascii_hexdigit) {
                        return None;
                    }
                    i += 6;
                }
                _ => return None,
            },
            b if b < 0x20 => return None,
            _ => i += 1,
        }
    }
}
