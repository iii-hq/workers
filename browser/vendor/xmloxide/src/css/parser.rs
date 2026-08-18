//! CSS selector parser.
//!
//! Hand-rolled recursive descent parser that converts a CSS selector string
//! into a [`SelectorGroup`] AST.

use super::types::{
    AttrMatcher, AttrOp, AttrSelector, Combinator, CompoundEntry, CompoundSelector, NthExpr,
    PseudoClass, Selector, SelectorGroup,
};

/// Parse error with position information.
#[derive(Debug, Clone)]
pub struct CssSelectorError {
    /// Human-readable error message.
    pub message: String,
    /// Byte offset in the input where the error occurred.
    pub position: usize,
}

impl std::fmt::Display for CssSelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CSS selector error at {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for CssSelectorError {}

/// Parse a CSS selector string into a [`SelectorGroup`].
///
/// # Errors
///
/// Returns a [`CssSelectorError`] if the selector string is malformed.
pub fn parse_selector(input: &str) -> Result<SelectorGroup, CssSelectorError> {
    let mut parser = Parser::new(input);
    parser.parse_selector_group()
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self, n: usize) {
        self.pos += n;
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.advance(1);
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn err(&self, msg: impl Into<String>) -> CssSelectorError {
        CssSelectorError {
            message: msg.into(),
            position: self.pos,
        }
    }

    // --- Grammar ---

    fn parse_selector_group(&mut self) -> Result<SelectorGroup, CssSelectorError> {
        let mut selectors = vec![self.parse_selector()?];
        loop {
            self.skip_whitespace();
            if self.peek() == Some(',') {
                self.advance(1);
                self.skip_whitespace();
                selectors.push(self.parse_selector()?);
            } else {
                break;
            }
        }
        if !self.at_end() {
            return Err(self.err(format!(
                "unexpected character '{}'",
                self.peek().unwrap_or('?')
            )));
        }
        Ok(SelectorGroup { selectors })
    }

    fn parse_selector(&mut self) -> Result<Selector, CssSelectorError> {
        let first = self.parse_compound()?;
        let mut compounds = vec![CompoundEntry {
            combinator: Combinator::None,
            compound: first,
        }];

        loop {
            let had_ws = self.skip_ws_and_check();
            if self.at_end() || self.peek() == Some(',') {
                break;
            }

            let combinator = if self.peek() == Some('>') {
                self.advance(1);
                self.skip_whitespace();
                Combinator::Child
            } else if self.peek() == Some('+') {
                self.advance(1);
                self.skip_whitespace();
                Combinator::NextSibling
            } else if self.peek() == Some('~') {
                self.advance(1);
                self.skip_whitespace();
                Combinator::SubsequentSibling
            } else if had_ws {
                Combinator::Descendant
            } else {
                break;
            };

            compounds.push(CompoundEntry {
                combinator,
                compound: self.parse_compound()?,
            });
        }

        Ok(Selector { compounds })
    }

    /// Skip whitespace and return whether any was skipped.
    fn skip_ws_and_check(&mut self) -> bool {
        let before = self.pos;
        self.skip_whitespace();
        self.pos > before
    }

    fn parse_compound(&mut self) -> Result<CompoundSelector, CssSelectorError> {
        let mut compound = CompoundSelector::default();
        let mut has_component = false;

        // Optional tag name or *
        if self
            .peek()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '*')
        {
            if self.peek() == Some('*') {
                self.advance(1);
                // Universal selector — tag stays None but is still a valid component
            } else {
                compound.tag = Some(self.parse_ident()?);
            }
            has_component = true;
        }

        // Simple selectors: #id, .class, [attr], :pseudo
        loop {
            match self.peek() {
                Some('#') => {
                    self.advance(1);
                    compound.id = Some(self.parse_ident()?);
                    has_component = true;
                }
                Some('.') => {
                    self.advance(1);
                    compound.classes.push(self.parse_ident()?);
                    has_component = true;
                }
                Some('[') => {
                    compound.attrs.push(self.parse_attr_selector()?);
                    has_component = true;
                }
                Some(':') => {
                    compound.pseudos.push(self.parse_pseudo_class()?);
                    has_component = true;
                }
                _ => break,
            }
        }

        if !has_component {
            return Err(self.err("expected selector"));
        }

        Ok(compound)
    }

