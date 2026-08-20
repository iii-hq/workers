//! `XPath` 1.0 expression tokenizer.
//!
//! This module implements a lexer for `XPath` 1.0 expressions as specified in
//! <https://www.w3.org/TR/xpath-10/#exprlex>. The lexer converts an `XPath`
//! expression string into a sequence of [`Token`]s that can be consumed by
//! the parser.
//!
//! # Disambiguation Rules
//!
//! The `XPath` 1.0 specification (section 3.7) defines several disambiguation
//! rules that the lexer must apply:
//!
//! - `*` is treated as a multiply operator (not a name test) when the
//!   preceding token could end an operand.
//! - A name followed by `(` is a function name or node type test.
//! - A name followed by `::` is an axis name.
//! - Otherwise, a name is a `NameTest`.

use std::fmt;

/// An error that occurred during `XPath` lexing or parsing.
#[derive(Debug, Clone)]
pub struct XPathError {
    /// Human-readable error message.
    pub message: String,
    /// 0-based byte offset in the expression where the error occurred.
    pub position: usize,
}

impl fmt::Display for XPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XPath error at position {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for XPathError {}

/// The set of node type names recognized as node type tests.
///
/// When one of these names appears before `(`, it is a node type test rather
/// than a function call. See `XPath` 1.0 section 3.7.
const NODE_TYPE_NAMES: &[&str] = &["comment", "text", "processing-instruction", "node"];

/// A token produced by the `XPath` lexer.
///
/// See `XPath` 1.0 section 3.7 for the full token grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// `(` -- left parenthesis.
    LeftParen,
    /// `)` -- right parenthesis.
    RightParen,
    /// `[` -- left bracket (predicate open).
    LeftBracket,
    /// `]` -- right bracket (predicate close).
    RightBracket,
    /// `.` -- current node (abbreviated step).
    Dot,
    /// `..` -- parent node (abbreviated step).
    DotDot,
    /// `@` -- attribute axis abbreviation.
    At,
    /// `,` -- argument separator in function calls.
    Comma,
    /// `::` -- axis separator.
    ColonColon,
    /// `/` -- child step separator.
    Slash,
    /// `//` -- descendant-or-self step abbreviation.
    DoubleSlash,
    /// `|` -- union operator.
    Pipe,
    /// `+` -- addition operator.
    Plus,
    /// `-` -- subtraction or unary negation.
    Minus,
    /// `*` -- multiplication operator (disambiguation from name test
    /// is handled by the lexer based on context).
    Star,
    /// `=` -- equality comparison.
    Equal,
    /// `!=` -- inequality comparison.
    NotEqual,
    /// `<` -- less-than comparison.
    LessThan,
    /// `<=` -- less-than-or-equal comparison.
    LessThanEqual,
    /// `>` -- greater-than comparison.
    GreaterThan,
    /// `>=` -- greater-than-or-equal comparison.
    GreaterThanEqual,
    /// `and` keyword operator.
    And,
    /// `or` keyword operator.
    Or,
    /// `mod` keyword operator.
    Mod,
    /// `div` keyword operator.
    Div,
    /// A numeric literal (e.g., `42`, `3.5`, `.5`).
    Number(f64),
    /// A string literal (e.g., `"hello"` or `'world'`).
    Literal(String),
    /// A qualified name used as a name test (e.g., `foo`, `svg:rect`).
    Name(String),
    /// A variable reference (e.g., `$var`). The string is the name without `$`.
    VariableReference(String),
    /// A function name (name that appeared before `(`).
    FunctionName(String),
    /// A node type test keyword (e.g., `node`, `text`, `comment`,
    /// `processing-instruction`).
    NodeType(String),
    /// An axis name (name that appeared before `::`).
    AxisName(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LeftParen => f.write_str("("),
            Self::RightParen => f.write_str(")"),
            Self::LeftBracket => f.write_str("["),
            Self::RightBracket => f.write_str("]"),
            Self::Dot => f.write_str("."),
            Self::DotDot => f.write_str(".."),
            Self::At => f.write_str("@"),
            Self::Comma => f.write_str(","),
            Self::ColonColon => f.write_str("::"),
            Self::Slash => f.write_str("/"),
            Self::DoubleSlash => f.write_str("//"),
            Self::Pipe => f.write_str("|"),
            Self::Plus => f.write_str("+"),
            Self::Minus => f.write_str("-"),
            Self::Star => f.write_str("*"),
            Self::Equal => f.write_str("="),
            Self::NotEqual => f.write_str("!="),
            Self::LessThan => f.write_str("<"),
            Self::LessThanEqual => f.write_str("<="),
            Self::GreaterThan => f.write_str(">"),
            Self::GreaterThanEqual => f.write_str(">="),
            Self::And => f.write_str("and"),
            Self::Or => f.write_str("or"),
            Self::Mod => f.write_str("mod"),
            Self::Div => f.write_str("div"),
            Self::Number(n) => write!(f, "{n}"),
            Self::Literal(s) => write!(f, "\"{s}\""),
            Self::Name(s) | Self::FunctionName(s) | Self::NodeType(s) | Self::AxisName(s) => {
                write!(f, "{s}")
            }
            Self::VariableReference(s) => write!(f, "${s}"),
        }
    }
}

