//! `XPath` 1.0 expression parser.
//!
//! This module implements a recursive descent parser for `XPath` 1.0 expressions
//! as specified in <https://www.w3.org/TR/xpath-10/#section-Grammar>. The parser
//! consumes a `Vec<Token>` (produced by the [`super::lexer::Lexer`]) and
//! produces an [`Expr`] AST.
//!
//! # Operator Precedence
//!
//! From lowest to highest:
//! 1. `or`
//! 2. `and`
//! 3. `=`, `!=` (equality)
//! 4. `<`, `<=`, `>`, `>=` (relational)
//! 5. `+`, `-` (additive)
//! 6. `*`, `div`, `mod` (multiplicative)
//! 7. Unary `-`
//! 8. `|` (union)
//! 9. Filter expressions (primary expression with predicates)
//! 10. Path expressions (location paths)
//!
//! # Grammar Productions
//!
//! The parser follows the `XPath` 1.0 grammar closely, with each grammar
//! production implemented as a method on the internal `Parser` struct.

use super::ast::{Axis, BinaryOp, Expr, NodeTest, Step};
use super::lexer::{Lexer, Token, XPathError};

/// Parses an `XPath` expression string into an AST.
///
/// This function tokenizes the input using the [`Lexer`] and then parses the
/// token stream into an [`Expr`] AST using a recursive descent parser.
///
/// # Errors
///
/// Returns [`XPathError`] if the input is not a valid `XPath` 1.0 expression.
/// The error includes a human-readable message and the byte offset where the
/// error was detected.
///
/// # Examples
///
/// ```ignore
/// use xmloxide::xpath::parser::parse;
///
/// let expr = parse("/html/body/p").unwrap();
/// let expr = parse("//book[@price > 10.00]").unwrap();
/// ```
pub fn parse(input: &str) -> Result<Expr, XPathError> {
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize()?;

    if tokens.is_empty() {
        return Err(XPathError {
            message: "empty XPath expression".to_string(),
            position: 0,
        });
    }

    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;

    if parser.pos < parser.tokens.len() {
        return Err(parser.error(&format!(
            "unexpected token '{}' after expression",
            parser.tokens[parser.pos]
        )));
    }

    Ok(expr)
}

/// Internal recursive descent parser for `XPath` 1.0 token streams.
struct Parser {
    /// The token stream produced by the lexer.
    tokens: Vec<Token>,
    /// Current position in the token stream.
    pos: usize,
}

impl Parser {
    /// Creates a new parser for the given token stream.
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    // -----------------------------------------------------------------------
    // Token access helpers
    // -----------------------------------------------------------------------

    /// Returns a reference to the current token, or `None` if at end.
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Returns `true` if the current token matches the given token.
    fn check(&self, token: &Token) -> bool {
        self.peek() == Some(token)
    }

