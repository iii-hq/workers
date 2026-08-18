//! WHATWG HTML5 tokenizer state machine.
//!
//! This module implements the tokenization stage of the HTML parsing algorithm
//! as defined in the WHATWG HTML Living Standard.
//!
//! See <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>

use std::borrow::Cow;
use std::collections::VecDeque;

use crate::html5::entities::lookup_entity;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single token produced by the HTML5 tokenizer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A DOCTYPE token.
    Doctype {
        /// The DOCTYPE name (e.g. `html`).
        name: Option<String>,
        /// The public identifier, if any.
        public_id: Option<String>,
        /// The system identifier, if any.
        system_id: Option<String>,
        /// Whether the force-quirks flag is set.
        force_quirks: bool,
    },
    /// A start tag token.
    StartTag {
        /// The tag name.
        name: String,
        /// The list of attributes.
        attributes: Vec<Attribute>,
        /// Whether the self-closing flag is set.
        self_closing: bool,
    },
    /// An end tag token.
    EndTag {
        /// The tag name.
        name: String,
    },
    /// A single character token.
    Character(char),
    /// A comment token.
    Comment(String),
    /// End-of-file token.
    Eof,
}

/// An attribute on a start tag token.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// The attribute name.
    pub name: String,
    /// The attribute value.
    pub value: String,
}

/// A tokenizer error with a WHATWG-specified error code and byte position.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenizerError {
    /// The WHATWG error code string (e.g. `"eof-in-doctype"`).
    pub code: &'static str,
    /// Byte offset in the input where the error occurred.
    pub span: usize,
}

// ---------------------------------------------------------------------------
// Tokenizer states
// ---------------------------------------------------------------------------

/// All states of the WHATWG HTML tokenizer state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::doc_markdown, dead_code)]
pub enum State {
    Data,
    RcData,
    RawText,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RcDataLessThanSign,
    RcDataEndTagOpen,
    RcDataEndTagName,
    RawTextLessThanSign,
    RawTextEndTagOpen,
    RawTextEndTagName,
    ScriptDataLessThanSign,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThanSign,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThanSign,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDoubleQuoted,
    DoctypePublicIdentifierSingleQuoted,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDoubleQuoted,
    DoctypeSystemIdentifierSingleQuoted,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    DecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// The WHATWG HTML5 tokenizer.
///
/// Converts an input string into a sequence of [`Token`] values by walking
/// through the state machine described in the WHATWG specification.
#[allow(clippy::struct_excessive_bools)]
pub struct Tokenizer<'a> {
    input: Cow<'a, str>,
    pos: usize,
    state: State,
    return_state: State,
    // Current tag being built
    current_tag_name: String,
    current_tag_attrs: Vec<Attribute>,
    current_tag_self_closing: bool,
    current_tag_is_end: bool,
    current_attr_name: String,
    current_attr_value: String,
    // Current comment being built
    current_comment: String,
    // Current DOCTYPE being built
    current_doctype_name: Option<String>,
    current_doctype_public_id: Option<String>,
    current_doctype_system_id: Option<String>,
    current_doctype_force_quirks: bool,
    // Temporary buffer (character references, end-tag matching)
    temp_buffer: String,
    // Output queue – tokens waiting to be returned (FIFO)
    pending_tokens: VecDeque<Token>,
    // Error tracking
    errors: Vec<TokenizerError>,
    // Last emitted start tag name (for appropriate end tag checks)
    last_start_tag_name: Option<String>,
    // Character reference accumulator
    char_ref_code: u32,
    // Whether the adjusted current node is in a foreign (non-HTML) namespace.
    // Set by the tree builder; controls CDATA section handling.
    allow_cdata: bool,
}

impl<'a> Tokenizer<'a> {
    /// Creates a new tokenizer for the given input string.
    ///
    /// The input is preprocessed to normalize newlines per the WHATWG spec:
    /// CR (U+000D) and CR+LF pairs are replaced with LF (U+000A).
    pub fn new(input: &'a str) -> Self {
        let input = if input.contains('\r') {
            Cow::Owned(normalize_newlines(input))
        } else {
            Cow::Borrowed(input)
        };
        Self {
            input,
            pos: 0,
            state: State::Data,
            return_state: State::Data,
            current_tag_name: String::new(),
            current_tag_attrs: Vec::new(),
            current_tag_self_closing: false,
            current_tag_is_end: false,
            current_attr_name: String::new(),
            current_attr_value: String::new(),
            current_comment: String::new(),
            current_doctype_name: None,
            current_doctype_public_id: None,
            current_doctype_system_id: None,
            current_doctype_force_quirks: false,
            temp_buffer: String::new(),
            pending_tokens: VecDeque::new(),
            errors: Vec::new(),
            last_start_tag_name: None,
            char_ref_code: 0,
            allow_cdata: false,
        }
    }

    /// Returns a reference to the errors collected so far.
    pub fn errors(&self) -> &[TokenizerError] {
        &self.errors
    }

    /// Allows the tree builder to switch the tokenizer state (e.g. to
    /// `RcData` or `RawText` when entering `<textarea>` or `<style>`).
    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    /// Sets the tokenizer state from a string name (for test harnesses).
    ///
    /// Accepted names: `"Data"`, `"Plaintext"`, `"RcData"`, `"RawText"`,
    /// `"ScriptData"`, `"CDataSection"`. Unknown names are ignored.
    pub fn set_state_for_test(&mut self, name: &str) {
        let state = match name {
            "Data" => State::Data,
            "Plaintext" => State::Plaintext,
            "RcData" => State::RcData,
            "RawText" => State::RawText,
            "ScriptData" => State::ScriptData,
            "CDataSection" => State::CdataSection,
            _ => return,
        };
        self.state = state;
    }

    /// Sets whether the adjusted current node is in a foreign (non-HTML)
    /// namespace. When `true`, the tokenizer will handle `<![CDATA[` as a
    /// CDATA section; when `false`, it will be treated as a bogus comment.
    pub fn set_allow_cdata(&mut self, allow: bool) {
        self.allow_cdata = allow;
    }

    /// Allows the tree builder to inform the tokenizer of the last emitted
    /// start tag name, so the tokenizer can match appropriate end tags in
    /// RCDATA/RAWTEXT/Script data states.
    pub fn set_last_start_tag(&mut self, name: &str) {
        self.last_start_tag_name = Some(name.to_string());
    }