    fn parse_ident(&mut self) -> Result<String, CssSelectorError> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            self.advance(self.peek().unwrap_or(' ').len_utf8());
        }
        if self.pos == start {
            return Err(self.err("expected identifier"));
        }
        Ok(self.input[start..self.pos].to_string())
    }

    fn parse_attr_selector(&mut self) -> Result<AttrSelector, CssSelectorError> {
        self.advance(1); // consume '['
        self.skip_whitespace();

        let name = self.parse_ident()?;
        self.skip_whitespace();

        let matcher = if self.peek() == Some(']') {
            None
        } else {
            let op = self.parse_attr_op()?;
            self.skip_whitespace();
            let value = self.parse_attr_value()?;
            self.skip_whitespace();
            let case_insensitive = if self.peek() == Some('i') || self.peek() == Some('I') {
                self.advance(1);
                self.skip_whitespace();
                true
            } else {
                false
            };
            Some(AttrMatcher {
                op,
                value,
                case_insensitive,
            })
        };

        if self.peek() != Some(']') {
            return Err(self.err("expected ']'"));
        }
        self.advance(1);

        Ok(AttrSelector { name, matcher })
    }

    fn parse_attr_op(&mut self) -> Result<AttrOp, CssSelectorError> {
        let op = match self.peek() {
            Some('=') => {
                self.advance(1);
                AttrOp::Exact
            }
            Some('~') => {
                self.advance(1);
                self.expect_char('=')?;
                AttrOp::Word
            }
            Some('|') => {
                self.advance(1);
                self.expect_char('=')?;
                AttrOp::DashPrefix
            }
            Some('^') => {
                self.advance(1);
                self.expect_char('=')?;
                AttrOp::Prefix
            }
            Some('$') => {
                self.advance(1);
                self.expect_char('=')?;
                AttrOp::Suffix
            }
            Some('*') => {
                self.advance(1);
                self.expect_char('=')?;
                AttrOp::Substring
            }
            _ => return Err(self.err("expected attribute operator")),
        };
        Ok(op)
    }

    fn parse_attr_value(&mut self) -> Result<String, CssSelectorError> {
        match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                self.advance(1);
                let start = self.pos;
                while self.peek().is_some_and(|c| c != quote) {
                    self.advance(self.peek().unwrap_or(' ').len_utf8());
                }
                let value = self.input[start..self.pos].to_string();
                self.expect_char(quote)?;
                Ok(value)
            }
            _ => self.parse_ident(),
        }
    }

    fn parse_pseudo_class(&mut self) -> Result<PseudoClass, CssSelectorError> {
        self.advance(1); // consume ':'
        let name = self.parse_ident()?;

        match name.as_str() {
            "first-child" => Ok(PseudoClass::FirstChild),
            "last-child" => Ok(PseudoClass::LastChild),
            "only-child" => Ok(PseudoClass::OnlyChild),
            "empty" => Ok(PseudoClass::Empty),
            "not" => {
                self.expect_char('(')?;
                self.skip_whitespace();
                let inner = self.parse_compound()?;
                self.skip_whitespace();
                self.expect_char(')')?;
                Ok(PseudoClass::Not(Box::new(inner)))
            }
            "nth-child" => {
                self.expect_char('(')?;
                let expr = self.parse_nth_expr()?;
                self.expect_char(')')?;
                Ok(PseudoClass::NthChild(expr))
            }
            "nth-last-child" => {
                self.expect_char('(')?;
                let expr = self.parse_nth_expr()?;
                self.expect_char(')')?;
                Ok(PseudoClass::NthLastChild(expr))
            }
            _ => Err(self.err(format!("unknown pseudo-class ':{name}'"))),
        }
    }

    fn parse_nth_expr(&mut self) -> Result<NthExpr, CssSelectorError> {
        self.skip_whitespace();

        // Handle keywords: odd, even
        if self.remaining().starts_with("odd") {
            self.advance(3);
            self.skip_whitespace();
            return Ok(NthExpr { a: 2, b: 1 });
        }
        if self.remaining().starts_with("even") {
            self.advance(4);
            self.skip_whitespace();
            return Ok(NthExpr { a: 2, b: 0 });
        }

        // Parse An+B
        let neg = self.peek() == Some('-');
        if neg || self.peek() == Some('+') {
            self.advance(1);
        }

        // Check for 'n' without leading number (means 1n or -1n)
        if self.peek() == Some('n') {
            self.advance(1);
            let a = if neg { -1 } else { 1 };
            let b = self.parse_nth_offset()?;
            self.skip_whitespace();
            return Ok(NthExpr { a, b });
        }

        // Parse number
        let num = self.parse_int()?;
        let num = if neg { -num } else { num };

        if self.peek() == Some('n') {
            self.advance(1);
            let b = self.parse_nth_offset()?;
            self.skip_whitespace();
            Ok(NthExpr { a: num, b })
        } else {
            self.skip_whitespace();
            Ok(NthExpr { a: 0, b: num })
        }
    }

    fn parse_nth_offset(&mut self) -> Result<i32, CssSelectorError> {
        self.skip_whitespace();
        match self.peek() {
            Some('+') => {
                self.advance(1);
                self.skip_whitespace();
                self.parse_int()
            }
            Some('-') => {
                self.advance(1);
                self.skip_whitespace();
                self.parse_int().map(|n| -n)
            }
            _ => Ok(0),
        }
    }

    fn parse_int(&mut self) -> Result<i32, CssSelectorError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.advance(1);
        }
        if self.pos == start {
            return Err(self.err("expected number"));
        }
        self.input[start..self.pos]
            .parse()
            .map_err(|_| self.err("invalid number"))
    }

    fn expect_char(&mut self, expected: char) -> Result<(), CssSelectorError> {
        if self.peek() == Some(expected) {
            self.advance(expected.len_utf8());
            Ok(())
        } else {
            Err(self.err(format!(
                "expected '{expected}', got '{}'",
                self.peek().unwrap_or('?')
            )))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_selector() {
        let sg = parse_selector("div").unwrap();
        assert_eq!(sg.selectors.len(), 1);
        let s = &sg.selectors[0];
        assert_eq!(s.compounds.len(), 1);
        assert_eq!(s.compounds[0].compound.tag.as_deref(), Some("div"));
    }

    #[test]
    fn test_class_selector() {
        let sg = parse_selector(".intro").unwrap();
        assert_eq!(sg.selectors[0].compounds[0].compound.classes, vec!["intro"]);
    }

    #[test]
    fn test_id_selector() {
        let sg = parse_selector("#main").unwrap();
        assert_eq!(
            sg.selectors[0].compounds[0].compound.id.as_deref(),
            Some("main")
        );
    }

    #[test]
    fn test_compound_selector() {
        let sg = parse_selector("div.intro#first").unwrap();
        let c = &sg.selectors[0].compounds[0].compound;
        assert_eq!(c.tag.as_deref(), Some("div"));
        assert_eq!(c.classes, vec!["intro"]);
        assert_eq!(c.id.as_deref(), Some("first"));
    }

    #[test]
    fn test_descendant_combinator() {
        let sg = parse_selector("div p").unwrap();
        assert_eq!(sg.selectors[0].compounds.len(), 2);
        assert_eq!(
            sg.selectors[0].compounds[1].combinator,
            Combinator::Descendant
        );
    }

    #[test]
    fn test_child_combinator() {
        let sg = parse_selector("div > p").unwrap();
        assert_eq!(sg.selectors[0].compounds[1].combinator, Combinator::Child);
    }

    #[test]
    fn test_sibling_combinators() {
        let sg = parse_selector("div + p").unwrap();
        assert_eq!(
            sg.selectors[0].compounds[1].combinator,
            Combinator::NextSibling
        );

        let sg = parse_selector("div ~ p").unwrap();
        assert_eq!(
            sg.selectors[0].compounds[1].combinator,
            Combinator::SubsequentSibling
        );
    }

    #[test]
    fn test_selector_group() {
        let sg = parse_selector("div, p, span").unwrap();
        assert_eq!(sg.selectors.len(), 3);
    }

    #[test]
    fn test_attr_existence() {
        let sg = parse_selector("[href]").unwrap();
        let attr = &sg.selectors[0].compounds[0].compound.attrs[0];
        assert_eq!(attr.name, "href");
        assert!(attr.matcher.is_none());
    }

    #[test]
    fn test_attr_exact() {
        let sg = parse_selector("[type=\"text\"]").unwrap();
        let attr = &sg.selectors[0].compounds[0].compound.attrs[0];
        assert_eq!(attr.name, "type");
        let m = attr.matcher.as_ref().unwrap();
        assert_eq!(m.op, AttrOp::Exact);
        assert_eq!(m.value, "text");
    }

    #[test]
    fn test_attr_prefix() {
        let sg = parse_selector("[href^=\"https\"]").unwrap();
        let m = sg.selectors[0].compounds[0].compound.attrs[0]
            .matcher
            .as_ref()
            .unwrap();
        assert_eq!(m.op, AttrOp::Prefix);
        assert_eq!(m.value, "https");
    }

    #[test]
    fn test_pseudo_first_child() {
        let sg = parse_selector("p:first-child").unwrap();
        assert!(matches!(
            sg.selectors[0].compounds[0].compound.pseudos[0],
            PseudoClass::FirstChild
        ));
    }

    #[test]
    fn test_pseudo_not() {
        let sg = parse_selector(":not(.hidden)").unwrap();
        if let PseudoClass::Not(inner) = &sg.selectors[0].compounds[0].compound.pseudos[0] {
            assert_eq!(inner.classes, vec!["hidden"]);
        } else {
            panic!("expected :not()");
        }
    }

    #[test]
    fn test_pseudo_nth_child() {
        let sg = parse_selector(":nth-child(2n+1)").unwrap();
        if let PseudoClass::NthChild(expr) = &sg.selectors[0].compounds[0].compound.pseudos[0] {
            assert_eq!(expr.a, 2);
            assert_eq!(expr.b, 1);
        } else {
            panic!("expected :nth-child()");
        }
    }

    #[test]
    fn test_pseudo_nth_child_odd() {
        let sg = parse_selector(":nth-child(odd)").unwrap();
        if let PseudoClass::NthChild(expr) = &sg.selectors[0].compounds[0].compound.pseudos[0] {
            assert_eq!(expr.a, 2);
            assert_eq!(expr.b, 1);
        } else {
            panic!("expected :nth-child()");
        }
    }

    #[test]
    fn test_universal_selector() {
        let sg = parse_selector("*").unwrap();
        assert!(sg.selectors[0].compounds[0].compound.tag.is_none());
    }

    #[test]
    fn test_complex_selector() {
        let sg = parse_selector("div.container > ul.nav li.active a[href]").unwrap();
        assert_eq!(sg.selectors[0].compounds.len(), 4);
    }
}