/// `XPath` 1.0 expression tokenizer.
///
/// The lexer processes an `XPath` expression string and produces a sequence of
/// [`Token`]s. It handles the disambiguation rules from `XPath` 1.0 section 3.7
/// to correctly distinguish between operators and name tests for `*`, and
/// between function names, node type tests, axis names, and plain name tests.
///
/// # Examples
///
/// ```ignore
/// use xmloxide::xpath::lexer::Lexer;
///
/// let mut lexer = Lexer::new("child::p[@class='intro']");
/// let tokens = lexer.tokenize().unwrap();
/// ```
pub struct Lexer<'a> {
    /// The input expression as bytes for efficient indexing.
    input: &'a [u8],
    /// Current byte offset into the input.
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// Creates a new lexer for the given `XPath` expression string.
    #[must_use]
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    /// Tokenizes the entire input expression into a sequence of tokens.
    ///
    /// Applies the `XPath` 1.0 disambiguation rules (section 3.7) to correctly
    /// classify tokens based on their context.
    ///
    /// # Errors
    ///
    /// Returns [`XPathError`] if the input contains an invalid token, such as
    /// an unterminated string literal or an unexpected character.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, XPathError> {
        let mut raw_tokens = Vec::new();

        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                break;
            }
            let token = self.next_raw_token()?;
            raw_tokens.push(token);
        }

        Ok(Self::disambiguate(raw_tokens))
    }

    /// Reads the next raw token from the input.
    ///
    /// At this stage, names are all emitted as `Name`, and `*` is always
    /// emitted as `Star`. Disambiguation happens in a second pass.
    fn next_raw_token(&mut self) -> Result<Token, XPathError> {
        let ch = self
            .peek_byte()
            .ok_or_else(|| self.error("unexpected end of input"))?;

        match ch {
            b'(' => {
                self.advance();
                Ok(Token::LeftParen)
            }
            b')' => {
                self.advance();
                Ok(Token::RightParen)
            }
            b'[' => {
                self.advance();
                Ok(Token::LeftBracket)
            }
            b']' => {
                self.advance();
                Ok(Token::RightBracket)
            }
            b'@' => {
                self.advance();
                Ok(Token::At)
            }
            b',' => {
                self.advance();
                Ok(Token::Comma)
            }
            b'|' => {
                self.advance();
                Ok(Token::Pipe)
            }
            b'+' => {
                self.advance();
                Ok(Token::Plus)
            }
            b'-' => {
                self.advance();
                Ok(Token::Minus)
            }
            b'=' => {
                self.advance();
                Ok(Token::Equal)
            }
            b'*' => {
                self.advance();
                Ok(Token::Star)
            }
            b':' => self.read_colon_colon(),
            b'.' => self.read_dot_or_number(),
            b'/' => Ok(self.read_slash()),
            b'!' => self.read_not_equal(),
            b'<' => Ok(self.read_less_than()),
            b'>' => Ok(self.read_greater_than()),
            b'"' | b'\'' => self.read_string_literal(),
            b'$' => self.read_variable_reference(),
            b'0'..=b'9' => self.read_number(),
            _ if is_name_start_char(ch) => Ok(self.read_name()),
            _ => Err(self.error(&format!("unexpected character '{}'", char::from(ch)))),
        }
    }

    /// Reads a `.` (`Dot`) or `..` (`DotDot`) token, or a number starting with `.`.
    fn read_dot_or_number(&mut self) -> Result<Token, XPathError> {
        self.advance(); // consume the first '.'

        if self.peek_byte() == Some(b'.') {
            self.advance();
            return Ok(Token::DotDot);
        }

        // Check if this is a number like .5
        if matches!(self.peek_byte(), Some(b'0'..=b'9')) {
            let start = self.pos - 1; // include the '.'
            self.advance_while(|b| b.is_ascii_digit());
            let text = self.slice_from(start);
            let value = text
                .parse::<f64>()
                .map_err(|_| make_error(start, &format!("invalid number literal: {text}")))?;
            return Ok(Token::Number(value));
        }

        Ok(Token::Dot)
    }

    /// Reads a `/` or `//` token.
    fn read_slash(&mut self) -> Token {
        self.advance(); // consume '/'
        if self.peek_byte() == Some(b'/') {
            self.advance();
            Token::DoubleSlash
        } else {
            Token::Slash
        }
    }

    /// Reads the `::` (axis separator) token.
    fn read_colon_colon(&mut self) -> Result<Token, XPathError> {
        let start = self.pos;
        self.advance(); // consume first ':'
        if self.peek_byte() == Some(b':') {
            self.advance();
            Ok(Token::ColonColon)
        } else {
            Err(make_error(start, "expected ':' after ':'"))
        }
    }

    /// Reads the `!=` token.
    fn read_not_equal(&mut self) -> Result<Token, XPathError> {
        let start = self.pos;
        self.advance(); // consume '!'
        if self.peek_byte() == Some(b'=') {
            self.advance();
            Ok(Token::NotEqual)
        } else {
            Err(make_error(start, "expected '=' after '!'"))
        }
    }

    /// Reads `<` or `<=`.
    fn read_less_than(&mut self) -> Token {
        self.advance(); // consume '<'
        if self.peek_byte() == Some(b'=') {
            self.advance();
            Token::LessThanEqual
        } else {
            Token::LessThan
        }
    }

    /// Reads `>` or `>=`.
    fn read_greater_than(&mut self) -> Token {
        self.advance(); // consume '>'
        if self.peek_byte() == Some(b'=') {
            self.advance();
            Token::GreaterThanEqual
        } else {
            Token::GreaterThan
        }
    }

    /// Reads a string literal (single or double quoted).
    ///
    /// See `XPath` 1.0 section 3.5: a `Literal` is `'"' [^"]* '"'` or
    /// `"'" [^']* "'"`.
    fn read_string_literal(&mut self) -> Result<Token, XPathError> {
        let start = self.pos;
        let quote = self
            .peek_byte()
            .ok_or_else(|| self.error("unexpected end of input"))?;
        self.advance(); // consume opening quote

        let content_start = self.pos;
        self.advance_while(|b| b != quote);

        if self.pos >= self.input.len() {
            return Err(make_error(start, "unterminated string literal"));
        }

        let content = self.slice_from(content_start).to_string();
        self.advance(); // consume closing quote

        Ok(Token::Literal(content))
    }

    /// Reads a variable reference (`$name`).
    ///
    /// See `XPath` 1.0 section 3.1.
    fn read_variable_reference(&mut self) -> Result<Token, XPathError> {
        let start = self.pos;
        self.advance(); // consume '$'

        if !self.peek_byte().is_some_and(is_name_start_char) {
            return Err(make_error(start, "expected name after '$'"));
        }

        let name_start = self.pos;
        self.advance_while(is_name_char);

        // Handle QName (prefix:localname)
        if self.peek_byte() == Some(b':')
            && self
                .peek_byte_at(self.pos + 1)
                .is_some_and(|b| b != b':' && is_name_start_char(b))
        {
            self.advance(); // consume ':'
            self.advance_while(is_name_char);
            let full_name = self.slice_from(name_start);
            return Ok(Token::VariableReference(full_name.to_string()));
        }

        let name = self.slice_from(name_start);
        Ok(Token::VariableReference(name.to_string()))
    }

    /// Reads a numeric literal.
    ///
    /// See `XPath` 1.0 section 3.5: a `Number` is `Digits ('.' Digits?)?` or
    /// `'.' Digits`.
    fn read_number(&mut self) -> Result<Token, XPathError> {
        let start = self.pos;
        self.advance_while(|b| b.is_ascii_digit());

        // Check for decimal point
        if self.peek_byte() == Some(b'.') {
            self.advance();
            self.advance_while(|b| b.is_ascii_digit());
        }

        let text = self.slice_from(start);
        let value = text
            .parse::<f64>()
            .map_err(|_| make_error(start, &format!("invalid number literal: {text}")))?;
        Ok(Token::Number(value))
    }

    /// Reads a name token (`NCName` or `QName`).
    ///
    /// At this stage, the token is always emitted as `Name`. The disambiguation
    /// pass will reclassify it as `FunctionName`, `NodeType`, or `AxisName`
    /// based on the following token.
    fn read_name(&mut self) -> Token {
        let start = self.pos;
        self.advance_while(is_name_char);

        // Check for QName (prefix:localname) or prefix:* -- but not prefix::axis
        if self.peek_byte() == Some(b':') {
            let next = self.peek_byte_at(self.pos + 1);
            if next.is_some_and(|b| b != b':' && is_name_start_char(b)) {
                // prefix:localname
                self.advance(); // consume ':'
                self.advance_while(is_name_char);
            } else if next == Some(b'*') {
                // prefix:* (namespace wildcard)
                self.advance(); // consume ':'
                self.advance(); // consume '*'
            }
        }

        let name = self.slice_from(start);
        Token::Name(name.to_string())
    }

    /// Applies the `XPath` 1.0 disambiguation rules (section 3.7).
    ///
    /// This function reclassifies raw tokens based on their context:
    /// - `*` after an operand-ending token becomes `Star` (multiply operator);
    ///   otherwise it remains `Name("*")` (name test).
    /// - A `Name` followed by `(` becomes `FunctionName` or `NodeType`.
    /// - A `Name` followed by `::` becomes `AxisName`.
    /// - `and`, `or`, `mod`, `div` after operand-ending tokens become operators.
    fn disambiguate(raw_tokens: Vec<Token>) -> Vec<Token> {
        let len = raw_tokens.len();
        let mut result = Vec::with_capacity(len);

        for (i, token) in raw_tokens.into_iter().enumerate() {
            let preceding_is_operand = if i == 0 {
                false
            } else {
                is_operand_ending(&result[result.len() - 1])
            };

            match token {
                Token::Star if !preceding_is_operand => {
                    // When * is not preceded by an operand-ending token, it's
                    // a name test wildcard, not a multiply operator.
                    result.push(Token::Name("*".to_string()));
                }
                other => result.push(other),
            }
        }

        // Second pass: now that we have the full list, apply name disambiguation.
        disambiguate_names(&mut result);

        result
    }

    // --- Utility methods ---

    /// Returns the byte at the current position, or `None` if at end.
    fn peek_byte(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    /// Returns the byte at the given position, or `None` if out of bounds.
    fn peek_byte_at(&self, pos: usize) -> Option<u8> {
        self.input.get(pos).copied()
    }

    /// Advances the position by one byte.
    fn advance(&mut self) {
        self.pos += 1;
    }

    /// Advances while the predicate holds for the current byte.
    fn advance_while<F: Fn(u8) -> bool>(&mut self, pred: F) {
        while self.pos < self.input.len() && pred(self.input[self.pos]) {
            self.pos += 1;
        }
    }

    /// Skips ASCII whitespace characters.
    fn skip_whitespace(&mut self) {
        self.advance_while(|b| b.is_ascii_whitespace());
    }

    /// Returns the substring from `start` to the current position.
    fn slice_from(&self, start: usize) -> &str {
        // The input was originally a &str so it is valid UTF-8. We only
        // split at ASCII byte boundaries, preserving UTF-8 validity.
        // Using the safe `from_utf8` to avoid any unsafe code.
        std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("")
    }

    /// Creates an error at the current position.
    fn error(&self, message: &str) -> XPathError {
        make_error(self.pos, message)
    }
}

