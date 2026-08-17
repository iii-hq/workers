//! Internal incremental JSON reconstruction for streamed tool arguments.
//!
//! Ported from <https://github.com/sergiofilhowz/json-stream-watcher>.
//!
//! MIT License
//! Copyright (c) 2024 Sérgio Marcelino
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to
//! deal in the Software without restriction, including without limitation the
//! rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
//! sell copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions: the above
//! copyright notice and this permission notice shall be included in all copies
//! or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//! FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
//! IN THE SOFTWARE.
//!
//! A normal JSON parser exposes nothing until the closing delimiter arrives.
//! [`JsonStream`] keeps parser state between writes and can instead return a
//! [`Snapshot`] containing every completed value plus the string or container
//! currently being built.  It is intentionally small and assumes the producer
//! intends to send valid JSON, matching the LLM tool-argument use case.

use serde_json::{Map, Number, Value};
use std::fmt;

/// One view of the streamed document.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub value: Value,
    /// `true` only after the root value's closing token was received.
    pub complete: bool,
}

/// A syntax error at a byte offset in the concatenated stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for Error {}

/// Stateful incremental JSON parser.
#[derive(Debug, Default)]
pub struct JsonStream {
    frames: Vec<Frame>,
    token: Option<Token>,
    root: Option<Value>,
    bytes_seen: usize,
    error: Option<Error>,
}

