//! CSS tokenizer and the token stream the parser reads from.
//!
//! The tokenizer walks the input once and emits [`Token`] values. It mirrors
//! the CSS syntax grammar for identifiers, hashes, strings, numbers, comments,
//! whitespace, and single-character delimiters. Escapes (`\HHHHHH` unicode
//! escapes and `\X` simple escapes) are decoded as the tokens are produced.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::error::SelectorError;

/// The kind of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    /// An identifier.
    Ident,
    /// A hash, such as `#foo`. The value drops the leading `#`.
    Hash,
    /// A quoted string. The value drops the quotes and decodes escapes.
    String,
    /// A run of whitespace, collapsed to a single space value.
    S,
    /// A number kept as its raw text, such as `-3.7`.
    Number,
    /// A single delimiter character that did not start another token.
    Delim,
    /// End of input.
    Eof,
}

impl TokenType {
    /// The short name used in token `repr` output and parser error messages.
    pub(crate) fn name(self) -> &'static str {
        match self {
            TokenType::Ident => "IDENT",
            TokenType::Hash => "HASH",
            TokenType::String => "STRING",
            TokenType::S => "S",
            TokenType::Number => "NUMBER",
            TokenType::Delim => "DELIM",
            TokenType::Eof => "EOF",
        }
    }
}

/// A single token with its source position.
///
/// Equality through [`Token::matches`] compares only the type and value. The
/// position is carried for error messages and the `:scope` placement guard but
/// is ignored when matching against expected `(type, value)` pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token kind.
    pub ty: TokenType,
    /// The decoded value. `None` only for the end-of-input token.
    pub value: Option<String>,
    /// The zero-based character index where the token starts.
    pub pos: usize,
}

impl Token {
    /// Build a token with a value.
    pub fn new(ty: TokenType, value: impl Into<String>, pos: usize) -> Token {
        Token {
            ty,
            value: Some(value.into()),
            pos,
        }
    }

    /// Build the end-of-input token.
    pub fn eof(pos: usize) -> Token {
        Token {
            ty: TokenType::Eof,
            value: None,
            pos,
        }
    }

    /// The decoded value as a string slice. Empty for the EOF token.
    pub fn value_str(&self) -> &str {
        self.value.as_deref().unwrap_or("")
    }

    /// True when this token is a delimiter whose value is in `values`.
    pub fn is_delim(&self, values: &[&str]) -> bool {
        self.ty == TokenType::Delim && values.contains(&self.value_str())
    }

    /// True when type and value match the given pair. Position is ignored.
    pub fn matches(&self, ty: TokenType, value: &str) -> bool {
        self.ty == ty && self.value_str() == value
    }

    /// The CSS serialization of the token value, used by `canonical()`.
    ///
    /// Strings are rendered with Python-style `repr`. Every other type returns
    /// its raw value.
    pub fn to_css(&self) -> String {
        if self.ty == TokenType::String {
            crate::util::py_repr(self.value_str())
        } else {
            String::from(self.value_str())
        }
    }
}

impl fmt::Display for Token {
    /// Render the token the way Python `repr(Token)` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ty == TokenType::Eof {
            write!(f, "<EOF at {}>", self.pos)
        } else {
            write!(
                f,
                "<{} '{}' at {}>",
                self.ty.name(),
                self.value_str(),
                self.pos
            )
        }
    }
}

/// Test whether a character is CSS whitespace.
fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{0c}')
}

/// Test whether a character can start an identifier (`nmstart` minus escapes).
fn is_nmstart(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || (c as u32) > 0x7F
}

/// Test whether a character can continue an identifier (`nmchar` minus escapes).
fn is_nmchar(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_alphanumeric() || (c as u32) > 0x7F
}