    /// Returns the next token from the input.
    ///
    /// Returns [`Token::Eof`] when the input is exhausted.
    #[allow(clippy::too_many_lines)]
    pub fn next_token(&mut self) -> Token {
        // Drain pending queue first (character reference expansions, etc.)
        if let Some(tok) = self.pending_tokens.pop_front() {
            return tok;
        }
        loop {
            if let Some(tok) = self.pending_tokens.pop_front() {
                return tok;
            }
            match self.state {
                State::Data => self.state_data(),
                State::RcData => self.state_rcdata(),
                State::RawText => self.state_rawtext(),
                State::ScriptData => self.state_script_data(),
                State::Plaintext => self.state_plaintext(),
                State::TagOpen => self.state_tag_open(),
                State::EndTagOpen => self.state_end_tag_open(),
                State::TagName => self.state_tag_name(),
                State::RcDataLessThanSign => self.state_rcdata_less_than_sign(),
                State::RcDataEndTagOpen => self.state_rcdata_end_tag_open(),
                State::RcDataEndTagName => self.state_rcdata_end_tag_name(),
                State::RawTextLessThanSign => self.state_rawtext_less_than_sign(),
                State::RawTextEndTagOpen => self.state_rawtext_end_tag_open(),
                State::RawTextEndTagName => self.state_rawtext_end_tag_name(),
                State::ScriptDataLessThanSign => self.state_script_data_less_than_sign(),
                State::ScriptDataEndTagOpen => self.state_script_data_end_tag_open(),
                State::ScriptDataEndTagName => self.state_script_data_end_tag_name(),
                State::ScriptDataEscapeStart => self.state_script_data_escape_start(),
                State::ScriptDataEscapeStartDash => {
                    self.state_script_data_escape_start_dash();
                }
                State::ScriptDataEscaped => self.state_script_data_escaped(),
                State::ScriptDataEscapedDash => self.state_script_data_escaped_dash(),
                State::ScriptDataEscapedDashDash => {
                    self.state_script_data_escaped_dash_dash();
                }
                State::ScriptDataEscapedLessThanSign => {
                    self.state_script_data_escaped_less_than_sign();
                }
                State::ScriptDataEscapedEndTagOpen => {
                    self.state_script_data_escaped_end_tag_open();
                }
                State::ScriptDataEscapedEndTagName => {
                    self.state_script_data_escaped_end_tag_name();
                }
                State::ScriptDataDoubleEscapeStart => {
                    self.state_script_data_double_escape_start();
                }
                State::ScriptDataDoubleEscaped => {
                    self.state_script_data_double_escaped();
                }
                State::ScriptDataDoubleEscapedDash => {
                    self.state_script_data_double_escaped_dash();
                }
                State::ScriptDataDoubleEscapedDashDash => {
                    self.state_script_data_double_escaped_dash_dash();
                }
                State::ScriptDataDoubleEscapedLessThanSign => {
                    self.state_script_data_double_escaped_less_than_sign();
                }
                State::ScriptDataDoubleEscapeEnd => {
                    self.state_script_data_double_escape_end();
                }
                State::BeforeAttributeName => self.state_before_attribute_name(),
                State::AttributeName => self.state_attribute_name(),
                State::AfterAttributeName => self.state_after_attribute_name(),
                State::BeforeAttributeValue => self.state_before_attribute_value(),
                State::AttributeValueDoubleQuoted => {
                    self.state_attribute_value_double_quoted();
                }
                State::AttributeValueSingleQuoted => {
                    self.state_attribute_value_single_quoted();
                }
                State::AttributeValueUnquoted => self.state_attribute_value_unquoted(),
                State::AfterAttributeValueQuoted => {
                    self.state_after_attribute_value_quoted();
                }
                State::SelfClosingStartTag => self.state_self_closing_start_tag(),
                State::BogusComment => self.state_bogus_comment(),
                State::MarkupDeclarationOpen => self.state_markup_declaration_open(),
                State::CommentStart => self.state_comment_start(),
                State::CommentStartDash => self.state_comment_start_dash(),
                State::Comment => self.state_comment(),
                State::CommentLessThanSign => self.state_comment_less_than_sign(),
                State::CommentLessThanSignBang => {
                    self.state_comment_less_than_sign_bang();
                }
                State::CommentLessThanSignBangDash => {
                    self.state_comment_less_than_sign_bang_dash();
                }
                State::CommentLessThanSignBangDashDash => {
                    self.state_comment_less_than_sign_bang_dash_dash();
                }
                State::CommentEndDash => self.state_comment_end_dash(),
                State::CommentEnd => self.state_comment_end(),
                State::CommentEndBang => self.state_comment_end_bang(),
                State::Doctype => self.state_doctype(),
                State::BeforeDoctypeName => self.state_before_doctype_name(),
                State::DoctypeName => self.state_doctype_name(),
                State::AfterDoctypeName => self.state_after_doctype_name(),
                State::AfterDoctypePublicKeyword => {
                    self.state_after_doctype_public_keyword();
                }
                State::BeforeDoctypePublicIdentifier => {
                    self.state_before_doctype_public_identifier();
                }
                State::DoctypePublicIdentifierDoubleQuoted => {
                    self.state_doctype_public_identifier_double_quoted();
                }
                State::DoctypePublicIdentifierSingleQuoted => {
                    self.state_doctype_public_identifier_single_quoted();
                }
                State::AfterDoctypePublicIdentifier => {
                    self.state_after_doctype_public_identifier();
                }
                State::BetweenDoctypePublicAndSystemIdentifiers => {
                    self.state_between_doctype_public_and_system_identifiers();
                }
                State::AfterDoctypeSystemKeyword => {
                    self.state_after_doctype_system_keyword();
                }
                State::BeforeDoctypeSystemIdentifier => {
                    self.state_before_doctype_system_identifier();
                }
                State::DoctypeSystemIdentifierDoubleQuoted => {
                    self.state_doctype_system_identifier_double_quoted();
                }
                State::DoctypeSystemIdentifierSingleQuoted => {
                    self.state_doctype_system_identifier_single_quoted();
                }
                State::AfterDoctypeSystemIdentifier => {
                    self.state_after_doctype_system_identifier();
                }
                State::BogusDoctype => self.state_bogus_doctype(),
                State::CdataSection => self.state_cdata_section(),
                State::CdataSectionBracket => self.state_cdata_section_bracket(),
                State::CdataSectionEnd => self.state_cdata_section_end(),
                State::CharacterReference => self.state_character_reference(),
                State::NamedCharacterReference => {
                    self.state_named_character_reference();
                }
                State::AmbiguousAmpersand => self.state_ambiguous_ampersand(),
                State::NumericCharacterReference => {
                    self.state_numeric_character_reference();
                }
                State::HexadecimalCharacterReferenceStart => {
                    self.state_hexadecimal_character_reference_start();
                }
                State::DecimalCharacterReferenceStart => {
                    self.state_decimal_character_reference_start();
                }
                State::HexadecimalCharacterReference => {
                    self.state_hexadecimal_character_reference();
                }
                State::DecimalCharacterReference => {
                    self.state_decimal_character_reference();
                }
                State::NumericCharacterReferenceEnd => {
                    self.state_numeric_character_reference_end();
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Peek at the next character without consuming it.
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Consume and return the next character, advancing `pos`.
    fn consume(&mut self) -> Option<char> {
        let ch = self.input[self.pos..].chars().next()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    /// Reconsume: back up by the byte-length of the given character.
    fn reconsume(&mut self, ch: char) {
        self.pos -= ch.len_utf8();
    }

    /// Check if the upcoming input (case-insensitively) matches `needle`,
    /// without consuming. `needle` must be ASCII.
    fn next_chars_are_ascii_ci(&self, needle: &str) -> bool {
        let remaining = self.input.as_bytes();
        if self.pos + needle.len() > remaining.len() {
            return false;
        }
        remaining[self.pos..self.pos + needle.len()].eq_ignore_ascii_case(needle.as_bytes())
    }

    /// Push a parse error.
    fn emit_error(&mut self, code: &'static str) {
        self.errors.push(TokenizerError {
            code,
            span: self.pos,
        });
    }

    /// Emit a character token (pushes to pending queue).
    fn emit_char(&mut self, ch: char) {
        self.pending_tokens.push_back(Token::Character(ch));
    }

    /// Emit EOF.
    fn emit_eof(&mut self) {
        self.pending_tokens.push_back(Token::Eof);
    }

    /// Emit the current comment token.
    fn emit_comment(&mut self) {
        let comment = std::mem::take(&mut self.current_comment);
        self.pending_tokens.push_back(Token::Comment(comment));
    }

    /// Emit the current tag token (start or end).
    fn emit_current_tag(&mut self) {
        self.finish_current_attr();
        if self.current_tag_is_end {
            self.pending_tokens.push_back(Token::EndTag {
                name: std::mem::take(&mut self.current_tag_name),
            });
        } else {
            let name = std::mem::take(&mut self.current_tag_name);
            self.last_start_tag_name = Some(name.clone());
            self.pending_tokens.push_back(Token::StartTag {
                name,
                attributes: std::mem::take(&mut self.current_tag_attrs),
                self_closing: self.current_tag_self_closing,
            });
        }
        self.current_tag_self_closing = false;
    }

    /// Emit the current DOCTYPE token.
    fn emit_doctype(&mut self) {
        self.pending_tokens.push_back(Token::Doctype {
            name: self.current_doctype_name.take(),
            public_id: self.current_doctype_public_id.take(),
            system_id: self.current_doctype_system_id.take(),
            force_quirks: self.current_doctype_force_quirks,
        });
        self.current_doctype_force_quirks = false;
    }

    /// Start building a new start-tag token.
    fn create_start_tag(&mut self) {
        self.current_tag_name.clear();
        self.current_tag_attrs.clear();
        self.current_tag_self_closing = false;
        self.current_tag_is_end = false;
        self.current_attr_name.clear();
        self.current_attr_value.clear();
    }

    /// Start building a new end-tag token.
    fn create_end_tag(&mut self) {
        self.current_tag_name.clear();
        self.current_tag_attrs.clear();
        self.current_tag_self_closing = false;
        self.current_tag_is_end = true;
        self.current_attr_name.clear();
        self.current_attr_value.clear();
    }

    /// Start a new attribute on the current tag.
    fn start_new_attr(&mut self) {
        self.finish_current_attr();
        self.current_attr_name.clear();
        self.current_attr_value.clear();
    }

    /// Finish the current attribute (push it to the tag's attribute list
    /// if the name is non-empty and not a duplicate).
    fn finish_current_attr(&mut self) {
        if self.current_attr_name.is_empty() {
            return;
        }
        let name = std::mem::take(&mut self.current_attr_name);
        let value = std::mem::take(&mut self.current_attr_value);
        // The spec says duplicate attributes are parse errors; keep first.
        if !self.current_tag_attrs.iter().any(|a| a.name == name) {
            self.current_tag_attrs.push(Attribute { name, value });
        }
    }

    /// Create a new DOCTYPE token with all fields empty.
    fn create_doctype(&mut self) {
        self.current_doctype_name = None;
        self.current_doctype_public_id = None;
        self.current_doctype_system_id = None;
        self.current_doctype_force_quirks = false;
    }

    /// Check if the current end tag is an appropriate end tag
    /// (its name matches the last emitted start tag name).
    fn is_appropriate_end_tag(&self) -> bool {
        if let Some(ref last) = self.last_start_tag_name {
            *last == self.current_tag_name
        } else {
            false
        }
    }

    /// Flush code points consumed as a character reference.
    ///
    /// If the return state is an attribute value state, append `temp_buffer`
    /// to the current attribute value; otherwise emit each character.
    fn flush_code_points_consumed_as_char_ref(&mut self) {
        let buf = std::mem::take(&mut self.temp_buffer);
        if is_attr_value_state(self.return_state) {
            self.current_attr_value.push_str(&buf);
        } else {
            // Emit each char individually.
            for ch in buf.chars() {
                self.pending_tokens.push_back(Token::Character(ch));
            }
        }
    }

    // -----------------------------------------------------------------------
    // State implementations
    // -----------------------------------------------------------------------

    // 13.2.5.1 Data state
    fn state_data(&mut self) {
        // Fast path: scan forward through bytes that don't need special
        // handling (not '<', '&', or '\0'). This avoids per-character
        // overhead for plain text runs.
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'<' || b == b'&' || b == 0 {
                break;
            }
            i += 1;
        }
        if i > start {
            // All bytes in start..i are safe plain text (no null, no < or &).
            // Because we only break on ASCII bytes and skip non-ASCII bytes,
            // start..i is always a valid UTF-8 slice boundary.
            for c in self.input[start..i].chars() {
                self.pending_tokens.push_back(Token::Character(c));
            }
            self.pos = i;
            return;
        }

        // Slow path: handle special characters one at a time.
        match self.consume() {
            Some('&') => {
                self.return_state = State::Data;
                self.state = State::CharacterReference;
            }
            Some('<') => {
                self.state = State::TagOpen;
            }
            Some('\0') => {
                // Per spec: emit the null character as-is (with a parse error).
                // Unlike RCDATA/RAWTEXT, Data state does NOT replace with U+FFFD.
                self.emit_error("unexpected-null-character");
                self.emit_char('\0');
            }
            None => {
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.2 RCDATA state
    fn state_rcdata(&mut self) {
        match self.consume() {
            Some('&') => {
                self.return_state = State::RcData;
                self.state = State::CharacterReference;
            }
            Some('<') => {
                self.state = State::RcDataLessThanSign;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.3 RAWTEXT state
    fn state_rawtext(&mut self) {
        match self.consume() {
            Some('<') => {
                self.state = State::RawTextLessThanSign;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.4 Script data state
    fn state_script_data(&mut self) {
        match self.consume() {
            Some('<') => {
                self.state = State::ScriptDataLessThanSign;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.5 PLAINTEXT state
    fn state_plaintext(&mut self) {
        match self.consume() {
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.6 Tag open state
    fn state_tag_open(&mut self) {
        match self.consume() {
            Some('!') => {
                self.state = State::MarkupDeclarationOpen;
            }
            Some('/') => {
                self.state = State::EndTagOpen;
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.create_start_tag();
                self.reconsume(c);
                self.state = State::TagName;
            }
            Some('?') => {
                self.emit_error("unexpected-question-mark-instead-of-tag-name");
                self.current_comment.clear();
                self.reconsume('?');
                self.state = State::BogusComment;
            }
            None => {
                self.emit_error("eof-before-tag-name");
                self.emit_char('<');
                self.emit_eof();
            }
            Some(c) => {
                self.emit_error("invalid-first-character-of-tag-name");
                self.reconsume(c);
                self.state = State::Data;
                self.emit_char('<');
            }
        }
    }

    // 13.2.5.7 End tag open state
    fn state_end_tag_open(&mut self) {
        match self.consume() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.create_end_tag();
                self.reconsume(c);
                self.state = State::TagName;
            }
            Some('>') => {
                self.emit_error("missing-end-tag-name");
                self.state = State::Data;
            }
            None => {
                self.emit_error("eof-before-tag-name");
                self.emit_char('<');
                self.emit_char('/');
                self.emit_eof();
            }
            Some(c) => {
                self.emit_error("invalid-first-character-of-tag-name");
                self.current_comment.clear();
                self.reconsume(c);
                self.state = State::BogusComment;
            }
        }
    }

    // 13.2.5.8 Tag name state
    fn state_tag_name(&mut self) {
        // Fast path: scan ahead through ASCII lowercase tag-name characters.
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            match b {
                b'\t' | b'\n' | 0x0C | b' ' | b'/' | b'>' | 0 | 0x80..=0xFF => break,
                b'A'..=b'Z' => {
                    self.current_tag_name.push((b + 32) as char);
                    i += 1;
                }
                _ => {
                    self.current_tag_name.push(b as char);
                    i += 1;
                }
            }
        }
        self.pos = i;

        // Now handle the terminating character.
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BeforeAttributeName;
            }
            Some('/') => {
                self.state = State::SelfClosingStartTag;
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_current_tag();
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_tag_name.push('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.current_tag_name.push(c.to_ascii_lowercase());
            }
        }
    }

    // 13.2.5.9 RCDATA less-than sign state
    fn state_rcdata_less_than_sign(&mut self) {
        if let Some('/') = self.peek() {
            self.consume();
            self.temp_buffer.clear();
            self.state = State::RcDataEndTagOpen;
        } else {
            self.state = State::RcData;
            self.emit_char('<');
        }
    }

    // 13.2.5.10 RCDATA end tag open state
    fn state_rcdata_end_tag_open(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.create_end_tag();
                self.state = State::RcDataEndTagName;
            }
            _ => {
                self.state = State::RcData;
                self.emit_char('<');
                self.emit_char('/');
            }
        }
    }

    // 13.2.5.11 RCDATA end tag name state
    fn state_rcdata_end_tag_name(&mut self) {
        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ')) => {
                if self.is_appropriate_end_tag() {
                    self.state = State::BeforeAttributeName;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume(c);
                    self.state = State::RcData;
                }
            }
            Some('/') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::SelfClosingStartTag;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('/');
                    self.state = State::RcData;
                }
            }
            Some('>') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::Data;
                    self.emit_current_tag();
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('>');
                    self.state = State::RcData;
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.current_tag_name.push(c.to_ascii_lowercase());
                self.temp_buffer.push(c);
            }
            None => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.state = State::RcData;
            }
            Some(c) => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.reconsume(c);
                self.state = State::RcData;
            }
        }
    }

    /// Emit each character in `temp_buffer` as a character token.
    fn emit_temp_buffer_chars(&mut self) {
        let buf = std::mem::take(&mut self.temp_buffer);
        for ch in buf.chars() {
            self.emit_char(ch);
        }
    }

    // 13.2.5.12 RAWTEXT less-than sign state
    fn state_rawtext_less_than_sign(&mut self) {
        if let Some('/') = self.peek() {
            self.consume();
            self.temp_buffer.clear();
            self.state = State::RawTextEndTagOpen;
        } else {
            self.state = State::RawText;
            self.emit_char('<');
        }
    }

    // 13.2.5.13 RAWTEXT end tag open state
    fn state_rawtext_end_tag_open(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.create_end_tag();
                self.state = State::RawTextEndTagName;
            }
            _ => {
                self.state = State::RawText;
                self.emit_char('<');
                self.emit_char('/');
            }
        }
    }

    // 13.2.5.14 RAWTEXT end tag name state
    fn state_rawtext_end_tag_name(&mut self) {
        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ')) => {
                if self.is_appropriate_end_tag() {
                    self.state = State::BeforeAttributeName;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume(c);
                    self.state = State::RawText;
                }
            }
            Some('/') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::SelfClosingStartTag;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('/');
                    self.state = State::RawText;
                }
            }
            Some('>') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::Data;
                    self.emit_current_tag();
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('>');
                    self.state = State::RawText;
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.current_tag_name.push(c.to_ascii_lowercase());
                self.temp_buffer.push(c);
            }
            None => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.state = State::RawText;
            }
            Some(c) => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.reconsume(c);
                self.state = State::RawText;
            }
        }
    }

    // 13.2.5.15 Script data less-than sign state
    fn state_script_data_less_than_sign(&mut self) {
        match self.peek() {
            Some('/') => {
                self.consume();
                self.temp_buffer.clear();
                self.state = State::ScriptDataEndTagOpen;
            }
            Some('!') => {
                self.consume();
                self.state = State::ScriptDataEscapeStart;
                self.emit_char('<');
                self.emit_char('!');
            }
            _ => {
                self.state = State::ScriptData;
                self.emit_char('<');
            }
        }
    }

    // 13.2.5.16 Script data end tag open state
    fn state_script_data_end_tag_open(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.create_end_tag();
                self.state = State::ScriptDataEndTagName;
            }
            _ => {
                self.state = State::ScriptData;
                self.emit_char('<');
                self.emit_char('/');
            }
        }
    }

    // 13.2.5.17 Script data end tag name state
    fn state_script_data_end_tag_name(&mut self) {
        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ')) => {
                if self.is_appropriate_end_tag() {
                    self.state = State::BeforeAttributeName;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume(c);
                    self.state = State::ScriptData;
                }
            }
            Some('/') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::SelfClosingStartTag;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('/');
                    self.state = State::ScriptData;
                }
            }
            Some('>') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::Data;
                    self.emit_current_tag();
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('>');
                    self.state = State::ScriptData;
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.current_tag_name.push(c.to_ascii_lowercase());
                self.temp_buffer.push(c);
            }
            None => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.state = State::ScriptData;
            }
            Some(c) => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.reconsume(c);
                self.state = State::ScriptData;
            }
        }
    }

    // 13.2.5.18 Script data escape start state
    fn state_script_data_escape_start(&mut self) {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::ScriptDataEscapeStartDash;
                self.emit_char('-');
            }
            _ => {
                self.state = State::ScriptData;
            }
        }
    }

    // 13.2.5.19 Script data escape start dash state
    fn state_script_data_escape_start_dash(&mut self) {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::ScriptDataEscapedDashDash;
                self.emit_char('-');
            }
            _ => {
                self.state = State::ScriptData;
            }
        }
    }

    // 13.2.5.20 Script data escaped state
    fn state_script_data_escaped(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::ScriptDataEscapedDash;
                self.emit_char('-');
            }
            Some('<') => {
                self.state = State::ScriptDataEscapedLessThanSign;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-script-html-comment-like-text");
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.21 Script data escaped dash state
    fn state_script_data_escaped_dash(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::ScriptDataEscapedDashDash;
                self.emit_char('-');
            }
            Some('<') => {
                self.state = State::ScriptDataEscapedLessThanSign;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.state = State::ScriptDataEscaped;
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-script-html-comment-like-text");
                self.emit_eof();
            }
            Some(c) => {
                self.state = State::ScriptDataEscaped;
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.22 Script data escaped dash dash state
    fn state_script_data_escaped_dash_dash(&mut self) {
        match self.consume() {
            Some('-') => {
                self.emit_char('-');
            }
            Some('<') => {
                self.state = State::ScriptDataEscapedLessThanSign;
            }
            Some('>') => {
                self.state = State::ScriptData;
                self.emit_char('>');
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.state = State::ScriptDataEscaped;
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-script-html-comment-like-text");
                self.emit_eof();
            }
            Some(c) => {
                self.state = State::ScriptDataEscaped;
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.23 Script data escaped less-than sign state
    fn state_script_data_escaped_less_than_sign(&mut self) {
        match self.peek() {
            Some('/') => {
                self.consume();
                self.temp_buffer.clear();
                self.state = State::ScriptDataEscapedEndTagOpen;
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.temp_buffer.clear();
                self.emit_char('<');
                self.state = State::ScriptDataDoubleEscapeStart;
            }
            _ => {
                self.emit_char('<');
                self.state = State::ScriptDataEscaped;
            }
        }
    }

    // 13.2.5.24 Script data escaped end tag open state
    fn state_script_data_escaped_end_tag_open(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                self.create_end_tag();
                self.state = State::ScriptDataEscapedEndTagName;
            }
            _ => {
                self.emit_char('<');
                self.emit_char('/');
                self.state = State::ScriptDataEscaped;
            }
        }
    }

    // 13.2.5.25 Script data escaped end tag name state
    fn state_script_data_escaped_end_tag_name(&mut self) {
        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ')) => {
                if self.is_appropriate_end_tag() {
                    self.state = State::BeforeAttributeName;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume(c);
                    self.state = State::ScriptDataEscaped;
                }
            }
            Some('/') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::SelfClosingStartTag;
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('/');
                    self.state = State::ScriptDataEscaped;
                }
            }
            Some('>') => {
                if self.is_appropriate_end_tag() {
                    self.state = State::Data;
                    self.emit_current_tag();
                } else {
                    self.emit_char('<');
                    self.emit_char('/');
                    self.emit_temp_buffer_chars();
                    self.reconsume('>');
                    self.state = State::ScriptDataEscaped;
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.current_tag_name.push(c.to_ascii_lowercase());
                self.temp_buffer.push(c);
            }
            None => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.state = State::ScriptDataEscaped;
            }
            Some(c) => {
                self.emit_char('<');
                self.emit_char('/');
                self.emit_temp_buffer_chars();
                self.reconsume(c);
                self.state = State::ScriptDataEscaped;
            }
        }
    }

    // 13.2.5.26 Script data double escape start state
    fn state_script_data_double_escape_start(&mut self) {
        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ' | '/' | '>')) => {
                if self.temp_buffer == "script" {
                    self.state = State::ScriptDataDoubleEscaped;
                } else {
                    self.state = State::ScriptDataEscaped;
                }
                self.emit_char(c);
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.temp_buffer.push(c.to_ascii_lowercase());
                self.emit_char(c);
            }
            _ => {
                if let Some(c) = self.input[self.pos..].chars().next() {
                    self.reconsume(c);
                }
                self.state = State::ScriptDataEscaped;
            }
        }
    }

    // 13.2.5.27 Script data double escaped state
    fn state_script_data_double_escaped(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::ScriptDataDoubleEscapedDash;
                self.emit_char('-');
            }
            Some('<') => {
                self.state = State::ScriptDataDoubleEscapedLessThanSign;
                self.emit_char('<');
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-script-html-comment-like-text");
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.28 Script data double escaped dash state
    fn state_script_data_double_escaped_dash(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::ScriptDataDoubleEscapedDashDash;
                self.emit_char('-');
            }
            Some('<') => {
                self.state = State::ScriptDataDoubleEscapedLessThanSign;
                self.emit_char('<');
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-script-html-comment-like-text");
                self.emit_eof();
            }
            Some(c) => {
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.29 Script data double escaped dash dash state
    fn state_script_data_double_escaped_dash_dash(&mut self) {
        match self.consume() {
            Some('-') => {
                self.emit_char('-');
            }
            Some('<') => {
                self.state = State::ScriptDataDoubleEscapedLessThanSign;
                self.emit_char('<');
            }
            Some('>') => {
                self.state = State::ScriptData;
                self.emit_char('>');
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_char('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-script-html-comment-like-text");
                self.emit_eof();
            }
            Some(c) => {
                self.state = State::ScriptDataDoubleEscaped;
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.30 Script data double escaped less-than sign state
    fn state_script_data_double_escaped_less_than_sign(&mut self) {
        match self.peek() {
            Some('/') => {
                self.consume();
                self.temp_buffer.clear();
                self.state = State::ScriptDataDoubleEscapeEnd;
                self.emit_char('/');
            }
            _ => {
                self.state = State::ScriptDataDoubleEscaped;
            }
        }
    }

    // 13.2.5.31 Script data double escape end state
    fn state_script_data_double_escape_end(&mut self) {
        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ' | '/' | '>')) => {
                if self.temp_buffer == "script" {
                    self.state = State::ScriptDataEscaped;
                } else {
                    self.state = State::ScriptDataDoubleEscaped;
                }
                self.emit_char(c);
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.temp_buffer.push(c.to_ascii_lowercase());
                self.emit_char(c);
            }
            _ => {
                if let Some(c) = self.input[self.pos..].chars().next() {
                    self.reconsume(c);
                }
                self.state = State::ScriptDataDoubleEscaped;
            }
        }
    }

    // 13.2.5.32 Before attribute name state
    fn state_before_attribute_name(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore whitespace.
            }
            Some('/' | '>') | None => {
                if let Some(c) = self.input[self.pos.saturating_sub(1)..].chars().next() {
                    if c == '/' || c == '>' {
                        self.reconsume(c);
                    }
                }
                if self.pos == self.input.len() {
                    // EOF – reconsume handled by AfterAttributeName
                }
                self.state = State::AfterAttributeName;
            }
            Some('=') => {
                self.emit_error("unexpected-equals-sign-before-attribute-name");
                self.start_new_attr();
                self.current_attr_name.push('=');
                self.state = State::AttributeName;
            }
            Some(c) => {
                self.start_new_attr();
                self.reconsume(c);
                self.state = State::AttributeName;
            }
        }
    }

    // 13.2.5.33 Attribute name state
    fn state_attribute_name(&mut self) {
        // Fast path: scan ahead for ASCII lowercase attribute name bytes.
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            match b {
                b'\t'
                | b'\n'
                | 0x0C
                | b' '
                | b'/'
                | b'>'
                | b'='
                | 0
                | b'"'
                | b'\''
                | b'<'
                | 0x80..=0xFF => break,
                b'A'..=b'Z' => {
                    self.current_attr_name.push((b + 32) as char);
                    i += 1;
                }
                _ => {
                    self.current_attr_name.push(b as char);
                    i += 1;
                }
            }
        }
        self.pos = i;

        match self.consume() {
            Some(c @ ('\t' | '\n' | '\x0C' | ' ' | '/' | '>')) => {
                self.reconsume(c);
                self.state = State::AfterAttributeName;
            }
            Some('=') => {
                self.state = State::BeforeAttributeValue;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_attr_name.push('\u{FFFD}');
            }
            Some(c @ ('"' | '\'' | '<')) => {
                self.emit_error("unexpected-character-in-attribute-name");
                self.current_attr_name.push(c);
            }
            None => {
                self.reconsume('\0'); // will be handled by after-attr
                self.pos = self.input.len(); // stay at EOF
                self.state = State::AfterAttributeName;
            }
            Some(c) => {
                self.current_attr_name.push(c.to_ascii_lowercase());
            }
        }
    }

    // 13.2.5.34 After attribute name state
    fn state_after_attribute_name(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('/') => {
                self.state = State::SelfClosingStartTag;
            }
            Some('=') => {
                self.state = State::BeforeAttributeValue;
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_current_tag();
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.start_new_attr();
                self.reconsume(c);
                self.state = State::AttributeName;
            }
        }
    }

    // 13.2.5.35 Before attribute value state
    fn state_before_attribute_value(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('"') => {
                self.state = State::AttributeValueDoubleQuoted;
            }
            Some('\'') => {
                self.state = State::AttributeValueSingleQuoted;
            }
            Some('>') => {
                self.emit_error("missing-attribute-value");
                self.state = State::Data;
                self.emit_current_tag();
            }
            Some(c) => {
                self.reconsume(c);
                self.state = State::AttributeValueUnquoted;
            }
            None => {
                self.reconsume('\0');
                self.pos = self.input.len();
                self.state = State::AttributeValueUnquoted;
            }
        }
    }

    // 13.2.5.36 Attribute value (double-quoted) state
    fn state_attribute_value_double_quoted(&mut self) {
        // Fast path: scan ahead for plain attribute value bytes (no ", &, \0).
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'"' | b'&' | 0 => break,
                _ => i += 1,
            }
        }
        if i > start {
            self.current_attr_value.push_str(&self.input[start..i]);
            self.pos = i;
        }

        match self.consume() {
            Some('"') => {
                self.state = State::AfterAttributeValueQuoted;
            }
            Some('&') => {
                self.return_state = State::AttributeValueDoubleQuoted;
                self.state = State::CharacterReference;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_attr_value.push('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.current_attr_value.push(c);
            }
        }
    }

    // 13.2.5.37 Attribute value (single-quoted) state
    fn state_attribute_value_single_quoted(&mut self) {
        // Fast path: scan ahead for plain attribute value bytes (no ', &, \0).
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'\'' | b'&' | 0 => break,
                _ => i += 1,
            }
        }
        if i > start {
            self.current_attr_value.push_str(&self.input[start..i]);
            self.pos = i;
        }

        match self.consume() {
            Some('\'') => {
                self.state = State::AfterAttributeValueQuoted;
            }
            Some('&') => {
                self.return_state = State::AttributeValueSingleQuoted;
                self.state = State::CharacterReference;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_attr_value.push('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.current_attr_value.push(c);
            }
        }
    }

    // 13.2.5.38 Attribute value (unquoted) state
    fn state_attribute_value_unquoted(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BeforeAttributeName;
            }
            Some('&') => {
                self.return_state = State::AttributeValueUnquoted;
                self.state = State::CharacterReference;
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_current_tag();
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_attr_value.push('\u{FFFD}');
            }
            Some(c @ ('"' | '\'' | '<' | '=' | '`')) => {
                self.emit_error("unexpected-character-in-unquoted-attribute-value");
                self.current_attr_value.push(c);
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.current_attr_value.push(c);
            }
        }
    }

    // 13.2.5.39 After attribute value (quoted) state
    fn state_after_attribute_value_quoted(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BeforeAttributeName;
            }
            Some('/') => {
                self.state = State::SelfClosingStartTag;
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_current_tag();
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.emit_error("missing-whitespace-between-attributes");
                self.reconsume(c);
                self.state = State::BeforeAttributeName;
            }
        }
    }

    // 13.2.5.40 Self-closing start tag state
    fn state_self_closing_start_tag(&mut self) {
        match self.consume() {
            Some('>') => {
                self.current_tag_self_closing = true;
                self.state = State::Data;
                self.emit_current_tag();
            }
            None => {
                self.emit_error("eof-in-tag");
                self.emit_eof();
            }
            Some(c) => {
                self.emit_error("unexpected-solidus-in-tag");
                self.reconsume(c);
                self.state = State::BeforeAttributeName;
            }
        }
    }

    // 13.2.5.41 Bogus comment state
    fn state_bogus_comment(&mut self) {
        match self.consume() {
            Some('>') => {
                self.state = State::Data;
                self.emit_comment();
            }
            None => {
                self.emit_comment();
                self.emit_eof();
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_comment.push('\u{FFFD}');
            }
            Some(c) => {
                self.current_comment.push(c);
            }
        }
    }

    // 13.2.5.42 Markup declaration open state
    fn state_markup_declaration_open(&mut self) {
        if self.next_chars_are_ascii_ci("--") {
            self.pos += 2;
            self.current_comment.clear();
            self.state = State::CommentStart;
        } else if self.next_chars_are_ascii_ci("DOCTYPE") {
            self.pos += 7;
            self.state = State::Doctype;
        } else if self.next_chars_are_ascii_ci("[CDATA[") {
            self.pos += 7;
            if self.allow_cdata {
                // In foreign content: treat as CDATA section.
                self.state = State::CdataSection;
            } else {
                // In HTML content: parse error, treat as bogus comment.
                self.emit_error("cdata-in-html-content");
                self.current_comment = "[CDATA[".to_string();
                self.state = State::BogusComment;
            }
        } else {
            self.emit_error("incorrectly-opened-comment");
            self.current_comment.clear();
            self.state = State::BogusComment;
        }
    }

    // 13.2.5.43 Comment start state
    fn state_comment_start(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::CommentStartDash;
            }
            Some('>') => {
                self.emit_error("abrupt-closing-of-empty-comment");
                self.state = State::Data;
                self.emit_comment();
            }
            Some(c) => {
                self.reconsume(c);
                self.state = State::Comment;
            }
            None => {
                self.reconsume('\0');
                self.pos = self.input.len();
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.44 Comment start dash state
    fn state_comment_start_dash(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::CommentEnd;
            }
            Some('>') => {
                self.emit_error("abrupt-closing-of-empty-comment");
                self.state = State::Data;
                self.emit_comment();
            }
            None => {
                self.emit_error("eof-in-comment");
                self.emit_comment();
                self.emit_eof();
            }
            Some(c) => {
                self.current_comment.push('-');
                self.reconsume(c);
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.45 Comment state
    fn state_comment(&mut self) {
        match self.consume() {
            Some('<') => {
                self.current_comment.push('<');
                self.state = State::CommentLessThanSign;
            }
            Some('-') => {
                self.state = State::CommentEndDash;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.current_comment.push('\u{FFFD}');
            }
            None => {
                self.emit_error("eof-in-comment");
                self.emit_comment();
                self.emit_eof();
            }
            Some(c) => {
                self.current_comment.push(c);
            }
        }
    }

    // 13.2.5.46 Comment less-than sign state
    fn state_comment_less_than_sign(&mut self) {
        match self.consume() {
            Some('!') => {
                self.current_comment.push('!');
                self.state = State::CommentLessThanSignBang;
            }
            Some('<') => {
                self.current_comment.push('<');
            }
            Some(c) => {
                self.reconsume(c);
                self.state = State::Comment;
            }
            None => {
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.47 Comment less-than sign bang state
    fn state_comment_less_than_sign_bang(&mut self) {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::CommentLessThanSignBangDash;
            }
            _ => {
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.48 Comment less-than sign bang dash state
    fn state_comment_less_than_sign_bang_dash(&mut self) {
        match self.peek() {
            Some('-') => {
                self.consume();
                self.state = State::CommentLessThanSignBangDashDash;
            }
            _ => {
                self.state = State::CommentEndDash;
            }
        }
    }

    // 13.2.5.49 Comment less-than sign bang dash dash state
    fn state_comment_less_than_sign_bang_dash_dash(&mut self) {
        match self.peek() {
            Some('>') | None => {
                self.state = State::CommentEnd;
            }
            _ => {
                self.emit_error("nested-comment");
                self.state = State::CommentEnd;
            }
        }
    }

    // 13.2.5.50 Comment end dash state
    fn state_comment_end_dash(&mut self) {
        match self.consume() {
            Some('-') => {
                self.state = State::CommentEnd;
            }
            None => {
                self.emit_error("eof-in-comment");
                self.emit_comment();
                self.emit_eof();
            }
            Some(c) => {
                self.current_comment.push('-');
                self.reconsume(c);
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.51 Comment end state
    fn state_comment_end(&mut self) {
        match self.consume() {
            Some('>') => {
                self.state = State::Data;
                self.emit_comment();
            }
            Some('!') => {
                self.state = State::CommentEndBang;
            }
            Some('-') => {
                self.current_comment.push('-');
            }
            None => {
                self.emit_error("eof-in-comment");
                self.emit_comment();
                self.emit_eof();
            }
            Some(c) => {
                self.current_comment.push('-');
                self.current_comment.push('-');
                self.reconsume(c);
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.52 Comment end bang state
    fn state_comment_end_bang(&mut self) {
        match self.consume() {
            Some('-') => {
                self.current_comment.push('-');
                self.current_comment.push('-');
                self.current_comment.push('!');
                self.state = State::CommentEndDash;
            }
            Some('>') => {
                self.emit_error("incorrectly-closed-comment");
                self.state = State::Data;
                self.emit_comment();
            }
            None => {
                self.emit_error("eof-in-comment");
                self.emit_comment();
                self.emit_eof();
            }
            Some(c) => {
                self.current_comment.push('-');
                self.current_comment.push('-');
                self.current_comment.push('!');
                self.reconsume(c);
                self.state = State::Comment;
            }
        }
    }

    // 13.2.5.53 DOCTYPE state
    fn state_doctype(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BeforeDoctypeName;
            }
            Some('>') => {
                self.reconsume('>');
                self.state = State::BeforeDoctypeName;
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.create_doctype();
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                self.emit_error("missing-whitespace-before-doctype-name");
                self.reconsume(c);
                self.state = State::BeforeDoctypeName;
            }
        }
    }

    // 13.2.5.54 Before DOCTYPE name state
    fn state_before_doctype_name(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                self.create_doctype();
                self.current_doctype_name = Some(String::from('\u{FFFD}'));
                self.state = State::DoctypeName;
            }
            Some('>') => {
                self.emit_error("missing-doctype-name");
                self.create_doctype();
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.create_doctype();
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                self.create_doctype();
                self.current_doctype_name = Some(String::from(c.to_ascii_lowercase()));
                self.state = State::DoctypeName;
            }
        }
    }

    // 13.2.5.55 DOCTYPE name state
    fn state_doctype_name(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::AfterDoctypeName;
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                if let Some(ref mut name) = self.current_doctype_name {
                    name.push('\u{FFFD}');
                }
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                if let Some(ref mut name) = self.current_doctype_name {
                    name.push(c.to_ascii_lowercase());
                }
            }
        }
    }

    // 13.2.5.56 After DOCTYPE name state
    fn state_after_doctype_name(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                // Check for PUBLIC or SYSTEM keywords.
                self.reconsume(c);
                if self.next_chars_are_ascii_ci("PUBLIC") {
                    self.pos += 6;
                    self.state = State::AfterDoctypePublicKeyword;
                } else if self.next_chars_are_ascii_ci("SYSTEM") {
                    self.pos += 6;
                    self.state = State::AfterDoctypeSystemKeyword;
                } else {
                    self.consume(); // re-consume the char we put back
                    self.emit_error("invalid-character-sequence-after-doctype-name");
                    self.current_doctype_force_quirks = true;
                    self.state = State::BogusDoctype;
                }
            }
        }
    }

    // 13.2.5.57 After DOCTYPE public keyword state
    fn state_after_doctype_public_keyword(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BeforeDoctypePublicIdentifier;
            }
            Some('"') => {
                self.emit_error("missing-whitespace-after-doctype-public-keyword");
                self.current_doctype_public_id = Some(String::new());
                self.state = State::DoctypePublicIdentifierDoubleQuoted;
            }
            Some('\'') => {
                self.emit_error("missing-whitespace-after-doctype-public-keyword");
                self.current_doctype_public_id = Some(String::new());
                self.state = State::DoctypePublicIdentifierSingleQuoted;
            }
            Some('>') => {
                self.emit_error("missing-doctype-public-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("missing-quote-before-doctype-public-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.58 Before DOCTYPE public identifier state
    fn state_before_doctype_public_identifier(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('"') => {
                self.current_doctype_public_id = Some(String::new());
                self.state = State::DoctypePublicIdentifierDoubleQuoted;
            }
            Some('\'') => {
                self.current_doctype_public_id = Some(String::new());
                self.state = State::DoctypePublicIdentifierSingleQuoted;
            }
            Some('>') => {
                self.emit_error("missing-doctype-public-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("missing-quote-before-doctype-public-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.59 DOCTYPE public identifier (double-quoted) state
    fn state_doctype_public_identifier_double_quoted(&mut self) {
        match self.consume() {
            Some('"') => {
                self.state = State::AfterDoctypePublicIdentifier;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                if let Some(ref mut id) = self.current_doctype_public_id {
                    id.push('\u{FFFD}');
                }
            }
            Some('>') => {
                self.emit_error("abrupt-doctype-public-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                if let Some(ref mut id) = self.current_doctype_public_id {
                    id.push(c);
                }
            }
        }
    }

    // 13.2.5.60 DOCTYPE public identifier (single-quoted) state
    fn state_doctype_public_identifier_single_quoted(&mut self) {
        match self.consume() {
            Some('\'') => {
                self.state = State::AfterDoctypePublicIdentifier;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                if let Some(ref mut id) = self.current_doctype_public_id {
                    id.push('\u{FFFD}');
                }
            }
            Some('>') => {
                self.emit_error("abrupt-doctype-public-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                if let Some(ref mut id) = self.current_doctype_public_id {
                    id.push(c);
                }
            }
        }
    }

    // 13.2.5.61 After DOCTYPE public identifier state
    fn state_after_doctype_public_identifier(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BetweenDoctypePublicAndSystemIdentifiers;
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Some('"') => {
                self.emit_error("missing-whitespace-between-doctype-public-and-system-identifiers");
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierDoubleQuoted;
            }
            Some('\'') => {
                self.emit_error("missing-whitespace-between-doctype-public-and-system-identifiers");
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierSingleQuoted;
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("missing-quote-before-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.62 Between DOCTYPE public and system identifiers state
    fn state_between_doctype_public_and_system_identifiers(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Some('"') => {
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierDoubleQuoted;
            }
            Some('\'') => {
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierSingleQuoted;
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("missing-quote-before-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.63 After DOCTYPE system keyword state
    fn state_after_doctype_system_keyword(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                self.state = State::BeforeDoctypeSystemIdentifier;
            }
            Some('"') => {
                self.emit_error("missing-whitespace-after-doctype-system-keyword");
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierDoubleQuoted;
            }
            Some('\'') => {
                self.emit_error("missing-whitespace-after-doctype-system-keyword");
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierSingleQuoted;
            }
            Some('>') => {
                self.emit_error("missing-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("missing-quote-before-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.64 Before DOCTYPE system identifier state
    fn state_before_doctype_system_identifier(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('"') => {
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierDoubleQuoted;
            }
            Some('\'') => {
                self.current_doctype_system_id = Some(String::new());
                self.state = State::DoctypeSystemIdentifierSingleQuoted;
            }
            Some('>') => {
                self.emit_error("missing-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("missing-quote-before-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.65 DOCTYPE system identifier (double-quoted) state
    fn state_doctype_system_identifier_double_quoted(&mut self) {
        match self.consume() {
            Some('"') => {
                self.state = State::AfterDoctypeSystemIdentifier;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                if let Some(ref mut id) = self.current_doctype_system_id {
                    id.push('\u{FFFD}');
                }
            }
            Some('>') => {
                self.emit_error("abrupt-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                if let Some(ref mut id) = self.current_doctype_system_id {
                    id.push(c);
                }
            }
        }
    }

    // 13.2.5.66 DOCTYPE system identifier (single-quoted) state
    fn state_doctype_system_identifier_single_quoted(&mut self) {
        match self.consume() {
            Some('\'') => {
                self.state = State::AfterDoctypeSystemIdentifier;
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                if let Some(ref mut id) = self.current_doctype_system_id {
                    id.push('\u{FFFD}');
                }
            }
            Some('>') => {
                self.emit_error("abrupt-doctype-system-identifier");
                self.current_doctype_force_quirks = true;
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(c) => {
                if let Some(ref mut id) = self.current_doctype_system_id {
                    id.push(c);
                }
            }
        }
    }

    // 13.2.5.67 After DOCTYPE system identifier state
    fn state_after_doctype_system_identifier(&mut self) {
        match self.consume() {
            Some('\t' | '\n' | '\x0C' | ' ') => {
                // Ignore.
            }
            Some('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            None => {
                self.emit_error("eof-in-doctype");
                self.current_doctype_force_quirks = true;
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                self.emit_error("unexpected-character-after-doctype-system-identifier");
                // Do NOT set force-quirks.
                self.state = State::BogusDoctype;
            }
        }
    }

    // 13.2.5.68 Bogus DOCTYPE state
    fn state_bogus_doctype(&mut self) {
        match self.consume() {
            Some('>') => {
                self.state = State::Data;
                self.emit_doctype();
            }
            Some('\0') => {
                self.emit_error("unexpected-null-character");
                // Ignore.
            }
            None => {
                self.emit_doctype();
                self.emit_eof();
            }
            Some(_) => {
                // Ignore.
            }
        }
    }

    // 13.2.5.69 CDATA section state
    fn state_cdata_section(&mut self) {
        match self.consume() {
            Some(']') => {
                self.state = State::CdataSectionBracket;
            }
            None => {
                self.emit_error("eof-in-cdata");
                self.emit_eof();
            }
            Some(c) => {
                self.emit_char(c);
            }
        }
    }

    // 13.2.5.70 CDATA section bracket state
    fn state_cdata_section_bracket(&mut self) {
        if let Some(']') = self.peek() {
            self.consume();
            self.state = State::CdataSectionEnd;
        } else {
            self.emit_char(']');
            self.state = State::CdataSection;
        }
    }

    // 13.2.5.71 CDATA section end state
    fn state_cdata_section_end(&mut self) {
        match self.peek() {
            Some(']') => {
                self.consume();
                self.emit_char(']');
            }
            Some('>') => {
                self.consume();
                self.state = State::Data;
            }
            _ => {
                self.emit_char(']');
                self.emit_char(']');
                self.state = State::CdataSection;
            }
        }
    }

    // 13.2.5.72 Character reference state
    fn state_character_reference(&mut self) {
        self.temp_buffer.clear();
        self.temp_buffer.push('&');
        match self.peek() {
            Some(c) if c.is_ascii_alphanumeric() => {
                self.state = State::NamedCharacterReference;
            }
            Some('#') => {
                self.consume();
                self.temp_buffer.push('#');
                self.state = State::NumericCharacterReference;
            }
            _ => {
                self.flush_code_points_consumed_as_char_ref();
                self.state = self.return_state;
            }
        }
    }

    // 13.2.5.73 Named character reference state
    fn state_named_character_reference(&mut self) {
        use crate::html5::entities::is_legacy_named_entity;

        // The WHATWG spec says: consume the maximum number of characters
        // possible where the consumed characters are one of the identifiers
        // in the named character references table. The table has entries
        // both with and without trailing semicolons. Entries without
        // semicolons (the "legacy" set) may match even when no `;` follows;
        // all other entries require the `;` to be present in the input.
        let start = self.pos;
        // (replacement, end_pos, had_semicolon)
        let mut best_semicolon_match: Option<(&str, usize)> = None;
        let mut best_legacy_match: Option<(&str, usize)> = None;
        let mut name = String::new();

        // Greedily consume characters that could be part of an entity name.
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() {
                name.push(c);
                self.consume();
                if let Some(replacement) = lookup_entity(&name) {
                    if self.peek() == Some(';') {
                        self.consume();
                        best_semicolon_match = Some((replacement, self.pos));
                        // A semicolon match is always the best. Keep going
                        // would be past the `;`, so stop.
                        break;
                    }
                    // Without semicolon: only valid for legacy entities.
                    if is_legacy_named_entity(&name) {
                        best_legacy_match = Some((replacement, self.pos));
                    }
                }
            } else {
                break;
            }
        }

        // Prefer semicolon match, then legacy match, then no match.
        let best_match = best_semicolon_match
            .map(|(r, p)| (r, p, true))
            .or(best_legacy_match.map(|(r, p)| (r, p, false)));

        if let Some((replacement, end_pos, had_semicolon)) = best_match {
            self.pos = end_pos;

            if !had_semicolon {
                // Check if we are in an attribute value and the next char
                // is `=` or alphanumeric — if so, treat as not a reference.
                if is_attr_value_state(self.return_state) {
                    if let Some(next) = self.peek() {
                        if next == '=' || next.is_ascii_alphanumeric() {
                            self.pos = start;
                            self.flush_code_points_consumed_as_char_ref();
                            self.state = self.return_state;
                            return;
                        }
                    }
                }
                self.emit_error("missing-semicolon-after-character-reference");
            }

            self.temp_buffer.clear();
            self.temp_buffer.push_str(replacement);
            self.flush_code_points_consumed_as_char_ref();
            self.state = self.return_state;
        } else {
            // No match found — rewind to start.
            self.pos = start;
            self.flush_code_points_consumed_as_char_ref();
            self.state = State::AmbiguousAmpersand;
        }
    }

    // 13.2.5.74 Ambiguous ampersand state
    fn state_ambiguous_ampersand(&mut self) {
        match self.consume() {
            Some(c) if c.is_ascii_alphanumeric() => {
                if is_attr_value_state(self.return_state) {
                    self.current_attr_value.push(c);
                } else {
                    self.emit_char(c);
                }
            }
            Some(';') => {
                self.emit_error("unknown-named-character-reference");
                self.reconsume(';');
                self.state = self.return_state;
            }
            Some(c) => {
                self.reconsume(c);
                self.state = self.return_state;
            }
            None => {
                self.state = self.return_state;
            }
        }
    }

    // 13.2.5.75 Numeric character reference state
    fn state_numeric_character_reference(&mut self) {
        self.char_ref_code = 0;
        match self.peek() {
            Some('x' | 'X') => {
                let c = self.consume();
                if let Some(ch) = c {
                    self.temp_buffer.push(ch);
                }
                self.state = State::HexadecimalCharacterReferenceStart;
            }
            _ => {
                self.state = State::DecimalCharacterReferenceStart;
            }
        }
    }

    // 13.2.5.76 Hexadecimal character reference start state
    fn state_hexadecimal_character_reference_start(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_hexdigit() => {
                self.state = State::HexadecimalCharacterReference;
            }
            _ => {
                self.emit_error("absence-of-digits-in-numeric-character-reference");
                self.flush_code_points_consumed_as_char_ref();
                self.state = self.return_state;
            }
        }
    }

    // 13.2.5.77 Decimal character reference start state
    fn state_decimal_character_reference_start(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_digit() => {
                self.state = State::DecimalCharacterReference;
            }
            _ => {
                self.emit_error("absence-of-digits-in-numeric-character-reference");
                self.flush_code_points_consumed_as_char_ref();
                self.state = self.return_state;
            }
        }
    }

    // 13.2.5.78 Hexadecimal character reference state
    fn state_hexadecimal_character_reference(&mut self) {
        match self.consume() {
            Some(c) if c.is_ascii_hexdigit() => {
                self.char_ref_code = self
                    .char_ref_code
                    .saturating_mul(16)
                    .saturating_add(hex_digit_value(c));
            }
            Some(';') => {
                self.state = State::NumericCharacterReferenceEnd;
            }
            Some(c) => {
                self.emit_error("missing-semicolon-after-character-reference");
                self.reconsume(c);
                self.state = State::NumericCharacterReferenceEnd;
            }
            None => {
                self.emit_error("missing-semicolon-after-character-reference");
                self.state = State::NumericCharacterReferenceEnd;
            }
        }
    }

    // 13.2.5.79 Decimal character reference state
    fn state_decimal_character_reference(&mut self) {
        match self.consume() {
            Some(c) if c.is_ascii_digit() => {
                self.char_ref_code = self
                    .char_ref_code
                    .saturating_mul(10)
                    .saturating_add(u32::from(c as u8 - b'0'));
            }
            Some(';') => {
                self.state = State::NumericCharacterReferenceEnd;
            }
            Some(c) => {
                self.emit_error("missing-semicolon-after-character-reference");
                self.reconsume(c);
                self.state = State::NumericCharacterReferenceEnd;
            }
            None => {
                self.emit_error("missing-semicolon-after-character-reference");
                self.state = State::NumericCharacterReferenceEnd;
            }
        }
    }

    // 13.2.5.80 Numeric character reference end state
    fn state_numeric_character_reference_end(&mut self) {
        let code = self.char_ref_code;
        let ch = if code == 0 {
            self.emit_error("null-character-reference");
            '\u{FFFD}'
        } else if code > 0x10_FFFF {
            self.emit_error("character-reference-outside-unicode-range");
            '\u{FFFD}'
        } else if is_surrogate(code) {
            self.emit_error("surrogate-character-reference");
            '\u{FFFD}'
        } else if is_noncharacter(code) {
            self.emit_error("noncharacter-character-reference");
            // The spec says to use the code point anyway for noncharacters.
            char_from_u32(code)
        } else if code == 0x0D || (is_control(code) && !is_ascii_whitespace_codepoint(code)) {
            self.emit_error("control-character-reference");
            numeric_ref_replacement(code)
        } else {
            char_from_u32(code)
        };

        self.temp_buffer.clear();
        self.temp_buffer.push(ch);
        self.flush_code_points_consumed_as_char_ref();
        self.state = self.return_state;
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Returns true if the given state is one of the attribute value states.
fn is_attr_value_state(state: State) -> bool {
    matches!(
        state,
        State::AttributeValueDoubleQuoted
            | State::AttributeValueSingleQuoted
            | State::AttributeValueUnquoted
    )
}

/// Convert a hex digit character to its numeric value.
fn hex_digit_value(c: char) -> u32 {
    match c {
        '0'..='9' => u32::from(c as u8 - b'0'),
        'a'..='f' => u32::from(c as u8 - b'a') + 10,
        'A'..='F' => u32::from(c as u8 - b'A') + 10,
        _ => 0,
    }
}

/// Convert a `u32` to a `char`, falling back to U+FFFD.
fn char_from_u32(code: u32) -> char {
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

/// Is the code point a surrogate (U+D800..=U+DFFF)?
fn is_surrogate(code: u32) -> bool {
    (0xD800..=0xDFFF).contains(&code)
}

/// Is the code point a noncharacter?
fn is_noncharacter(code: u32) -> bool {
    matches!(
        code,
        0xFDD0
            ..=0xFDEF
                | 0xFFFE
                | 0xFFFF
                | 0x1_FFFE
                | 0x1_FFFF
                | 0x2_FFFE
                | 0x2_FFFF
                | 0x3_FFFE
                | 0x3_FFFF
                | 0x4_FFFE
                | 0x4_FFFF
                | 0x5_FFFE
                | 0x5_FFFF
                | 0x6_FFFE
                | 0x6_FFFF
                | 0x7_FFFE
                | 0x7_FFFF
                | 0x8_FFFE
                | 0x8_FFFF
                | 0x9_FFFE
                | 0x9_FFFF
                | 0xA_FFFE
                | 0xA_FFFF
                | 0xB_FFFE
                | 0xB_FFFF
                | 0xC_FFFE
                | 0xC_FFFF
                | 0xD_FFFE
                | 0xD_FFFF
                | 0xE_FFFE
                | 0xE_FFFF
                | 0xF_FFFE
                | 0xF_FFFF
                | 0x10_FFFE
                | 0x10_FFFF
    )
}

/// Is the code point a control character (C0 or DEL range)?
fn is_control(code: u32) -> bool {
    matches!(code, 0x00..=0x1F | 0x7F..=0x9F)
}

/// Is the code point one of the ASCII whitespace code points?
fn is_ascii_whitespace_codepoint(code: u32) -> bool {
    matches!(code, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// The WHATWG numeric character reference replacement table (section 13.2.5.80).
///
/// Certain control-character code points in the 0x80..=0x9F range are
/// replaced with Windows-1252 code points.
fn numeric_ref_replacement(code: u32) -> char {
    match code {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        _ => char_from_u32(code),
    }
}

/// Normalize newlines per WHATWG spec §13.2.3.
///
/// Replace every CR (U+000D) and every CR+LF pair with a single LF (U+000A).
fn normalize_newlines(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            result.push('\n');
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Collect all tokens from a tokenizer.
    fn tokenize(input: &str) -> Vec<Token> {
        let mut tok = Tokenizer::new(input);
        let mut tokens = Vec::new();
        loop {
            let t = tok.next_token();
            if t == Token::Eof {
                tokens.push(t);
                break;
            }
            tokens.push(t);
        }
        tokens
    }

    /// Collect all non-EOF tokens.
    fn tokenize_body(input: &str) -> Vec<Token> {
        tokenize(input)
            .into_iter()
            .filter(|t| *t != Token::Eof)
            .collect()
    }

    /// Helper: collect just the errors.
    fn tokenize_errors(input: &str) -> Vec<String> {
        let mut tok = Tokenizer::new(input);
        loop {
            if tok.next_token() == Token::Eof {
                break;
            }
        }
        tok.errors().iter().map(|e| e.code.to_string()).collect()
    }

    #[test]
    fn test_basic_start_tag() {
        let tokens = tokenize_body("<div>");
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "div".into(),
                attributes: vec![],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_basic_end_tag() {
        let tokens = tokenize_body("</div>");
        assert_eq!(tokens, vec![Token::EndTag { name: "div".into() }]);
    }

    #[test]
    fn test_self_closing_tag() {
        let tokens = tokenize_body("<br/>");
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "br".into(),
                attributes: vec![],
                self_closing: true,
            }]
        );
    }

    #[test]
    fn test_tag_with_attributes() {
        let tokens = tokenize_body(r#"<div class="main" id='app'>"#);
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "div".into(),
                attributes: vec![
                    Attribute {
                        name: "class".into(),
                        value: "main".into(),
                    },
                    Attribute {
                        name: "id".into(),
                        value: "app".into(),
                    },
                ],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_unquoted_attribute() {
        let tokens = tokenize_body("<div class=main>");
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "div".into(),
                attributes: vec![Attribute {
                    name: "class".into(),
                    value: "main".into(),
                }],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_boolean_attribute() {
        let tokens = tokenize_body("<input disabled>");
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "input".into(),
                attributes: vec![Attribute {
                    name: "disabled".into(),
                    value: String::new(),
                }],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_comment() {
        let tokens = tokenize_body("<!-- hello -->");
        assert_eq!(tokens, vec![Token::Comment(" hello ".into())]);
    }

    #[test]
    fn test_doctype() {
        let tokens = tokenize_body("<!DOCTYPE html>");
        assert_eq!(
            tokens,
            vec![Token::Doctype {
                name: Some("html".into()),
                public_id: None,
                system_id: None,
                force_quirks: false,
            }]
        );
    }

    #[test]
    fn test_doctype_with_public_system() {
        let tokens = tokenize_body(
            r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd">"#,
        );
        assert_eq!(
            tokens,
            vec![Token::Doctype {
                name: Some("html".into()),
                public_id: Some("-//W3C//DTD HTML 4.01//EN".into()),
                system_id: Some("http://www.w3.org/TR/html4/strict.dtd".into()),
                force_quirks: false,
            }]
        );
    }

    #[test]
    fn test_character_data() {
        let tokens = tokenize_body("hello");
        assert_eq!(
            tokens,
            vec![
                Token::Character('h'),
                Token::Character('e'),
                Token::Character('l'),
                Token::Character('l'),
                Token::Character('o'),
            ]
        );
    }

    #[test]
    fn test_named_character_reference() {
        let tokens = tokenize_body("&amp;");
        assert_eq!(tokens, vec![Token::Character('&')]);
    }

    #[test]
    fn test_named_character_reference_nbsp() {
        let tokens = tokenize_body("&nbsp;");
        assert_eq!(tokens, vec![Token::Character('\u{00A0}')]);
    }

    #[test]
    fn test_numeric_decimal_reference() {
        let tokens = tokenize_body("&#65;");
        assert_eq!(tokens, vec![Token::Character('A')]);
    }

    #[test]
    fn test_numeric_hex_reference() {
        let tokens = tokenize_body("&#x41;");
        assert_eq!(tokens, vec![Token::Character('A')]);
    }

    #[test]
    fn test_numeric_hex_reference_uppercase() {
        let tokens = tokenize_body("&#X41;");
        assert_eq!(tokens, vec![Token::Character('A')]);
    }

    #[test]
    fn test_numeric_reference_replacement_table() {
        // &#128; (0x80) should map to Euro sign U+20AC
        let tokens = tokenize_body("&#128;");
        assert_eq!(tokens, vec![Token::Character('\u{20AC}')]);
    }

    #[test]
    fn test_numeric_reference_null() {
        // &#0; should map to U+FFFD
        let tokens = tokenize_body("&#0;");
        assert_eq!(tokens, vec![Token::Character('\u{FFFD}')]);
    }

    #[test]
    fn test_set_state_rawtext() {
        let mut tok = Tokenizer::new("<div>ignored</div>");
        // Simulate tree builder switching to RawText after seeing a style tag.
        tok.set_state(State::RawText);
        // In RawText, everything is character tokens until `</` + matching tag.
        let first = tok.next_token();
        assert_eq!(first, Token::Character('<'));
    }

    #[test]
    fn test_set_state_rcdata() {
        let mut tok = Tokenizer::new("hello &amp; world");
        tok.set_state(State::RcData);
        let mut chars = String::new();
        loop {
            match tok.next_token() {
                Token::Character(c) => chars.push(c),
                Token::Eof => break,
                _ => {}
            }
        }
        assert_eq!(chars, "hello & world");
    }

    #[test]
    fn test_eof_in_tag_error() {
        let errors = tokenize_errors("<div");
        assert!(errors.contains(&"eof-in-tag".to_string()));
    }

    #[test]
    fn test_eof_in_comment_error() {
        let errors = tokenize_errors("<!-- unclosed");
        assert!(errors.contains(&"eof-in-comment".to_string()));
    }

    #[test]
    fn test_missing_attribute_value_error() {
        let errors = tokenize_errors("<div class=>");
        assert!(errors.contains(&"missing-attribute-value".to_string()));
    }

    #[test]
    fn test_eof_before_tag_name() {
        let tokens = tokenize_body("<");
        // Should emit '<' as character, then EOF (which we filter).
        assert_eq!(tokens, vec![Token::Character('<')]);
    }

    #[test]
    fn test_duplicate_attributes_ignored() {
        let tokens = tokenize_body(r#"<div a="1" a="2">"#);
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "div".into(),
                attributes: vec![Attribute {
                    name: "a".into(),
                    value: "1".into(),
                }],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_tag_name_case_lowered() {
        let tokens = tokenize_body("<DIV>");
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "div".into(),
                attributes: vec![],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_cdata_section() {
        // In non-foreign content (allow_cdata=false), CDATA is treated as bogus comment.
        let tokens = tokenize_body("<![CDATA[hello]]>");
        assert_eq!(tokens, vec![Token::Comment("[CDATA[hello]]".into())]);
    }

    #[test]
    fn test_cdata_section_in_foreign_content() {
        // When allow_cdata=true, CDATA content is emitted as character tokens.
        let mut tok = Tokenizer::new("<![CDATA[hello]]>");
        tok.set_allow_cdata(true);
        let mut chars = String::new();
        loop {
            match tok.next_token() {
                Token::Character(c) => chars.push(c),
                Token::Eof => break,
                _ => {}
            }
        }
        assert_eq!(chars, "hello");
    }

    #[test]
    fn test_bogus_comment_from_question_mark() {
        let tokens = tokenize_body("<?xml version='1.0'?>");
        // Should be treated as a bogus comment.
        assert_eq!(tokens, vec![Token::Comment("?xml version='1.0'?".into())]);
    }

    #[test]
    fn test_null_in_data_emitted() {
        // Per WHATWG spec, Data state emits null as-is (with parse error).
        let tokens = tokenize_body("\0");
        assert_eq!(tokens, vec![Token::Character('\0')]);
    }

    #[test]
    fn test_empty_input() {
        let tokens = tokenize("");
        assert_eq!(tokens, vec![Token::Eof]);
    }

    #[test]
    fn test_multiple_attributes_mixed_quoting() {
        let tokens = tokenize_body(r#"<a href="url" target=_blank title='tip'>"#);
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "a".into(),
                attributes: vec![
                    Attribute {
                        name: "href".into(),
                        value: "url".into(),
                    },
                    Attribute {
                        name: "target".into(),
                        value: "_blank".into(),
                    },
                    Attribute {
                        name: "title".into(),
                        value: "tip".into(),
                    },
                ],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_character_reference_in_attribute() {
        let tokens = tokenize_body(r#"<a href="?a=1&amp;b=2">"#);
        assert_eq!(
            tokens,
            vec![Token::StartTag {
                name: "a".into(),
                attributes: vec![Attribute {
                    name: "href".into(),
                    value: "?a=1&b=2".into(),
                }],
                self_closing: false,
            }]
        );
    }

    #[test]
    fn test_abrupt_closing_of_empty_comment() {
        let tokens = tokenize_body("<!-->");
        assert_eq!(tokens, vec![Token::Comment(String::new())]);
        let errors = tokenize_errors("<!-->");
        assert!(errors.contains(&"abrupt-closing-of-empty-comment".to_string()));
    }

    #[test]
    fn test_incorrectly_opened_comment() {
        let tokens = tokenize_body("<!foo>");
        assert_eq!(tokens, vec![Token::Comment("foo".into())]);
        let errors = tokenize_errors("<!foo>");
        assert!(errors.contains(&"incorrectly-opened-comment".to_string()));
    }

    #[test]
    fn test_eof_in_doctype() {
        let tokens = tokenize_body("<!DOCTYPE");
        assert_eq!(
            tokens,
            vec![Token::Doctype {
                name: None,
                public_id: None,
                system_id: None,
                force_quirks: true,
            }]
        );
        let errors = tokenize_errors("<!DOCTYPE");
        assert!(errors.contains(&"eof-in-doctype".to_string()));
    }
}