    /// Consumes the current token if it matches `token`, returning `true`.
    /// Returns `false` without consuming if it does not match.
    fn eat(&mut self, token: &Token) -> bool {
        if self.check(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consumes the current token if it matches `token`, or returns an error.
    fn expect(&mut self, token: &Token) -> Result<(), XPathError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.error(&format!(
                "expected '{}', found {}",
                token,
                self.describe_current()
            )))
        }
    }

    /// Advances the parser by one token and returns the consumed token.
    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(token)
        } else {
            None
        }
    }

    /// Returns a human-readable description of the current token (for errors).
    fn describe_current(&self) -> String {
        self.peek()
            .map_or_else(|| "end of expression".to_string(), |t| format!("'{t}'"))
    }

    /// Creates an error at the current position.
    fn error(&self, message: &str) -> XPathError {
        XPathError {
            message: message.to_string(),
            position: self.pos,
        }
    }

    // -----------------------------------------------------------------------
    // Grammar productions
    // -----------------------------------------------------------------------

    /// Parses an `XPath` expression.
    ///
    /// ```text
    /// Expr ::= OrExpr
    /// ```
    /// See `XPath` 1.0 section 3.
    fn parse_expr(&mut self) -> Result<Expr, XPathError> {
        self.parse_or_expr()
    }

    /// Parses an `or` expression.
    ///
    /// ```text
    /// OrExpr ::= AndExpr ('or' AndExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.4.
    fn parse_or_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_and_expr()?;
        while self.eat(&Token::Or) {
            let right = self.parse_and_expr()?;
            left = Expr::BinaryOp {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Parses an `and` expression.
    ///
    /// ```text
    /// AndExpr ::= EqualityExpr ('and' EqualityExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.4.
    fn parse_and_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_equality_expr()?;
        while self.eat(&Token::And) {
            let right = self.parse_equality_expr()?;
            left = Expr::BinaryOp {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Parses an equality expression.
    ///
    /// ```text
    /// EqualityExpr ::= RelationalExpr (('=' | '!=') RelationalExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.4.
    fn parse_equality_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_relational_expr()?;
        loop {
            if self.eat(&Token::Equal) {
                let right = self.parse_relational_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Eq,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(&Token::NotEqual) {
                let right = self.parse_relational_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Neq,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Parses a relational expression.
    ///
    /// ```text
    /// RelationalExpr ::= AdditiveExpr (('<' | '<=' | '>' | '>=') AdditiveExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.4.
    fn parse_relational_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_additive_expr()?;
        loop {
            let op = if self.eat(&Token::LessThan) {
                Some(BinaryOp::Lt)
            } else if self.eat(&Token::LessThanEqual) {
                Some(BinaryOp::Lte)
            } else if self.eat(&Token::GreaterThan) {
                Some(BinaryOp::Gt)
            } else if self.eat(&Token::GreaterThanEqual) {
                Some(BinaryOp::Gte)
            } else {
                None
            };
            if let Some(op) = op {
                let right = self.parse_additive_expr()?;
                left = Expr::BinaryOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Parses an additive expression.
    ///
    /// ```text
    /// AdditiveExpr ::= MultiplicativeExpr (('+' | '-') MultiplicativeExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.5.
    fn parse_additive_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_multiplicative_expr()?;
        loop {
            if self.eat(&Token::Plus) {
                let right = self.parse_multiplicative_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Add,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Minus) {
                let right = self.parse_multiplicative_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Sub,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Parses a multiplicative expression.
    ///
    /// ```text
    /// MultiplicativeExpr ::= UnaryExpr (('*' | 'div' | 'mod') UnaryExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.5.
    fn parse_multiplicative_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_unary_expr()?;
        loop {
            if self.eat(&Token::Star) {
                let right = self.parse_unary_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Mul,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Div) {
                let right = self.parse_unary_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Div,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Mod) {
                let right = self.parse_unary_expr()?;
                left = Expr::BinaryOp {
                    op: BinaryOp::Mod,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// Parses a unary expression.
    ///
    /// ```text
    /// UnaryExpr ::= '-'* UnionExpr
    /// ```
    /// See `XPath` 1.0 section 3.5.
    fn parse_unary_expr(&mut self) -> Result<Expr, XPathError> {
        if self.eat(&Token::Minus) {
            let inner = self.parse_unary_expr()?;
            Ok(Expr::UnaryNeg(Box::new(inner)))
        } else {
            self.parse_union_expr()
        }
    }

    /// Parses a union expression.
    ///
    /// ```text
    /// UnionExpr ::= PathExpr ('|' PathExpr)*
    /// ```
    /// See `XPath` 1.0 section 3.3.
    fn parse_union_expr(&mut self) -> Result<Expr, XPathError> {
        let mut left = self.parse_path_expr()?;
        while self.eat(&Token::Pipe) {
            let right = self.parse_path_expr()?;
            left = Expr::Union(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Parses a path expression.
    ///
    /// ```text
    /// PathExpr ::= LocationPath
    ///            | FilterExpr
    ///            | FilterExpr '/' RelativeLocationPath
    ///            | FilterExpr '//' RelativeLocationPath
    /// ```
    ///
    /// The tricky part is distinguishing location paths from filter expressions.
    /// A location path starts with `/`, `//`, `.`, `..`, `@`, an axis name, a
    /// node type, or a name test. A filter expression starts with a primary
    /// expression (variable, literal, number, function call, or parenthesized
    /// expression).
    ///
    /// See `XPath` 1.0 section 3.3.
    fn parse_path_expr(&mut self) -> Result<Expr, XPathError> {
        match self.peek() {
            // Tokens that start a location path: `/`, `//`, `.`, `..`, `@`,
            // axis names, node types, or name tests.
            Some(
                Token::Slash
                | Token::DoubleSlash
                | Token::Dot
                | Token::DotDot
                | Token::At
                | Token::AxisName(_)
                | Token::NodeType(_)
                | Token::Name(_),
            ) => self.parse_location_path(),

            // Primary expressions: variable, literal, number, function call,
            // or parenthesized expression. These start filter expressions.
            Some(
                Token::VariableReference(_)
                | Token::Literal(_)
                | Token::Number(_)
                | Token::LeftParen
                | Token::FunctionName(_),
            ) => {
                let expr = self.parse_filter_expr()?;

                // A filter expression can be followed by '/' or '//' to
                // extend it with a relative location path.
                if self.check(&Token::Slash) || self.check(&Token::DoubleSlash) {
                    self.parse_filter_path_continuation(expr)
                } else {
                    Ok(expr)
                }
            }

            _ => Err(self.error(&format!(
                "expected expression, found {}",
                self.describe_current()
            ))),
        }
    }

    /// Parses the `'/' relative_path` or `'//' relative_path` continuation
    /// after a filter expression.
    ///
    /// Produces an [`Expr::FilterPath`] holding the filter expression and
    /// the continuation steps; the evaluator applies the steps to the
    /// filter's node-set result. See `XPath` 1.0 section 3.3 (`PathExpr`).
    fn parse_filter_path_continuation(&mut self, filter: Expr) -> Result<Expr, XPathError> {
        let mut steps = Vec::new();

        if self.eat(&Token::DoubleSlash) {
            // '//' is shorthand for /descendant-or-self::node()/
            steps.push(Step {
                axis: Axis::DescendantOrSelf,
                node_test: NodeTest::Node,
                predicates: Vec::new(),
            });
        } else {
            self.expect(&Token::Slash)?;
        }

        // Parse the relative location path
        self.parse_relative_location_path_into(&mut steps)?;

        Ok(Expr::FilterPath {
            expr: Box::new(filter),
            steps,
        })
    }

    /// Parses a filter expression.
    ///
    /// ```text
    /// FilterExpr ::= PrimaryExpr Predicate*
    /// ```
    /// See `XPath` 1.0 section 3.3.
    fn parse_filter_expr(&mut self) -> Result<Expr, XPathError> {
        let expr = self.parse_primary_expr()?;
        let predicates = self.parse_predicates()?;

        if predicates.is_empty() {
            Ok(expr)
        } else {
            Ok(Expr::Filter {
                expr: Box::new(expr),
                predicates,
            })
        }
    }

    /// Parses a primary expression.
    ///
    /// ```text
    /// PrimaryExpr ::= VariableReference
    ///               | '(' Expr ')'
    ///               | Literal
    ///               | Number
    ///               | FunctionCall
    /// ```
    /// See `XPath` 1.0 section 3.5.
    fn parse_primary_expr(&mut self) -> Result<Expr, XPathError> {
        match self.peek().cloned() {
            Some(Token::VariableReference(name)) => {
                self.pos += 1;
                Ok(Expr::Variable(name))
            }
            Some(Token::Literal(value)) => {
                self.pos += 1;
                Ok(Expr::String(value))
            }
            Some(Token::Number(value)) => {
                self.pos += 1;
                Ok(Expr::Number(value))
            }
            Some(Token::LeftParen) => {
                self.pos += 1; // consume '('
                let expr = self.parse_expr()?;
                self.expect(&Token::RightParen)?;
                Ok(expr)
            }
            Some(Token::FunctionName(_)) => self.parse_function_call(),
            _ => Err(self.error(&format!(
                "expected primary expression, found {}",
                self.describe_current()
            ))),
        }
    }

    /// Parses a function call.
    ///
    /// ```text
    /// FunctionCall ::= FunctionName '(' (Argument (',' Argument)*)? ')'
    /// Argument ::= Expr
    /// ```
    /// See `XPath` 1.0 section 3.2.
    fn parse_function_call(&mut self) -> Result<Expr, XPathError> {
        let Some(Token::FunctionName(name)) = self.advance() else {
            return Err(self.error("expected function name"));
        };
        self.expect(&Token::LeftParen)?;

        let mut args = Vec::new();
        if !self.check(&Token::RightParen) {
            args.push(self.parse_expr()?);
            while self.eat(&Token::Comma) {
                args.push(self.parse_expr()?);
            }
        }

        self.expect(&Token::RightParen)?;

        Ok(Expr::FunctionCall { name, args })
    }

    /// Parses a location path.
    ///
    /// ```text
    /// LocationPath ::= RelativeLocationPath
    ///                | AbsoluteLocationPath
    /// AbsoluteLocationPath ::= '/' RelativeLocationPath?
    ///                        | AbbreviatedAbsoluteLocationPath
    /// AbbreviatedAbsoluteLocationPath ::= '//' RelativeLocationPath
    /// ```
    /// See `XPath` 1.0 section 2.
    fn parse_location_path(&mut self) -> Result<Expr, XPathError> {
        if self.check(&Token::Slash) {
            self.pos += 1; // consume '/'
            let mut steps = Vec::new();

            // Check if there's a relative location path after '/'
            if self.is_step_start() {
                self.parse_relative_location_path_into(&mut steps)?;
            }

            Ok(Expr::RootPath { steps })
        } else if self.eat(&Token::DoubleSlash) {
            // '//' is shorthand for '/descendant-or-self::node()/'
            let mut steps = vec![Step {
                axis: Axis::DescendantOrSelf,
                node_test: NodeTest::Node,
                predicates: Vec::new(),
            }];
            self.parse_relative_location_path_into(&mut steps)?;
            Ok(Expr::RootPath { steps })
        } else {
            // Relative location path
            let mut steps = Vec::new();
            self.parse_relative_location_path_into(&mut steps)?;
            Ok(Expr::Path { steps })
        }
    }

    /// Parses a relative location path, appending steps to the provided vector.
    ///
    /// ```text
    /// RelativeLocationPath ::= Step
    ///                        | RelativeLocationPath '/' Step
    ///                        | AbbreviatedRelativeLocationPath
    /// AbbreviatedRelativeLocationPath ::= RelativeLocationPath '//' Step
    /// ```
    /// See `XPath` 1.0 section 2.
    fn parse_relative_location_path_into(
        &mut self,
        steps: &mut Vec<Step>,
    ) -> Result<(), XPathError> {
        steps.push(self.parse_step()?);

        loop {
            if self.eat(&Token::DoubleSlash) {
                // '//' inserts a descendant-or-self::node() step
                steps.push(Step {
                    axis: Axis::DescendantOrSelf,
                    node_test: NodeTest::Node,
                    predicates: Vec::new(),
                });
                steps.push(self.parse_step()?);
            } else if self.eat(&Token::Slash) {
                // Only continue if the next token can start a step
                if self.is_step_start() {
                    steps.push(self.parse_step()?);
                } else {
                    // Trailing slash with no step -- put it back
                    self.pos -= 1;
                    break;
                }
            } else {
                break;
            }
        }

        Ok(())
    }

    /// Returns `true` if the current token can begin a location step.
    fn is_step_start(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                Token::Dot
                    | Token::DotDot
                    | Token::At
                    | Token::Name(_)
                    | Token::NodeType(_)
                    | Token::AxisName(_)
            )
        )
    }

    /// Parses a single step in a location path.
    ///
    /// ```text
    /// Step ::= AxisSpecifier NodeTest Predicate*
    ///        | AbbreviatedStep
    /// AbbreviatedStep ::= '.' | '..'
    /// ```
    /// See `XPath` 1.0 section 2.1.
    fn parse_step(&mut self) -> Result<Step, XPathError> {
        // Handle abbreviated steps
        if self.eat(&Token::Dot) {
            // '.' is shorthand for self::node()
            return Ok(Step {
                axis: Axis::Self_,
                node_test: NodeTest::Node,
                predicates: Vec::new(),
            });
        }
        if self.eat(&Token::DotDot) {
            // '..' is shorthand for parent::node()
            return Ok(Step {
                axis: Axis::Parent,
                node_test: NodeTest::Node,
                predicates: Vec::new(),
            });
        }

        // Parse axis specifier
        let axis = self.parse_axis_specifier();

        // Parse node test
        let node_test = self.parse_node_test()?;

        // Parse predicates
        let predicates = self.parse_predicates()?;

        Ok(Step {
            axis,
            node_test,
            predicates,
        })
    }

    /// Parses an axis specifier.
    ///
    /// ```text
    /// AxisSpecifier ::= AxisName '::'
    ///                 | AbbreviatedAxisSpecifier
    /// AbbreviatedAxisSpecifier ::= '@'?
    /// ```
    /// See `XPath` 1.0 section 2.2.
    fn parse_axis_specifier(&mut self) -> Axis {
        if self.eat(&Token::At) {
            // '@' is shorthand for attribute::
            return Axis::Attribute;
        }

        // Check for AxisName '::'
        if let Some(Token::AxisName(name)) = self.peek().cloned() {
            if self.tokens.get(self.pos + 1) == Some(&Token::ColonColon) {
                if let Some(axis) = Axis::parse(&name) {
                    self.pos += 2; // consume axis name and '::'
                    return axis;
                }
            }
        }

        // Default axis is child
        Axis::Child
    }

    /// Parses a node test.
    ///
    /// ```text
    /// NodeTest ::= NameTest
    ///            | NodeType '(' ')'
    ///            | 'processing-instruction' '(' Literal ')'
    /// NameTest ::= '*'
    ///            | NCName ':' '*'
    ///            | QName
    /// ```
    /// See `XPath` 1.0 section 2.3.
    fn parse_node_test(&mut self) -> Result<NodeTest, XPathError> {
        match self.peek().cloned() {
            Some(Token::NodeType(name)) => {
                self.pos += 1; // consume node type name
                self.expect(&Token::LeftParen)?;

                let node_test = match name.as_str() {
                    "node" => {
                        self.expect(&Token::RightParen)?;
                        NodeTest::Node
                    }
                    "text" => {
                        self.expect(&Token::RightParen)?;
                        NodeTest::Text
                    }
                    "comment" => {
                        self.expect(&Token::RightParen)?;
                        NodeTest::Comment
                    }
                    "processing-instruction" => {
                        // Optional literal argument
                        if let Some(Token::Literal(target)) = self.peek().cloned() {
                            self.pos += 1;
                            self.expect(&Token::RightParen)?;
                            NodeTest::ProcessingInstruction(Some(target))
                        } else {
                            self.expect(&Token::RightParen)?;
                            NodeTest::ProcessingInstruction(None)
                        }
                    }
                    _ => {
                        return Err(self.error(&format!("unknown node type: {name}")));
                    }
                };

                Ok(node_test)
            }
            Some(Token::Name(name)) => {
                self.pos += 1;
                if name == "*" {
                    Ok(NodeTest::Wildcard)
                } else if let Some(prefix) = name.strip_suffix(":*") {
                    Ok(NodeTest::PrefixWildcard(prefix.to_string()))
                } else {
                    Ok(NodeTest::Name(name))
                }
            }
            _ => Err(self.error(&format!(
                "expected node test, found {}",
                self.describe_current()
            ))),
        }
    }

    /// Parses zero or more predicates.
    ///
    /// ```text
    /// Predicate ::= '[' PredicateExpr ']'
    /// PredicateExpr ::= Expr
    /// ```
    /// See `XPath` 1.0 section 2.4.
    fn parse_predicates(&mut self) -> Result<Vec<Expr>, XPathError> {
        let mut predicates = Vec::new();
        while self.check(&Token::LeftBracket) {
            self.pos += 1; // consume '['
            let expr = self.parse_expr()?;
            self.expect(&Token::RightBracket)?;
            predicates.push(expr);
        }
        Ok(predicates)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helper for concise test assertions
    // -----------------------------------------------------------------------

    /// Parses the input and returns the AST, panicking on error.
    fn p(input: &str) -> Expr {
        parse(input).unwrap()
    }

    /// Asserts that parsing the input fails.
    fn assert_parse_error(input: &str) {
        assert!(parse(input).is_err(), "expected parse error for: {input}");
    }

    // -----------------------------------------------------------------------
    // Simple paths
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_root_only() {
        // `/` alone is a valid XPath selecting the root node
        let expr = p("/");
        match expr {
            Expr::RootPath { ref steps } => assert!(steps.is_empty()),
            _ => panic!("expected RootPath, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_absolute_path_single_step() {
        // `/root`
        let expr = p("/root");
        match expr {
            Expr::RootPath { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].axis, Axis::Child);
                assert_eq!(steps[0].node_test, NodeTest::Name("root".to_string()));
            }
            _ => panic!("expected RootPath, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_relative_path() {
        // `root/child`
        let expr = p("root/child");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 2);
                assert_eq!(steps[0].node_test, NodeTest::Name("root".to_string()));
                assert_eq!(steps[1].node_test, NodeTest::Name("child".to_string()));
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_double_slash_path() {
        // `//child`
        let expr = p("//child");
        match expr {
            Expr::RootPath { ref steps } => {
                assert_eq!(steps.len(), 2);
                // First step is the implicit descendant-or-self::node()
                assert_eq!(steps[0].axis, Axis::DescendantOrSelf);
                assert_eq!(steps[0].node_test, NodeTest::Node);
                // Second step is child::child
                assert_eq!(steps[1].axis, Axis::Child);
                assert_eq!(steps[1].node_test, NodeTest::Name("child".to_string()));
            }
            _ => panic!("expected RootPath, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Abbreviated syntax
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_dot() {
        // `.` is self::node()
        let expr = p(".");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].axis, Axis::Self_);
                assert_eq!(steps[0].node_test, NodeTest::Node);
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_dotdot() {
        // `..` is parent::node()
        let expr = p("..");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].axis, Axis::Parent);
                assert_eq!(steps[0].node_test, NodeTest::Node);
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_at_attribute() {
        // `@attr` is attribute::attr
        let expr = p("@attr");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].axis, Axis::Attribute);
                assert_eq!(steps[0].node_test, NodeTest::Name("attr".to_string()));
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_dot_double_slash_child() {
        // `.//child` is self::node() / descendant-or-self::node() / child::child
        let expr = p(".//child");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 3);
                assert_eq!(steps[0].axis, Axis::Self_);
                assert_eq!(steps[0].node_test, NodeTest::Node);
                assert_eq!(steps[1].axis, Axis::DescendantOrSelf);
                assert_eq!(steps[1].node_test, NodeTest::Node);
                assert_eq!(steps[2].axis, Axis::Child);
                assert_eq!(steps[2].node_test, NodeTest::Name("child".to_string()));
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Predicates
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_predicate_numeric() {
        // `child[1]`
        let expr = p("child[1]");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].node_test, NodeTest::Name("child".to_string()));
                assert_eq!(steps[0].predicates.len(), 1);
                match &steps[0].predicates[0] {
                    Expr::Number(n) => assert!((n - 1.0).abs() < f64::EPSILON),
                    other => panic!("expected Number predicate, got: {other:?}"),
                }
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_predicate_attribute_eq() {
        // `child[@id='x']`
        let expr = p("child[@id='x']");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].predicates.len(), 1);
                match &steps[0].predicates[0] {
                    Expr::BinaryOp { op, left, right } => {
                        assert_eq!(*op, BinaryOp::Eq);
                        // LHS should be @id (attribute::id path)
                        match left.as_ref() {
                            Expr::Path { steps } => {
                                assert_eq!(steps[0].axis, Axis::Attribute);
                                assert_eq!(steps[0].node_test, NodeTest::Name("id".to_string()));
                            }
                            other => panic!("expected Path for @id, got: {other:?}"),
                        }
                        // RHS should be 'x'
                        match right.as_ref() {
                            Expr::String(s) => assert_eq!(s, "x"),
                            other => panic!("expected String 'x', got: {other:?}"),
                        }
                    }
                    other => panic!("expected BinaryOp predicate, got: {other:?}"),
                }
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_predicate_function_call() {
        // `child[position()=1]`
        let expr = p("child[position()=1]");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].predicates.len(), 1);
                match &steps[0].predicates[0] {
                    Expr::BinaryOp { op, left, .. } => {
                        assert_eq!(*op, BinaryOp::Eq);
                        match left.as_ref() {
                            Expr::FunctionCall { name, args } => {
                                assert_eq!(name, "position");
                                assert!(args.is_empty());
                            }
                            other => panic!("expected FunctionCall, got: {other:?}"),
                        }
                    }
                    other => panic!("expected BinaryOp, got: {other:?}"),
                }
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Operators
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_addition() {
        // `1 + 2`
        let expr = p("1 + 2");
        match expr {
            Expr::BinaryOp { op, left, right } => {
                assert_eq!(op, BinaryOp::Add);
                match (*left, *right) {
                    (Expr::Number(l), Expr::Number(r)) => {
                        assert!((l - 1.0).abs() < f64::EPSILON);
                        assert!((r - 2.0).abs() < f64::EPSILON);
                    }
                    (l, r) => panic!("expected Number operands, got: {l:?}, {r:?}"),
                }
            }
            _ => panic!("expected BinaryOp, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_equality() {
        // `a = b`
        let expr = p("a = b");
        match expr {
            Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::Eq),
            _ => panic!("expected BinaryOp Eq, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_and() {
        // `a and b`
        let expr = p("a and b");
        match expr {
            Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::And),
            _ => panic!("expected BinaryOp And, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_or() {
        // `a or b`
        let expr = p("a or b");
        match expr {
            Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::Or),
            _ => panic!("expected BinaryOp Or, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Function calls
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_function_call_count() {
        // `count(//node)`
        let expr = p("count(//node)");
        match expr {
            Expr::FunctionCall { name, args } => {
                assert_eq!(name, "count");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("expected FunctionCall, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_function_call_string_length() {
        // `string-length('hello')`
        let expr = p("string-length('hello')");
        match expr {
            Expr::FunctionCall { name, args } => {
                assert_eq!(name, "string-length");
                assert_eq!(args.len(), 1);
                match &args[0] {
                    Expr::String(s) => assert_eq!(s, "hello"),
                    other => panic!("expected String arg, got: {other:?}"),
                }
            }
            _ => panic!("expected FunctionCall, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_function_call_concat() {
        // `concat('a', 'b', 'c')`
        let expr = p("concat('a', 'b', 'c')");
        match expr {
            Expr::FunctionCall { name, args } => {
                assert_eq!(name, "concat");
                assert_eq!(args.len(), 3);
                for (i, expected) in ["a", "b", "c"].iter().enumerate() {
                    match &args[i] {
                        Expr::String(s) => assert_eq!(s, *expected),
                        other => panic!("expected String arg, got: {other:?}"),
                    }
                }
            }
            _ => panic!("expected FunctionCall, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Complex expressions
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_complex_predicate_with_comparison() {
        // `//book[@price > 10.00]`
        let expr = p("//book[@price > 10.00]");
        match expr {
            Expr::RootPath { ref steps } => {
                // descendant-or-self::node(), child::book[@price > 10]
                assert_eq!(steps.len(), 2);
                assert_eq!(steps[1].node_test, NodeTest::Name("book".to_string()));
                assert_eq!(steps[1].predicates.len(), 1);
                match &steps[1].predicates[0] {
                    Expr::BinaryOp { op, .. } => assert_eq!(*op, BinaryOp::Gt),
                    other => panic!("expected BinaryOp Gt, got: {other:?}"),
                }
            }
            _ => panic!("expected RootPath, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_complex_path_with_last() {
        // `/root/child[last()]/text()`
        let expr = p("/root/child[last()]/text()");
        match expr {
            Expr::RootPath { ref steps } => {
                assert_eq!(steps.len(), 3);
                assert_eq!(steps[0].node_test, NodeTest::Name("root".to_string()));
                assert_eq!(steps[1].node_test, NodeTest::Name("child".to_string()));
                assert_eq!(steps[1].predicates.len(), 1);
                match &steps[1].predicates[0] {
                    Expr::FunctionCall { name, args } => {
                        assert_eq!(name, "last");
                        assert!(args.is_empty());
                    }
                    other => panic!("expected FunctionCall last(), got: {other:?}"),
                }
                assert_eq!(steps[2].node_test, NodeTest::Text);
            }
            _ => panic!("expected RootPath, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Union
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_union() {
        // `a | b`
        let expr = p("a | b");
        match expr {
            Expr::Union(left, right) => {
                match *left {
                    Expr::Path { ref steps } => {
                        assert_eq!(steps[0].node_test, NodeTest::Name("a".to_string()));
                    }
                    _ => panic!("expected Path for left union operand"),
                }
                match *right {
                    Expr::Path { ref steps } => {
                        assert_eq!(steps[0].node_test, NodeTest::Name("b".to_string()));
                    }
                    _ => panic!("expected Path for right union operand"),
                }
            }
            _ => panic!("expected Union, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Variables
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_variable_plus_number() {
        // `$x + 1`
        let expr = p("$x + 1");
        match expr {
            Expr::BinaryOp { op, left, right } => {
                assert_eq!(op, BinaryOp::Add);
                match *left {
                    Expr::Variable(ref name) => assert_eq!(name, "x"),
                    _ => panic!("expected Variable"),
                }
                match *right {
                    Expr::Number(n) => assert!((n - 1.0).abs() < f64::EPSILON),
                    _ => panic!("expected Number"),
                }
            }
            _ => panic!("expected BinaryOp, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Nested/parenthesized expressions
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_parenthesized_expr() {
        // `(1 + 2) * 3`
        let expr = p("(1 + 2) * 3");
        match expr {
            Expr::BinaryOp { op, left, right } => {
                assert_eq!(op, BinaryOp::Mul);
                match *left {
                    Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::Add),
                    _ => panic!("expected inner BinaryOp Add"),
                }
                match *right {
                    Expr::Number(n) => assert!((n - 3.0).abs() < f64::EPSILON),
                    _ => panic!("expected Number 3"),
                }
            }
            _ => panic!("expected BinaryOp, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Processing instructions
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_processing_instruction_with_target() {
        // `processing-instruction('xml-stylesheet')`
        let expr = p("processing-instruction('xml-stylesheet')");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(
                    steps[0].node_test,
                    NodeTest::ProcessingInstruction(Some("xml-stylesheet".to_string()))
                );
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_processing_instruction_no_target() {
        // `processing-instruction()`
        let expr = p("processing-instruction()");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].node_test, NodeTest::ProcessingInstruction(None));
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Wildcards
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_wildcard() {
        // `*` matches any element
        let expr = p("*");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].node_test, NodeTest::Wildcard);
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Multiple predicates
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_multiple_predicates() {
        // `child[1][@type='x']`
        let expr = p("child[1][@type='x']");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].predicates.len(), 2);
                match &steps[0].predicates[0] {
                    Expr::Number(n) => assert!((n - 1.0).abs() < f64::EPSILON),
                    other => panic!("expected Number predicate, got: {other:?}"),
                }
                match &steps[0].predicates[1] {
                    Expr::BinaryOp { op, .. } => assert_eq!(*op, BinaryOp::Eq),
                    other => panic!("expected BinaryOp Eq predicate, got: {other:?}"),
                }
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Operator precedence
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_operator_precedence() {
        // `1 + 2 * 3` should be `1 + (2 * 3)` due to precedence
        let expr = p("1 + 2 * 3");
        match expr {
            Expr::BinaryOp { op, left, right } => {
                assert_eq!(op, BinaryOp::Add);
                match *left {
                    Expr::Number(n) => assert!((n - 1.0).abs() < f64::EPSILON),
                    _ => panic!("expected Number 1"),
                }
                match *right {
                    Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::Mul),
                    _ => panic!("expected inner Mul"),
                }
            }
            _ => panic!("expected BinaryOp, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Explicit axis syntax
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_explicit_axis() {
        // `descendant::div`
        let expr = p("descendant::div");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps.len(), 1);
                assert_eq!(steps[0].axis, Axis::Descendant);
                assert_eq!(steps[0].node_test, NodeTest::Name("div".to_string()));
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Error cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_empty_expression_error() {
        assert_parse_error("");
    }

    #[test]
    fn test_parse_unexpected_token_error() {
        assert_parse_error(")");
    }

    #[test]
    fn test_parse_unclosed_paren_error() {
        assert_parse_error("(1 + 2");
    }

    #[test]
    fn test_parse_unclosed_bracket_error() {
        assert_parse_error("child[1");
    }

    // -----------------------------------------------------------------------
    // Additional coverage
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_unary_negation() {
        // `-5`
        let expr = p("-5");
        match expr {
            Expr::UnaryNeg(inner) => match *inner {
                Expr::Number(n) => assert!((n - 5.0).abs() < f64::EPSILON),
                _ => panic!("expected Number inside UnaryNeg"),
            },
            _ => panic!("expected UnaryNeg, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_double_negation() {
        // `--5`
        let expr = p("--5");
        match expr {
            Expr::UnaryNeg(inner) => match *inner {
                Expr::UnaryNeg(_) => {} // correct: double negation
                _ => panic!("expected nested UnaryNeg"),
            },
            _ => panic!("expected UnaryNeg, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_string_literal() {
        let expr = p("'hello'");
        match expr {
            Expr::String(s) => assert_eq!(s, "hello"),
            _ => panic!("expected String, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_number_literal() {
        let expr = p("42.5");
        match expr {
            Expr::Number(n) => assert!((n - 42.5).abs() < f64::EPSILON),
            _ => panic!("expected Number, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_comment_node_test() {
        let expr = p("comment()");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps[0].node_test, NodeTest::Comment);
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_node_node_test() {
        let expr = p("node()");
        match expr {
            Expr::Path { ref steps } => {
                assert_eq!(steps[0].node_test, NodeTest::Node);
            }
            _ => panic!("expected Path, got: {expr:?}"),
        }
    }

    #[test]
    fn test_parse_or_precedence_over_and() {
        // `a and b or c and d` should be `(a and b) or (c and d)`
        let expr = p("a and b or c and d");
        match expr {
            Expr::BinaryOp { op, left, right } => {
                assert_eq!(op, BinaryOp::Or);
                match *left {
                    Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::And),
                    _ => panic!("expected left And"),
                }
                match *right {
                    Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::And),
                    _ => panic!("expected right And"),
                }
            }
            _ => panic!("expected BinaryOp Or, got: {expr:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Filter-path expressions (issue #20)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_filter_path_continuation() {
        // `(//a)[1]/@href`: a filter expression followed by a relative
        // location path must parse to FilterPath, not to a predicate.
        let (inner, steps) = match p("(//a)[1]/@href") {
            Expr::FilterPath { expr, steps } => (expr, steps),
            other => panic!("expected FilterPath, got: {other:?}"),
        };
        match *inner {
            Expr::Filter { ref predicates, .. } => assert_eq!(predicates.len(), 1),
            ref other => panic!("expected inner Filter, got: {other:?}"),
        }
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].axis, Axis::Attribute);
        assert_eq!(steps[0].node_test, NodeTest::Name("href".to_string()));
    }

    #[test]
    fn test_parse_filter_path_vs_predicate_distinct() {
        // `(//a)/b` (path continuation) and `(//a)[b]` (predicate) must
        // produce different ASTs — they mean different things.
        let path = p("(//a)/b");
        assert!(
            matches!(path, Expr::FilterPath { .. }),
            "expected FilterPath, got: {path:?}"
        );
        let filter = p("(//a)[b]");
        assert!(
            matches!(filter, Expr::Filter { .. }),
            "expected Filter, got: {filter:?}"
        );
    }
}