/// Tokenize a selector string into a token vector ending in an EOF token.
///
/// Returns a syntax error only for an unterminated or malformed string. Every
/// other byte folds into some token.
pub fn tokenize(s: &str) -> Result<Vec<Token>, SelectorError> {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut pos = 0usize;
    let mut tokens = Vec::new();

    while pos < len {
        let c = chars[pos];

        // 1. Whitespace.
        if is_ws(c) {
            let start = pos;
            while pos < len && is_ws(chars[pos]) {
                pos += 1;
            }
            tokens.push(Token::new(TokenType::S, " ", start));
            continue;
        }

        // 2. Identifier. A leading '-' is allowed when followed by an nmstart
        //    character or an escape.
        if let Some(end) = match_ident(&chars, pos) {
            let raw: String = chars[pos..end].iter().collect();
            let value = unescape_ident(&raw);
            tokens.push(Token::new(TokenType::Ident, value, pos));
            pos = end;
            continue;
        }

        // 3. Hash.
        if c == '#' {
            if let Some(end) = match_hash(&chars, pos) {
                let raw: String = chars[pos + 1..end].iter().collect();
                let value = unescape_ident(&raw);
                tokens.push(Token::new(TokenType::Hash, value, pos));
                pos = end;
                continue;
            }
        }

        // 4. String.
        if c == '\'' || c == '"' {
            let quote = c;
            let end = match_string_body(&chars, pos + 1, quote);
            if end == len {
                return Err(SelectorError::Syntax(alloc::format!(
                    "Unclosed string at {pos}"
                )));
            }
            if chars[end] != quote {
                return Err(SelectorError::Syntax(alloc::format!(
                    "Invalid string at {pos}"
                )));
            }
            let raw: String = chars[pos + 1..end].iter().collect();
            let value = unescape_string(&raw);
            tokens.push(Token::new(TokenType::String, value, pos));
            pos = end + 1;
            continue;
        }

        // 5. Number.
        if let Some(end) = match_number(&chars, pos) {
            let raw: String = chars[pos..end].iter().collect();
            tokens.push(Token::new(TokenType::Number, raw, pos));
            pos = end;
            continue;
        }

        // 6. Comment. An unterminated comment consumes to end of input.
        if c == '/' && pos + 1 < len && chars[pos + 1] == '*' {
            let mut search = pos + 2;
            let mut found = None;
            while search + 1 < len {
                if chars[search] == '*' && chars[search + 1] == '/' {
                    found = Some(search + 2);
                    break;
                }
                search += 1;
            }
            pos = found.unwrap_or(len);
            continue;
        }

        // 7. Otherwise a single-character delimiter.
        tokens.push(Token::new(TokenType::Delim, c, pos));
        pos += 1;
    }

    tokens.push(Token::eof(len));
    Ok(tokens)
}

/// Length of an escape sequence starting at `chars[i]` (where `chars[i]` is the
/// backslash), or `None` when the backslash does not begin a valid escape.
///
/// A unicode escape is a backslash, one to six hex digits, and an optional
/// trailing whitespace character (with `\r\n` counted as one). A simple escape
/// is a backslash and one character that is not a newline or hex digit.
fn escape_len(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i) != Some(&'\\') {
        return None;
    }
    let after = i + 1;
    let next = chars.get(after)?;
    if next.is_ascii_hexdigit() {
        let mut j = after;
        let mut count = 0;
        while j < chars.len() && count < 6 && chars[j].is_ascii_hexdigit() {
            j += 1;
            count += 1;
        }
        // Optional single trailing whitespace, with \r\n consumed as a pair.
        if j + 1 < chars.len() && chars[j] == '\r' && chars[j + 1] == '\n' {
            j += 2;
        } else if j < chars.len() && is_ws(chars[j]) {
            j += 1;
        }
        Some(j - i)
    } else if matches!(next, '\n' | '\r' | '\u{0c}') {
        // Not a valid simple escape: backslash followed by a newline.
        None
    } else {
        // Simple escape consumes the backslash and one character.
        Some(2)
    }
}

/// Match an identifier starting at `pos`. Returns the end index, exclusive.
fn match_ident(chars: &[char], pos: usize) -> Option<usize> {
    let len = chars.len();
    let mut i = pos;
    if i < len && chars[i] == '-' {
        i += 1;
    }
    // First piece must be an nmstart or an escape.
    if i < len && is_nmstart(chars[i]) {
        i += 1;
    } else {
        let step = escape_len(chars, i)?;
        i += step;
    }
    // Remaining nmchar pieces.
    loop {
        if i < len && is_nmchar(chars[i]) {
            i += 1;
        } else if let Some(step) = escape_len(chars, i) {
            i += step;
        } else {
            break;
        }
    }
    Some(i)
}

/// Match a hash starting at `pos` (the `#`). Returns the end index, exclusive.
fn match_hash(chars: &[char], pos: usize) -> Option<usize> {
    let len = chars.len();
    let mut i = pos + 1;
    let start = i;
    loop {
        if i < len && is_nmchar(chars[i]) {
            i += 1;
        } else if let Some(step) = escape_len(chars, i) {
            i += step;
        } else {
            break;
        }
    }
    if i == start {
        None
    } else {
        Some(i)
    }
}