/// Creates an [`XPathError`] at the given position.
fn make_error(position: usize, message: &str) -> XPathError {
    XPathError {
        message: message.to_string(),
        position,
    }
}

/// Second-pass disambiguation for names, function names, node types,
/// axis names, and keyword operators.
fn disambiguate_names(tokens: &mut [Token]) {
    let len = tokens.len();
    let mut i = 0;
    while i < len {
        // Check if current token is a Name
        if let Token::Name(ref name) = tokens[i] {
            let name_clone = name.clone();

            // Determine if the preceding token ends an operand
            let preceding_is_operand = if i == 0 {
                false
            } else {
                is_operand_ending(&tokens[i - 1])
            };

            // Look ahead to next token (whitespace is already stripped).
            let next = tokens.get(i + 1);

            if preceding_is_operand {
                // After an operand, these names are operators.
                match name_clone.as_str() {
                    "and" => tokens[i] = Token::And,
                    "or" => tokens[i] = Token::Or,
                    "mod" => tokens[i] = Token::Mod,
                    "div" => tokens[i] = Token::Div,
                    "*" => {
                        // * after operand is multiply
                        tokens[i] = Token::Star;
                    }
                    _ => {}
                }
            } else if matches!(next, Some(Token::LeftParen)) {
                // Name followed by '(' -- function name or node type test
                if NODE_TYPE_NAMES.contains(&name_clone.as_str()) {
                    tokens[i] = Token::NodeType(name_clone);
                } else {
                    tokens[i] = Token::FunctionName(name_clone);
                }
            } else if matches!(next, Some(Token::ColonColon)) {
                // Name followed by '::' -- axis name
                tokens[i] = Token::AxisName(name_clone);
            }
        }

        i += 1;
    }
}