#[derive(Debug)]
enum Frame {
    Object {
        values: Map<String, Value>,
        state: ObjectState,
        key: Option<String>,
    },
    Array {
        values: Vec<Value>,
        state: ArrayState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectState {
    KeyOrEnd,
    Key,
    Colon,
    Value,
    CommaOrEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayState {
    ValueOrEnd,
    Value,
    CommaOrEnd,
}

#[derive(Debug)]
enum Token {
    String(StringToken),
    Number(String),
    Literal { kind: Literal, matched: usize },
}

#[derive(Debug)]
struct StringToken {
    value: String,
    target: StringTarget,
    escape: StringEscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringTarget {
    Key,
    Value,
}

#[derive(Debug)]
enum StringEscape {
    None,
    Escape,
    Unicode { value: u16, digits: u8 },
    LowSurrogateSlash { high: u16 },
    LowSurrogateU { high: u16 },
    LowSurrogate { high: u16, value: u16, digits: u8 },
}

#[derive(Debug, Clone, Copy)]
enum Literal {
    True,
    False,
    Null,
}

impl Literal {
    fn text(self) -> &'static str {
        match self {
            Self::True => "true",
            Self::False => "false",
            Self::Null => "null",
        }
    }

    fn value(self) -> Value {
        match self {
            Self::True => Value::Bool(true),
            Self::False => Value::Bool(false),
            Self::Null => Value::Null,
        }
    }
}

impl JsonStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume another piece of the same JSON document.
    pub fn write(&mut self, chunk: &str) -> Result<(), Error> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        for (index, ch) in chunk.char_indices() {
            let offset = self.bytes_seen + index;
            let mut reprocess = true;
            while reprocess {
                reprocess = match self.consume(ch, offset) {
                    Ok(reprocess) => reprocess,
                    Err(error) => {
                        self.error = Some(error.clone());
                        return Err(error);
                    }
                };
            }
        }
        self.bytes_seen += chunk.len();
        Ok(())
    }

    /// Everything observable so far. Open strings and containers are present
    /// in the returned value; incomplete keys and literals are not.
    pub fn snapshot(&self) -> Option<Snapshot> {
        if let Some(root) = &self.root {
            return Some(Snapshot {
                value: root.clone(),
                complete: self.frames.is_empty() && self.token.is_none() && self.error.is_none(),
            });
        }

        let mut current = self.partial_token_value();
        for frame in self.frames.iter().rev() {
            current = Some(match frame {
                Frame::Object { values, state, key } => {
                    let mut values = values.clone();
                    if *state == ObjectState::Value {
                        if let (Some(key), Some(value)) = (key, current.take()) {
                            values.insert(key.clone(), value);
                        }
                    }
                    Value::Object(values)
                }
                Frame::Array { values, state } => {
                    let mut values = values.clone();
                    if matches!(state, ArrayState::ValueOrEnd | ArrayState::Value) {
                        if let Some(value) = current.take() {
                            values.push(value);
                        }
                    }
                    Value::Array(values)
                }
            });
        }

        current.map(|value| Snapshot {
            value,
            complete: false,
        })
    }

    #[cfg(test)]
    pub fn is_complete(&self) -> bool {
        self.root.is_some()
            && self.frames.is_empty()
            && self.token.is_none()
            && self.error.is_none()
    }

    fn consume(&mut self, ch: char, offset: usize) -> Result<bool, Error> {
        if self.token.is_some() {
            return self.consume_token(ch, offset);
        }
        if ch.is_whitespace() {
            return Ok(false);
        }
        if self.root.is_some() && self.frames.is_empty() {
            return Err(syntax(offset, "unexpected data after the root value"));
        }

        enum Context {
            RootValue,
            Object(ObjectState),
            Array(ArrayState),
        }
        let context = match self.frames.last() {
            Some(Frame::Object { state, .. }) => Context::Object(*state),
            Some(Frame::Array { state, .. }) => Context::Array(*state),
            None => Context::RootValue,
        };

        match context {
            Context::RootValue
            | Context::Object(ObjectState::Value)
            | Context::Array(ArrayState::Value) => self.start_value(ch, offset)?,
            Context::Array(ArrayState::ValueOrEnd) => {
                if ch == ']' {
                    self.close_array(offset)?;
                } else {
                    self.start_value(ch, offset)?;
                }
            }
            Context::Object(state @ (ObjectState::KeyOrEnd | ObjectState::Key)) => match ch {
                '}' if state == ObjectState::KeyOrEnd => self.close_object(offset)?,
                '"' => {
                    self.token = Some(Token::String(StringToken {
                        value: String::new(),
                        target: StringTarget::Key,
                        escape: StringEscape::None,
                    }));
                }
                _ => return Err(syntax(offset, "expected an object key")),
            },
            Context::Object(ObjectState::Colon) => {
                if ch != ':' {
                    return Err(syntax(offset, "expected ':' after an object key"));
                }
                if let Some(Frame::Object { state, .. }) = self.frames.last_mut() {
                    *state = ObjectState::Value;
                }
            }
            Context::Object(ObjectState::CommaOrEnd) => match ch {
                ',' => {
                    if let Some(Frame::Object { state, .. }) = self.frames.last_mut() {
                        *state = ObjectState::Key;
                    }
                }
                '}' => self.close_object(offset)?,
                _ => return Err(syntax(offset, "expected ',' or '}' in an object")),
            },
            Context::Array(ArrayState::CommaOrEnd) => match ch {
                ',' => {
                    if let Some(Frame::Array { state, .. }) = self.frames.last_mut() {
                        *state = ArrayState::Value;
                    }
                }
                ']' => self.close_array(offset)?,
                _ => return Err(syntax(offset, "expected ',' or ']' in an array")),
            },
        }
        Ok(false)
    }

    fn start_value(&mut self, ch: char, offset: usize) -> Result<(), Error> {
        match ch {
            '{' => self.frames.push(Frame::Object {
                values: Map::new(),
                state: ObjectState::KeyOrEnd,
                key: None,
            }),
            '[' => self.frames.push(Frame::Array {
                values: Vec::new(),
                state: ArrayState::ValueOrEnd,
            }),
            '"' => {
                self.token = Some(Token::String(StringToken {
                    value: String::new(),
                    target: StringTarget::Value,
                    escape: StringEscape::None,
                }));
            }
            '-' | '0'..='9' => self.token = Some(Token::Number(ch.to_string())),
            't' => {
                self.token = Some(Token::Literal {
                    kind: Literal::True,
                    matched: 1,
                })
            }
            'f' => {
                self.token = Some(Token::Literal {
                    kind: Literal::False,
                    matched: 1,
                })
            }
            'n' => {
                self.token = Some(Token::Literal {
                    kind: Literal::Null,
                    matched: 1,
                })
            }
            _ => return Err(syntax(offset, "expected a JSON value")),
        }
        Ok(())
    }

    /// Returns true when the same character is a delimiter that still needs
    /// to be handled after completing a number token.
    fn consume_token(&mut self, ch: char, offset: usize) -> Result<bool, Error> {
        let token = self.token.take().expect("token checked above");
        match token {
            Token::String(mut string) => {
                match string.escape {
                    StringEscape::None => match ch {
                        '"' => match string.target {
                            StringTarget::Key => self.complete_key(string.value, offset)?,
                            StringTarget::Value => {
                                self.complete_value(Value::String(string.value), offset)?
                            }
                        },
                        '\\' => {
                            string.escape = StringEscape::Escape;
                            self.token = Some(Token::String(string));
                        }
                        c if c <= '\u{1f}' => {
                            return Err(syntax(offset, "unescaped control character in a string"))
                        }
                        c => {
                            string.value.push(c);
                            self.token = Some(Token::String(string));
                        }
                    },
                    StringEscape::Escape => {
                        match ch {
                            '"' | '\\' | '/' => string.value.push(ch),
                            'b' => string.value.push('\u{0008}'),
                            'f' => string.value.push('\u{000c}'),
                            'n' => string.value.push('\n'),
                            'r' => string.value.push('\r'),
                            't' => string.value.push('\t'),
                            'u' => {
                                string.escape = StringEscape::Unicode {
                                    value: 0,
                                    digits: 0,
                                };
                                self.token = Some(Token::String(string));
                                return Ok(false);
                            }
                            _ => return Err(syntax(offset, "invalid string escape")),
                        }
                        string.escape = StringEscape::None;
                        self.token = Some(Token::String(string));
                    }
                    StringEscape::Unicode {
                        mut value,
                        mut digits,
                    } => {
                        value = (value << 4) | hex(ch, offset)?;
                        digits += 1;
                        string.escape = if digits < 4 {
                            StringEscape::Unicode { value, digits }
                        } else if (0xd800..=0xdbff).contains(&value) {
                            StringEscape::LowSurrogateSlash { high: value }
                        } else if (0xdc00..=0xdfff).contains(&value) {
                            return Err(syntax(offset, "unexpected low surrogate"));
                        } else {
                            string.value.push(
                                char::from_u32(u32::from(value))
                                    .ok_or_else(|| syntax(offset, "invalid unicode escape"))?,
                            );
                            StringEscape::None
                        };
                        self.token = Some(Token::String(string));
                    }
                    StringEscape::LowSurrogateSlash { high } => {
                        if ch != '\\' {
                            return Err(syntax(offset, "expected a low surrogate"));
                        }
                        string.escape = StringEscape::LowSurrogateU { high };
                        self.token = Some(Token::String(string));
                    }
                    StringEscape::LowSurrogateU { high } => {
                        if ch != 'u' {
                            return Err(syntax(offset, "expected a low surrogate"));
                        }
                        string.escape = StringEscape::LowSurrogate {
                            high,
                            value: 0,
                            digits: 0,
                        };
                        self.token = Some(Token::String(string));
                    }
                    StringEscape::LowSurrogate {
                        high,
                        mut value,
                        mut digits,
                    } => {
                        value = (value << 4) | hex(ch, offset)?;
                        digits += 1;
                        if digits < 4 {
                            string.escape = StringEscape::LowSurrogate {
                                high,
                                value,
                                digits,
                            };
                        } else {
                            if !(0xdc00..=0xdfff).contains(&value) {
                                return Err(syntax(offset, "invalid low surrogate"));
                            }
                            let scalar = 0x1_0000
                                + ((u32::from(high) - 0xd800) << 10)
                                + (u32::from(value) - 0xdc00);
                            string.value.push(
                                char::from_u32(scalar)
                                    .ok_or_else(|| syntax(offset, "invalid surrogate pair"))?,
                            );
                            string.escape = StringEscape::None;
                        }
                        self.token = Some(Token::String(string));
                    }
                }
                Ok(false)
            }
            Token::Number(mut raw) => {
                if matches!(ch, '0'..='9' | '.' | 'e' | 'E' | '+' | '-') {
                    raw.push(ch);
                    self.token = Some(Token::Number(raw));
                    return Ok(false);
                }
                let number = serde_json::from_str::<Number>(&raw)
                    .map_err(|_| syntax(offset, "invalid JSON number"))?;
                self.complete_value(Value::Number(number), offset)?;
                Ok(true)
            }
            Token::Literal { kind, matched } => {
                let text = kind.text();
                if text.as_bytes().get(matched).copied() != Some(ch as u8) {
                    return Err(syntax(offset, "invalid JSON literal"));
                }
                let matched = matched + 1;
                if matched == text.len() {
                    self.complete_value(kind.value(), offset)?;
                } else {
                    self.token = Some(Token::Literal { kind, matched });
                }
                Ok(false)
            }
        }
    }

    fn complete_key(&mut self, key: String, offset: usize) -> Result<(), Error> {
        match self.frames.last_mut() {
            Some(Frame::Object {
                state: state @ (ObjectState::KeyOrEnd | ObjectState::Key),
                key: slot,
                ..
            }) => {
                *slot = Some(key);
                *state = ObjectState::Colon;
                Ok(())
            }
            _ => Err(syntax(offset, "object key outside an object")),
        }
    }

    fn complete_value(&mut self, value: Value, offset: usize) -> Result<(), Error> {
        match self.frames.last_mut() {
            Some(Frame::Object {
                values,
                state: state @ ObjectState::Value,
                key,
            }) => {
                let key = key
                    .take()
                    .ok_or_else(|| syntax(offset, "object value has no key"))?;
                values.insert(key, value);
                *state = ObjectState::CommaOrEnd;
            }
            Some(Frame::Array { values, state })
                if matches!(state, ArrayState::ValueOrEnd | ArrayState::Value) =>
            {
                values.push(value);
                *state = ArrayState::CommaOrEnd;
            }
            Some(_) => return Err(syntax(offset, "value in an unexpected position")),
            None if self.root.is_none() => self.root = Some(value),
            None => return Err(syntax(offset, "multiple root values")),
        }
        Ok(())
    }

    fn close_object(&mut self, offset: usize) -> Result<(), Error> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| syntax(offset, "unexpected '}'"))?;
        match frame {
            Frame::Object {
                values,
                state: ObjectState::KeyOrEnd | ObjectState::CommaOrEnd,
                ..
            } => self.complete_value(Value::Object(values), offset),
            frame => {
                self.frames.push(frame);
                Err(syntax(offset, "object ended before its value was complete"))
            }
        }
    }

    fn close_array(&mut self, offset: usize) -> Result<(), Error> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| syntax(offset, "unexpected ']'"))?;
        match frame {
            Frame::Array {
                values,
                state: ArrayState::ValueOrEnd | ArrayState::CommaOrEnd,
            } => self.complete_value(Value::Array(values), offset),
            frame => {
                self.frames.push(frame);
                Err(syntax(offset, "array ended before its value was complete"))
            }
        }
    }

    fn partial_token_value(&self) -> Option<Value> {
        match self.token.as_ref()? {
            Token::String(StringToken {
                value,
                target: StringTarget::Value,
                ..
            }) => Some(Value::String(value.clone())),
            Token::Number(raw) => serde_json::from_str::<Number>(raw).ok().map(Value::Number),
            Token::String(_) | Token::Literal { .. } => None,
        }
    }
}