/// Match a number starting at `pos`. Returns the end index, exclusive.
///
/// The pattern is an optional sign, then either digits, a dot, and digits, or a
/// plain run of digits.
fn match_number(chars: &[char], pos: usize) -> Option<usize> {
    let len = chars.len();
    let mut i = pos;
    if i < len && (chars[i] == '+' || chars[i] == '-') {
        i += 1;
    }
    let int_start = i;
    while i < len && chars[i].is_ascii_digit() {
        i += 1;
    }
    let had_int = i > int_start;
    if i < len && chars[i] == '.' {
        let dot = i;
        i += 1;
        let frac_start = i;
        while i < len && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i > frac_start {
            return Some(i);
        }
        // A dot with no following digits is only valid if integer digits ran.
        i = dot;
    }
    if had_int {
        Some(i)
    } else {
        None
    }
}

/// Find where a string body ends starting at `pos` (just past the open quote).
///
/// Returns the index of the closing quote, or the input length when the string
/// runs to end of input. The body allows escapes, including line-continuation
/// escapes, and forbids raw newlines.
fn match_string_body(chars: &[char], pos: usize, quote: char) -> usize {
    let len = chars.len();
    let mut i = pos;
    while i < len {
        let c = chars[i];
        if c == quote {
            return i;
        }
        if matches!(c, '\n' | '\r' | '\u{0c}') {
            return i;
        }
        if c == '\\' {
            // Line continuation: backslash then a newline form.
            if i + 1 < len {
                if chars[i + 1] == '\r' && i + 2 < len && chars[i + 2] == '\n' {
                    i += 3;
                    continue;
                }
                if matches!(chars[i + 1], '\n' | '\r' | '\u{0c}') {
                    i += 2;
                    continue;
                }
            }
            if let Some(step) = escape_len(chars, i) {
                i += step;
                continue;
            }
            // A trailing lone backslash.
            i += 1;
            continue;
        }
        i += 1;
    }
    len
}

/// Decode the unicode and simple escapes in an identifier or hash value.
pub fn unescape_ident(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    decode_escapes(&chars, false)
}

/// Decode the escapes in a string value, including line continuations.
fn unescape_string(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    decode_escapes(&chars, true)
}

/// Shared escape decoder.
///
/// When `strip_newlines` is set, a backslash before a newline form is dropped
/// (line continuation). A unicode escape maps to its code point. Two ranges map
/// to U+FFFD instead: code points above U+10FFFF, and the surrogate range
/// U+D800 through U+DFFF. A Rust string cannot hold a surrogate, so the decoder
/// folds it to the replacement character. Simple escapes drop the backslash.
fn decode_escapes(chars: &[char], strip_newlines: bool) -> String {
    let len = chars.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if chars[i] != '\\' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let after = i + 1;
        if after >= len {
            out.push('\\');
            i += 1;
            continue;
        }
        let next = chars[after];
        if strip_newlines {
            if next == '\r' && after + 1 < len && chars[after + 1] == '\n' {
                i = after + 2;
                continue;
            }
            if matches!(next, '\n' | '\r' | '\u{0c}') {
                i = after + 1;
                continue;
            }
        }
        if next.is_ascii_hexdigit() {
            let mut j = after;
            let mut count = 0;
            let mut cp: u32 = 0;
            while j < len && count < 6 && chars[j].is_ascii_hexdigit() {
                cp = cp * 16
                    + chars[j]
                        .to_digit(16)
                        .expect("hex digit checked by the loop guard");
                j += 1;
                count += 1;
            }
            // Eat one optional trailing whitespace, with \r\n as a pair.
            if j + 1 < len && chars[j] == '\r' && chars[j + 1] == '\n' {
                j += 2;
            } else if j < len && is_ws(chars[j]) {
                j += 1;
            }
            // Above the Unicode maximum folds to U+FFFD. A surrogate code point
            // also folds here, since `char::from_u32` rejects the surrogate
            // range and a Rust string cannot store one.
            let decoded = if cp > 0x10FFFF {
                '\u{FFFD}'
            } else {
                char::from_u32(cp).unwrap_or('\u{FFFD}')
            };
            out.push(decoded);
            i = j;
        } else {
            // Simple escape: drop the backslash, keep the character.
            out.push(next);
            i = after + 1;
        }
    }
    out
}