/// Returns `true` if the given token could end an operand.
///
/// Per `XPath` 1.0 section 3.7, the preceding token determines whether `*` is
/// a multiply operator. If the preceding token is one that could end an
/// expression or name test, then `*` is multiply. If there is no preceding
/// token, or the preceding token is an operator or punctuation, then `*` is
/// a name test (wildcard).
fn is_operand_ending(token: &Token) -> bool {
    matches!(
        token,
        Token::RightParen
            | Token::RightBracket
            | Token::Dot
            | Token::DotDot
            | Token::Number(_)
            | Token::Literal(_)
            | Token::Name(_)
            | Token::VariableReference(_)
            | Token::NodeType(_)
            | Token::Star
    )
}

/// Returns `true` if the byte is a valid name start character.
///
/// For simplicity, we accept ASCII letters and `_`. A full implementation
/// would also accept Unicode letters per the XML `NameStartChar` production,
/// but `XPath` names in practice are ASCII.
fn is_name_start_char(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Returns `true` if the byte is a valid name continuation character.
///
/// Accepts letters, digits, `-`, `_`, and `.`. The hyphen is included
/// because `XPath` axis names like `descendant-or-self` use hyphens, and we
/// consume the full name before disambiguation. The dot is included for
/// names that contain dots, though it is not part of XML `NCName`.
fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Helper to tokenize and return the token vector, panicking on error.
    fn tokenize(input: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(input);
        lexer.tokenize().unwrap()
    }

    #[test]
    fn test_tokenize_simple_path() {
        let tokens = tokenize("child::p");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], Token::AxisName("child".to_string()));
        assert_eq!(tokens[1], Token::ColonColon);
        assert_eq!(tokens[2], Token::Name("p".to_string()));
    }

    #[test]
    fn test_tokenize_abbreviated_path() {
        let tokens = tokenize("/html/body");
        assert_eq!(
            tokens,
            vec![
                Token::Slash,
                Token::Name("html".to_string()),
                Token::Slash,
                Token::Name("body".to_string()),
            ]
        );
    }

    #[test]
    fn test_tokenize_double_slash() {
        let tokens = tokenize("//div");
        assert_eq!(
            tokens,
            vec![Token::DoubleSlash, Token::Name("div".to_string()),]
        );
    }

    #[test]
    fn test_tokenize_predicate() {
        let tokens = tokenize("p[1]");
        assert_eq!(
            tokens,
            vec![
                Token::Name("p".to_string()),
                Token::LeftBracket,
                Token::Number(1.0),
                Token::RightBracket,
            ]
        );
    }

    #[test]
    fn test_tokenize_attribute_access() {
        let tokens = tokenize("@class");
        assert_eq!(tokens, vec![Token::At, Token::Name("class".to_string()),]);
    }

    #[test]
    fn test_tokenize_function_call() {
        let tokens = tokenize("contains(name, 'foo')");
        assert_eq!(
            tokens,
            vec![
                Token::FunctionName("contains".to_string()),
                Token::LeftParen,
                Token::Name("name".to_string()),
                Token::Comma,
                Token::Literal("foo".to_string()),
                Token::RightParen,
            ]
        );
    }

    #[test]
    fn test_tokenize_node_type() {
        let tokens = tokenize("text()");
        assert_eq!(
            tokens,
            vec![
                Token::NodeType("text".to_string()),
                Token::LeftParen,
                Token::RightParen,
            ]
        );
    }

    #[test]
    fn test_tokenize_all_node_types() {
        for name in &["node", "text", "comment", "processing-instruction"] {
            let input = format!("{name}()");
            let tokens = tokenize(&input);
            assert_eq!(tokens[0], Token::NodeType((*name).to_string()));
        }
    }

    #[test]
    fn test_tokenize_string_literals() {
        let tokens = tokenize(r#""hello""#);
        assert_eq!(tokens, vec![Token::Literal("hello".to_string())]);

        let tokens = tokenize("'world'");
        assert_eq!(tokens, vec![Token::Literal("world".to_string())]);
    }

    #[test]
    fn test_tokenize_number_literals() {
        let tokens = tokenize("42");
        assert_eq!(tokens, vec![Token::Number(42.0)]);

        let tokens = tokenize("3.5");
        assert_eq!(tokens, vec![Token::Number(3.5)]);

        let tokens = tokenize(".5");
        assert_eq!(tokens, vec![Token::Number(0.5)]);

        let tokens = tokenize("0.0");
        assert_eq!(tokens, vec![Token::Number(0.0)]);
    }

    #[test]
    fn test_tokenize_variable_reference() {
        let tokens = tokenize("$foo");
        assert_eq!(tokens, vec![Token::VariableReference("foo".to_string())]);
    }

    #[test]
    fn test_tokenize_comparison_operators() {
        let tokens = tokenize("a = b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Equal,
                Token::Name("b".to_string()),
            ]
        );

        let tokens = tokenize("a != b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::NotEqual,
                Token::Name("b".to_string()),
            ]
        );

        let tokens = tokenize("a < b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::LessThan,
                Token::Name("b".to_string()),
            ]
        );

        let tokens = tokenize("a <= b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::LessThanEqual,
                Token::Name("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_tokenize_arithmetic() {
        let tokens = tokenize("1 + 2");
        assert_eq!(
            tokens,
            vec![Token::Number(1.0), Token::Plus, Token::Number(2.0),]
        );

        let tokens = tokenize("a - b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Minus,
                Token::Name("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_tokenize_star_as_wildcard() {
        // * at the start of an expression is a name test (wildcard), not multiply
        let tokens = tokenize("*");
        assert_eq!(tokens, vec![Token::Name("*".to_string())]);

        // * after / is a name test
        let tokens = tokenize("/*");
        assert_eq!(tokens, vec![Token::Slash, Token::Name("*".to_string())]);
    }

    #[test]
    fn test_tokenize_star_as_multiply() {
        // * after a name (operand-ending) is multiply
        let tokens = tokenize("a * b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Star,
                Token::Name("b".to_string()),
            ]
        );

        // * after ) is multiply
        let tokens = tokenize("count(x) * 2");
        assert_eq!(
            tokens,
            vec![
                Token::FunctionName("count".to_string()),
                Token::LeftParen,
                Token::Name("x".to_string()),
                Token::RightParen,
                Token::Star,
                Token::Number(2.0),
            ]
        );
    }

    #[test]
    fn test_tokenize_keyword_operators() {
        // 'and' and 'or' after operand-ending tokens are operators
        let tokens = tokenize("a and b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::And,
                Token::Name("b".to_string()),
            ]
        );

        let tokens = tokenize("a or b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Or,
                Token::Name("b".to_string()),
            ]
        );

        let tokens = tokenize("a div b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Div,
                Token::Name("b".to_string()),
            ]
        );

        let tokens = tokenize("a mod b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Mod,
                Token::Name("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_tokenize_dot_and_dotdot() {
        let tokens = tokenize("./.. ");
        assert_eq!(tokens, vec![Token::Dot, Token::Slash, Token::DotDot,]);
    }

    #[test]
    fn test_tokenize_complex_expression() {
        let tokens = tokenize("//div[@class='main']/p[position() > 1]");
        assert_eq!(
            tokens,
            vec![
                Token::DoubleSlash,
                Token::Name("div".to_string()),
                Token::LeftBracket,
                Token::At,
                Token::Name("class".to_string()),
                Token::Equal,
                Token::Literal("main".to_string()),
                Token::RightBracket,
                Token::Slash,
                Token::Name("p".to_string()),
                Token::LeftBracket,
                Token::FunctionName("position".to_string()),
                Token::LeftParen,
                Token::RightParen,
                Token::GreaterThan,
                Token::Number(1.0),
                Token::RightBracket,
            ]
        );
    }

    #[test]
    fn test_tokenize_union_operator() {
        let tokens = tokenize("a | b");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::Pipe,
                Token::Name("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_tokenize_qname() {
        let tokens = tokenize("svg:rect");
        assert_eq!(tokens, vec![Token::Name("svg:rect".to_string())]);
    }

    #[test]
    fn test_tokenize_axis_specifier() {
        let tokens = tokenize("ancestor-or-self::node()");
        assert_eq!(
            tokens,
            vec![
                Token::AxisName("ancestor-or-self".to_string()),
                Token::ColonColon,
                Token::NodeType("node".to_string()),
                Token::LeftParen,
                Token::RightParen,
            ]
        );
    }

    #[test]
    fn test_tokenize_empty_input() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_whitespace_only() {
        let tokens = tokenize("   \t\n  ");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_unterminated_string_error() {
        let mut lexer = Lexer::new("\"unterminated");
        let result = lexer.tokenize();
        assert!(result.is_err());
        if let Err(err) = result {
            assert!(err.message.contains("unterminated"));
        }
    }

    #[test]
    fn test_tokenize_invalid_char_after_bang() {
        let mut lexer = Lexer::new("!x");
        let result = lexer.tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn test_tokenize_variable_reference_qname() {
        let tokens = tokenize("$ns:var");
        assert_eq!(tokens, vec![Token::VariableReference("ns:var".to_string())]);
    }

    #[test]
    fn test_xpath_error_display() {
        let err = XPathError {
            message: "test error".to_string(),
            position: 5,
        };
        assert_eq!(err.to_string(), "XPath error at position 5: test error");
    }

    #[test]
    fn test_token_display() {
        assert_eq!(Token::LeftParen.to_string(), "(");
        assert_eq!(Token::RightParen.to_string(), ")");
        assert_eq!(Token::Slash.to_string(), "/");
        assert_eq!(Token::DoubleSlash.to_string(), "//");
        assert_eq!(Token::Star.to_string(), "*");
        assert_eq!(Token::Number(2.5).to_string(), "2.5");
        assert_eq!(Token::Literal("hi".to_string()).to_string(), "\"hi\"");
        assert_eq!(Token::VariableReference("x".to_string()).to_string(), "$x");
    }

    #[test]
    fn test_tokenize_multiple_predicates() {
        let tokens = tokenize("p[1][@class]");
        assert_eq!(
            tokens,
            vec![
                Token::Name("p".to_string()),
                Token::LeftBracket,
                Token::Number(1.0),
                Token::RightBracket,
                Token::LeftBracket,
                Token::At,
                Token::Name("class".to_string()),
                Token::RightBracket,
            ]
        );
    }

    #[test]
    fn test_tokenize_nested_function_calls() {
        let tokens = tokenize("concat(substring(a, 1), 'x')");
        assert_eq!(
            tokens,
            vec![
                Token::FunctionName("concat".to_string()),
                Token::LeftParen,
                Token::FunctionName("substring".to_string()),
                Token::LeftParen,
                Token::Name("a".to_string()),
                Token::Comma,
                Token::Number(1.0),
                Token::RightParen,
                Token::Comma,
                Token::Literal("x".to_string()),
                Token::RightParen,
            ]
        );
    }

    #[test]
    fn test_tokenize_unary_minus() {
        let tokens = tokenize("-5");
        assert_eq!(tokens, vec![Token::Minus, Token::Number(5.0),]);
    }

    #[test]
    fn test_tokenize_greater_than_equal() {
        let tokens = tokenize("a >= 10");
        assert_eq!(
            tokens,
            vec![
                Token::Name("a".to_string()),
                Token::GreaterThanEqual,
                Token::Number(10.0),
            ]
        );
    }
}