fn hex(ch: char, offset: usize) -> Result<u16, Error> {
    ch.to_digit(16)
        .map(|digit| digit as u16)
        .ok_or_else(|| syntax(offset, "invalid unicode escape"))
}

fn syntax(offset: usize, message: impl Into<String>) -> Error {
    Error {
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exposes_a_string_before_its_closing_quote() {
        let mut stream = JsonStream::new();
        stream.write(r#"{"function":"state::se"#).unwrap();
        assert_eq!(
            stream.snapshot(),
            Some(Snapshot {
                value: json!({ "function": "state::se" }),
                complete: false,
            })
        );
    }

    #[test]
    fn preserves_completed_and_nested_partial_values_across_chunks() {
        let mut stream = JsonStream::new();
        for chunk in [
            r#"{"function":"state::set","desc"#,
            r#"ription":"Updating st"#,
            r#"ate","payload":{"scope":"art"#,
        ] {
            stream.write(chunk).unwrap();
        }
        assert_eq!(
            stream.snapshot().unwrap().value,
            json!({
                "function": "state::set",
                "description": "Updating state",
                "payload": { "scope": "art" }
            })
        );
        stream.write(r#"ifacts","values":[1,2]} }"#).unwrap();
        let snapshot = stream.snapshot().unwrap();
        assert!(snapshot.complete);
        assert_eq!(snapshot.value["payload"]["scope"], "artifacts");
        assert_eq!(snapshot.value["payload"]["values"], json!([1, 2]));
    }

    #[test]
    fn handles_escapes_and_surrogate_pairs_split_between_chunks() {
        let mut stream = JsonStream::new();
        for chunk in [r#"{"text":"line\n\uD83"#, r#"D\uDE"#, r#"80"}"#] {
            stream.write(chunk).unwrap();
        }
        assert_eq!(
            stream.snapshot().unwrap().value,
            json!({ "text": "line\n🚀" })
        );
        assert!(stream.is_complete());
    }

    #[test]
    fn parses_empty_containers_literals_and_decimal_numbers() {
        let mut stream = JsonStream::new();
        stream
            .write(r#"{"empty":[],"nested":{},"yes":true,"no":false,"nil":null,"n":-1.5e2}"#)
            .unwrap();
        assert_eq!(
            stream.snapshot().unwrap().value,
            json!({
                "empty": [],
                "nested": {},
                "yes": true,
                "no": false,
                "nil": null,
                "n": -150.0
            })
        );
    }

    #[test]
    fn every_chunk_boundary_matches_serde_json_when_complete() {
        let input = r#"{"string":"quote: \" slash: \\ newline: \n","unicode":"\uD83D\uDE80","numbers":[-1.25e2,0,42],"values":[true,false,null,{},[]]}"#;
        let expected: Value = serde_json::from_str(input).unwrap();
        let boundaries: Vec<usize> = input
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(input.len()))
            .collect();

        for &boundary in &boundaries {
            let mut stream = JsonStream::new();
            stream.write(&input[..boundary]).unwrap();
            stream.write(&input[boundary..]).unwrap();
            let snapshot = stream.snapshot().unwrap();
            assert!(snapshot.complete, "boundary {boundary}");
            assert_eq!(snapshot.value, expected, "boundary {boundary}");
        }

        let mut char_at_a_time = JsonStream::new();
        for ch in input.chars() {
            char_at_a_time.write(&ch.to_string()).unwrap();
        }
        assert_eq!(char_at_a_time.snapshot().unwrap().value, expected);
    }

    #[test]
    fn rejects_trailing_commas_and_stays_failed() {
        let mut stream = JsonStream::new();
        let first = stream.write(r#"{"x":1,}"#).unwrap_err();
        assert_eq!(stream.write(" ").unwrap_err(), first);
        assert!(!stream.is_complete());
    }
}
