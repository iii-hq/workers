//! Tokenizer-driven parser, the AST node types, and the `parse` entry point.
//!
//! The parser turns a CSS group of selectors into a list of [`Selector`]
//! values. Each selector wraps a [`Tree`] and an optional pseudo-element. The
//! tree nodes carry the `repr`, `canonical`, and `specificity` behavior the
//! tests pin.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::SelectorError;
use crate::tokenizer::{tokenize, Token, TokenType};

/// A specificity triple `(a, b, c)`: IDs, then classes and attributes and
/// pseudo-classes, then types and pseudo-elements.
pub type Specificity = (u32, u32, u32);

/// A pseudo-element on a selector: a plain identifier or a functional form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PseudoElement {
    /// A plain pseudo-element identifier such as `before`.
    Ident(String),
    /// A functional pseudo-element such as `attr(name)`.
    Functional(FunctionalPseudoElement),
}

/// A functional pseudo-element such as `::name(args)`.
///
/// The name is ASCII lower-cased. The arguments are the raw tokens between the
/// parentheses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionalPseudoElement {
    /// The pseudo-element name, ASCII lower-cased.
    pub name: String,
    /// The argument tokens.
    pub arguments: Vec<Token>,
}

impl FunctionalPseudoElement {
    /// Build a functional pseudo-element, lower-casing the name.
    pub fn new(name: &str, arguments: Vec<Token>) -> FunctionalPseudoElement {
        FunctionalPseudoElement {
            name: name.to_ascii_lowercase(),
            arguments,
        }
    }

    /// The type names of the argument tokens.
    pub fn argument_types(&self) -> Vec<&'static str> {
        self.arguments.iter().map(|t| t.ty.name()).collect()
    }

    /// The CSS serialization, such as `attr(name)`.
    pub fn canonical(&self) -> String {
        let args: String = self.arguments.iter().map(|t| t.to_css()).collect();
        format!("{}({})", self.name, args)
    }

    /// The Python `repr` form used inside selector `repr` output.
    pub fn repr(&self) -> String {
        format!(
            "FunctionalPseudoElement[::{}({})]",
            self.name,
            repr_token_values(&self.arguments)
        )
    }
}

/// Render a token-value list the way Python renders `[t.value for t in args]`.
fn repr_token_values(tokens: &[Token]) -> String {
    let mut out = String::from("[");
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&crate::util::py_repr(t.value_str()));
    }
    out.push(']');
    out
}

/// A parsed selector tree node.
///
/// The variants cover every selector construct the grammar accepts. Each node
/// answers `repr`, `canonical`, and `specificity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tree {
    /// `namespace|element`, with `None` element meaning the universal `*`.
    Element {
        /// The namespace prefix, if any.
        namespace: Option<String>,
        /// The element name, or `None` for `*`.
        element: Option<String>,
    },
    /// `selector#id`.
    Hash {
        /// The selector this hash applies to.
        selector: Box<Tree>,
        /// The id value.
        id: String,
    },
    /// `selector.class_name`.
    Class {
        /// The selector this class applies to.
        selector: Box<Tree>,
        /// The class name.
        class_name: String,
    },
    /// `selector[ns|attrib op value]`.
    Attrib {
        /// The selector this attribute test applies to.
        selector: Box<Tree>,
        /// The attribute namespace, if any.
        namespace: Option<String>,
        /// The attribute name.
        attrib: String,
        /// The operator, such as `=`, `^=`, or `exists`.
        operator: String,
        /// The value token, or `None` for the `exists` operator.
        value: Option<Token>,
    },
    /// `selector:ident`.
    Pseudo {
        /// The selector this pseudo-class applies to.
        selector: Box<Tree>,
        /// The pseudo-class identifier, ASCII lower-cased.
        ident: String,
    },
    /// `selector:name(args)`.
    Function {
        /// The selector this functional pseudo-class applies to.
        selector: Box<Tree>,
        /// The function name, ASCII lower-cased.
        name: String,
        /// The argument tokens.
        arguments: Vec<Token>,
    },
    /// `selector:not(subselector)`.
    Negation {
        /// The selector being negated against.
        selector: Box<Tree>,
        /// The negated subselector.
        subselector: Box<Tree>,
    },
    /// `selector:has(subselector)`.
    Relation {
        /// The selector this relation applies to.
        selector: Box<Tree>,
        /// The combinator token.
        combinator: Token,
        /// The relative subselector.
        subselector: Box<Selector>,
    },
    /// `selector:is(selector_list)`, also `:matches`.
    Matching {
        /// The selector this match applies to.
        selector: Box<Tree>,
        /// The selector list.
        selector_list: Vec<Tree>,
    },
    /// `selector:where(selector_list)`.
    SpecificityAdjustment {
        /// The selector this adjustment applies to.
        selector: Box<Tree>,
        /// The selector list.
        selector_list: Vec<Tree>,
    },
    /// `selector combinator subselector`.
    Combined {
        /// The left selector.
        selector: Box<Tree>,
        /// The combinator, one of `" "`, `">"`, `"+"`, `"~"`.
        combinator: String,
        /// The right selector.
        subselector: Box<Tree>,
    },
}

impl Tree {
    /// The Python `repr` of this node.
    pub fn repr(&self) -> String {
        match self {
            Tree::Element { .. } => format!("Element[{}]", self.canonical()),
            Tree::Hash { selector, id } => format!("Hash[{}#{}]", selector.repr(), id),
            Tree::Class {
                selector,
                class_name,
            } => format!("Class[{}.{}]", selector.repr(), class_name),
            Tree::Attrib {
                selector,
                namespace,
                attrib,
                operator,
                value,
            } => {
                let attr = match namespace {
                    Some(ns) => format!("{ns}|{attrib}"),
                    None => attrib.clone(),
                };
                if operator == "exists" {
                    format!("Attrib[{}[{}]]", selector.repr(), attr)
                } else {
                    let v = value.as_ref().map(|t| t.value_str()).unwrap_or("");
                    format!(
                        "Attrib[{}[{} {} {}]]",
                        selector.repr(),
                        attr,
                        operator,
                        crate::util::py_repr(v)
                    )
                }
            }
            Tree::Pseudo { selector, ident } => {
                format!("Pseudo[{}:{}]", selector.repr(), ident)
            }
            Tree::Function {
                selector,
                name,
                arguments,
            } => format!(
                "Function[{}:{}({})]",
                selector.repr(),
                name,
                repr_token_values(arguments)
            ),
            Tree::Negation {
                selector,
                subselector,
            } => format!("Negation[{}:not({})]", selector.repr(), subselector.repr()),
            Tree::Relation {
                selector,
                subselector,
                ..
            } => format!("Relation[{}:has({})]", selector.repr(), subselector.repr()),
            Tree::Matching {
                selector,
                selector_list,
            } => {
                let inner = join_reprs(selector_list);
                format!("Matching[{}:is({})]", selector.repr(), inner)
            }
            Tree::SpecificityAdjustment {
                selector,
                selector_list,
            } => {
                let inner = join_reprs(selector_list);
                format!(
                    "SpecificityAdjustment[{}:where({})]",
                    selector.repr(),
                    inner
                )
            }
            Tree::Combined {
                selector,
                combinator,
                subselector,
            } => {
                let comb = if combinator == " " {
                    "<followed>"
                } else {
                    combinator.as_str()
                };
                format!(
                    "CombinedSelector[{} {} {}]",
                    selector.repr(),
                    comb,
                    subselector.repr()
                )
            }
        }
    }

    /// The CSS serialization of this node.
    pub fn canonical(&self) -> String {
        match self {
            Tree::Element { namespace, element } => {
                let el = element.as_deref().unwrap_or("*");
                match namespace {
                    Some(ns) => format!("{ns}|{el}"),
                    None => el.to_string(),
                }
            }
            Tree::Hash { selector, id } => format!("{}#{}", selector.canonical(), id),
            Tree::Class {
                selector,
                class_name,
            } => format!("{}.{}", selector.canonical(), class_name),
            Tree::Attrib {
                selector,
                namespace,
                attrib,
                operator,
                value,
            } => {
                let attr = match namespace {
                    Some(ns) => format!("{ns}|{attrib}"),
                    None => attrib.clone(),
                };
                let op = if operator == "exists" {
                    attr
                } else {
                    let v = value.as_ref().map(|t| t.to_css()).unwrap_or_default();
                    format!("{attr}{operator}{v}")
                };
                format!("{}[{}]", selector.canonical(), op)
            }
            Tree::Pseudo { selector, ident } => {
                format!("{}:{}", selector.canonical(), ident)
            }
            Tree::Function {
                selector,
                name,
                arguments,
            } => {
                let args: String = arguments.iter().map(|t| t.to_css()).collect();
                format!("{}:{}({})", selector.canonical(), name, args)
            }
            Tree::Negation {
                selector,
                subselector,
            } => {
                let mut subsel = subselector.canonical();
                if subsel.chars().count() > 1 {
                    subsel = lstrip_star(&subsel);
                }
                format!("{}:not({})", selector.canonical(), subsel)
            }
            Tree::Relation {
                selector,
                subselector,
                ..
            } => {
                let mut subsel = subselector.canonical();
                if subsel.chars().count() > 1 {
                    subsel = lstrip_star(&subsel);
                }
                format!("{}:has({})", selector.canonical(), subsel)
            }
            Tree::Matching {
                selector,
                selector_list,
            } => {
                let inner = join_canonical_stripped(selector_list);
                format!("{}:is({})", selector.canonical(), inner)
            }
            Tree::SpecificityAdjustment {
                selector,
                selector_list,
            } => {
                let inner = join_canonical_stripped(selector_list);
                format!("{}:where({})", selector.canonical(), inner)
            }
            Tree::Combined {
                selector,
                combinator,
                subselector,
            } => {
                let mut subsel = subselector.canonical();
                if subsel.chars().count() > 1 {
                    subsel = lstrip_star(&subsel);
                }
                format!("{} {} {}", selector.canonical(), combinator, subsel)
            }
        }
    }

    /// The specificity triple of this node.
    pub fn specificity(&self) -> Specificity {
        match self {
            Tree::Element { element, .. } => {
                if element.is_some() {
                    (0, 0, 1)
                } else {
                    (0, 0, 0)
                }
            }
            Tree::Hash { selector, .. } => {
                let (a, b, c) = selector.specificity();
                (a + 1, b, c)
            }
            Tree::Class { selector, .. }
            | Tree::Attrib { selector, .. }
            | Tree::Pseudo { selector, .. }
            | Tree::Function { selector, .. } => {
                let (a, b, c) = selector.specificity();
                (a, b + 1, c)
            }
            Tree::Negation {
                selector,
                subselector,
            } => add_spec(selector.specificity(), subselector.specificity()),
            Tree::Relation {
                selector,
                subselector,
                ..
            } => add_spec(
                selector.specificity(),
                subselector.parsed_tree.specificity(),
            ),
            Tree::Matching {
                selector,
                selector_list,
            } => add_spec(selector.specificity(), max_spec(selector_list)),
            Tree::SpecificityAdjustment { selector, .. } => selector.specificity(),
            Tree::Combined {
                selector,
                subselector,
                ..
            } => add_spec(selector.specificity(), subselector.specificity()),
        }
    }
}

/// Strip leading `*` characters, matching Python `str.lstrip("*")`.
fn lstrip_star(s: &str) -> String {
    s.trim_start_matches('*').to_string()
}

/// Component-wise sum of two specificity triples.
fn add_spec(x: Specificity, y: Specificity) -> Specificity {
    (x.0 + y.0, x.1 + y.1, x.2 + y.2)
}

/// Lexicographic maximum of a selector list's specificity triples.
fn max_spec(list: &[Tree]) -> Specificity {
    list.iter()
        .map(|t| t.specificity())
        .max()
        .unwrap_or((0, 0, 0))
}

/// Join the `repr` of each tree with `, `.
fn join_reprs(list: &[Tree]) -> String {
    let mut out = String::new();
    for (i, t) in list.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&t.repr());
    }
    out
}

/// Join each tree's canonical form with `, `, stripping a leading `*` from each.
fn join_canonical_stripped(list: &[Tree]) -> String {
    let mut out = String::new();
    for (i, t) in list.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&lstrip_star(&t.canonical()));
    }
    out
}

/// A parsed selector: a tree plus an optional pseudo-element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// The parsed selector tree.
    pub parsed_tree: Tree,
    /// The pseudo-element, if any.
    pub pseudo_element: Option<PseudoElement>,
}

impl Selector {
    /// Build a selector, ASCII lower-casing a plain pseudo-element name.
    pub fn new(tree: Tree, pseudo_element: Option<PseudoElement>) -> Selector {
        let pseudo_element = pseudo_element.map(|pe| match pe {
            PseudoElement::Ident(name) => PseudoElement::Ident(name.to_ascii_lowercase()),
            other => other,
        });
        Selector {
            parsed_tree: tree,
            pseudo_element,
        }
    }

    /// The Python `repr` of this selector.
    pub fn repr(&self) -> String {
        let pe = match &self.pseudo_element {
            Some(PseudoElement::Functional(f)) => f.repr(),
            Some(PseudoElement::Ident(name)) if !name.is_empty() => format!("::{name}"),
            _ => String::new(),
        };
        format!("Selector[{}{}]", self.parsed_tree.repr(), pe)
    }

    /// The CSS serialization of this selector.
    ///
    /// A leading `*` is stripped when the result is longer than one character.
    pub fn canonical(&self) -> String {
        let pe = match &self.pseudo_element {
            Some(PseudoElement::Functional(f)) => format!("::{}", f.canonical()),
            Some(PseudoElement::Ident(name)) if !name.is_empty() => format!("::{name}"),
            _ => String::new(),
        };
        let mut res = format!("{}{}", self.parsed_tree.canonical(), pe);
        if res.chars().count() > 1 {
            res = lstrip_star(&res);
        }
        res
    }

    /// The specificity of this selector. A pseudo-element adds one to `c`.
    pub fn specificity(&self) -> Specificity {
        let (a, b, mut c) = self.parsed_tree.specificity();
        if self.pseudo_element.is_some() {
            c += 1;
        }
        (a, b, c)
    }
}

/// Parse the arguments for `:nth-child()` and friends into an `(a, b)` pair.
///
/// Returns an error when a string token appears or a coefficient does not parse
/// as an integer. The caller turns the error into an expression error.
pub fn parse_series(tokens: &[Token]) -> Result<(i64, i64), SelectorError> {
    for t in tokens {
        if t.ty == TokenType::String {
            return Err(SelectorError::Syntax(
                "String tokens not allowed in series.".to_string(),
            ));
        }
    }
    let joined: String = tokens.iter().map(|t| t.value_str()).collect();
    let s = joined.trim();
    match s {
        "odd" => return Ok((2, 1)),
        "even" => return Ok((2, 0)),
        "n" => return Ok((1, 0)),
        _ => {}
    }
    if !s.contains('n') {
        let b = parse_int(s)?;
        return Ok((0, b));
    }
    let idx = s
        .find('n')
        .expect("series text contains 'n' on this branch");
    let a_part = &s[..idx];
    let b_part = &s[idx + 1..];
    let a = if a_part.is_empty() {
        1
    } else if a_part == "-" || a_part == "+" {
        parse_int(&format!("{a_part}1"))?
    } else {
        parse_int(a_part)?
    };
    let b = if b_part.is_empty() {
        0
    } else {
        parse_int(b_part)?
    };
    Ok((a, b))
}

/// Parse a signed integer with the same acceptance as Python `int(str)`.
///
/// A single leading `+` or `-` is allowed, then digits. No surrounding
/// whitespace, since the series text was already trimmed. A value that does not
/// fit in `i64` returns a series error rather than overflowing, so a long digit
/// run in an `:nth-*` argument cannot panic the translator.
fn parse_int(s: &str) -> Result<i64, SelectorError> {
    let digits = match s.as_bytes().first() {
        Some(b'+') | Some(b'-') => &s[1..],
        _ => s,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(series_error());
    }
    s.parse::<i64>().map_err(|_| series_error())
}

/// The error raised when a series coefficient does not parse.
fn series_error() -> SelectorError {
    SelectorError::Syntax("Invalid series".to_string())
}

/// The token stream the parser reads from.
///
/// `used` records consumed tokens. The `:scope` placement guard and the
/// "Expected selector" check both depend on its length. Peeking does not add to
/// `used`.
struct TokenStream {
    tokens: Vec<Token>,
    index: usize,
    used: Vec<Token>,
}

impl TokenStream {
    fn new(tokens: Vec<Token>) -> TokenStream {
        TokenStream {
            tokens,
            index: 0,
            used: Vec::new(),
        }
    }

    /// Consume and return the next token, recording it in `used`.
    fn next(&mut self) -> Token {
        let token = self.peek_token();
        self.index += 1;
        self.used.push(token.clone());
        token
    }

    /// Look at the next token without consuming it.
    fn peek(&self) -> Token {
        self.peek_token()
    }

    /// The next token, clamped to the final EOF token.
    fn peek_token(&self) -> Token {
        if self.index < self.tokens.len() {
            self.tokens[self.index].clone()
        } else {
            self.tokens[self.tokens.len() - 1].clone()
        }
    }

    /// Consume an identifier or report an error.
    fn next_ident(&mut self) -> Result<String, SelectorError> {
        let t = self.next();
        if t.ty != TokenType::Ident {
            return Err(SelectorError::Syntax(format!("Expected ident, got {t}")));
        }
        Ok(t.value_str().to_string())
    }

    /// Consume an identifier or `*`, returning `None` for `*`.
    fn next_ident_or_star(&mut self) -> Result<Option<String>, SelectorError> {
        let t = self.next();
        if t.ty == TokenType::Ident {
            Ok(Some(t.value_str().to_string()))
        } else if t.matches(TokenType::Delim, "*") {
            Ok(None)
        } else {
            Err(SelectorError::Syntax(format!(
                "Expected ident or '*', got {t}"
            )))
        }
    }

    /// Skip a single whitespace token if present.
    fn skip_whitespace(&mut self) {
        if self.peek().ty == TokenType::S {
            self.next();
        }
    }
}

/// Parse a CSS group of selectors into one [`Selector`] per comma-separated part.
///
/// Returns a [`SelectorError::Syntax`] on an invalid selector.
pub fn parse(css: &str) -> Result<Vec<Selector>, SelectorError> {
    // Fast paths for the most common simple selectors. They produce the same
    // trees as the full parser but skip tokenizing and never error on the
    // whitespace-padded forms.
    if let Some(el) = fast_element(css) {
        return Ok(alloc::vec![Selector::new(
            Tree::Element {
                namespace: None,
                element: Some(el),
            },
            None
        )]);
    }
    if let Some((el, id)) = fast_hash(css) {
        let element = if el.is_empty() { None } else { Some(el) };
        return Ok(alloc::vec![Selector::new(
            Tree::Hash {
                selector: Box::new(Tree::Element {
                    namespace: None,
                    element,
                }),
                id,
            },
            None
        )]);
    }
    if let Some((el, class)) = fast_class(css) {
        let element = if el.is_empty() { None } else { Some(el) };
        return Ok(alloc::vec![Selector::new(
            Tree::Class {
                selector: Box::new(Tree::Element {
                    namespace: None,
                    element,
                }),
                class_name: class,
            },
            None
        )]);
    }

    let tokens = tokenize(css)?;
    let mut stream = TokenStream::new(tokens);
    parse_selector_group(&mut stream)
}

/// Match the `_el_re` fast path: optional whitespace, ASCII letters, whitespace.
fn fast_element(css: &str) -> Option<String> {
    let body = css.trim_matches(is_fast_ws);
    if !body.is_empty() && body.bytes().all(|b| b.is_ascii_alphabetic()) {
        Some(body.to_string())
    } else {
        None
    }
}

/// Match the `_id_re` fast path: `[a-zA-Z]*#[a-zA-Z0-9_-]+`.
fn fast_hash(css: &str) -> Option<(String, String)> {
    let body = css.trim_matches(is_fast_ws);
    let hash = body.find('#')?;
    let el = &body[..hash];
    let id = &body[hash + 1..];
    if el.bytes().all(|b| b.is_ascii_alphabetic()) && !id.is_empty() && id.bytes().all(is_id_char) {
        Some((el.to_string(), id.to_string()))
    } else {
        None
    }
}

/// Match the `_class_re` fast path: `[a-zA-Z]*\.[a-zA-Z][a-zA-Z0-9_-]*`.
fn fast_class(css: &str) -> Option<(String, String)> {
    let body = css.trim_matches(is_fast_ws);
    let dot = body.find('.')?;
    let el = &body[..dot];
    let class = &body[dot + 1..];
    let cb = class.as_bytes();
    if el.bytes().all(|b| b.is_ascii_alphabetic())
        && !class.is_empty()
        && cb[0].is_ascii_alphabetic()
        && class.bytes().all(is_id_char)
    {
        Some((el.to_string(), class.to_string()))
    } else {
        None
    }
}

/// Whitespace characters the fast-path regexes trim.
fn is_fast_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{0c}')
}

/// Characters allowed in the id and class fast paths after the first.
fn is_id_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Parse a comma-separated group of selectors.
fn parse_selector_group(stream: &mut TokenStream) -> Result<Vec<Selector>, SelectorError> {
    let mut selectors = Vec::new();
    stream.skip_whitespace();
    loop {
        let (tree, pseudo) = parse_selector(stream)?;
        selectors.push(Selector::new(tree, pseudo));
        if stream.peek().matches(TokenType::Delim, ",") {
            stream.next();
            stream.skip_whitespace();
        } else {
            break;
        }
    }
    Ok(selectors)
}

/// Parse one selector, including combinators.
fn parse_selector(
    stream: &mut TokenStream,
) -> Result<(Tree, Option<PseudoElement>), SelectorError> {
    let (mut result, mut pseudo_element) = parse_simple_selector(stream, false)?;
    loop {
        stream.skip_whitespace();
        let peek = stream.peek();
        if peek.matches(TokenType::Eof, "") || peek.matches(TokenType::Delim, ",") {
            break;
        }
        if let Some(pe) = &pseudo_element {
            return Err(not_at_end_error(pe));
        }
        let combinator = if peek.is_delim(&["+", ">", "~"]) {
            let c = stream.next().value_str().to_string();
            stream.skip_whitespace();
            c
        } else {
            " ".to_string()
        };
        let (next_selector, next_pseudo) = parse_simple_selector(stream, false)?;
        pseudo_element = next_pseudo;
        result = Tree::Combined {
            selector: Box::new(result),
            combinator,
            subselector: Box::new(next_selector),
        };
    }
    Ok((result, pseudo_element))
}

/// The error for a pseudo-element that is not at the end of a selector.
fn not_at_end_error(pe: &PseudoElement) -> SelectorError {
    SelectorError::Syntax(format!(
        "Got pseudo-element ::{} not at the end of a selector",
        pseudo_element_name(pe)
    ))
}

/// The display name of a pseudo-element for error messages.
fn pseudo_element_name(pe: &PseudoElement) -> String {
    match pe {
        PseudoElement::Ident(name) => name.clone(),
        PseudoElement::Functional(f) => f.repr(),
    }
}

/// Parse a simple selector: an optional type or universal selector followed by
/// any number of qualifiers (hash, class, attribute, pseudo-class).
fn parse_simple_selector(
    stream: &mut TokenStream,
    inside_negation: bool,
) -> Result<(Tree, Option<PseudoElement>), SelectorError> {
    stream.skip_whitespace();
    let selector_start = stream.used.len();
    let peek = stream.peek();

    let (mut namespace, mut element): (Option<String>, Option<String>) =
        if peek.ty == TokenType::Ident || peek.matches(TokenType::Delim, "*") {
            let ns = if peek.ty == TokenType::Ident {
                Some(stream.next().value_str().to_string())
            } else {
                stream.next();
                None
            };
            if stream.peek().matches(TokenType::Delim, "|") {
                stream.next();
                let el = stream.next_ident_or_star()?;
                (ns, el)
            } else {
                (None, ns)
            }
        } else {
            (None, None)
        };

    let mut result = Tree::Element {
        namespace: namespace.take(),
        element: element.take(),
    };
    let mut pseudo_element: Option<PseudoElement> = None;

    loop {
        let peek = stream.peek();
        if peek.ty == TokenType::S
            || peek.ty == TokenType::Eof
            || peek.is_delim(&[",", "+", ">", "~"])
            || (inside_negation && peek.matches(TokenType::Delim, ")"))
        {
            break;
        }
        if let Some(pe) = &pseudo_element {
            return Err(not_at_end_error(pe));
        }
        if peek.ty == TokenType::Hash {
            let id = stream.next().value_str().to_string();
            result = Tree::Hash {
                selector: Box::new(result),
                id,
            };
        } else if peek.matches(TokenType::Delim, ".") {
            stream.next();
            let class_name = stream.next_ident()?;
            result = Tree::Class {
                selector: Box::new(result),
                class_name,
            };
        } else if peek.matches(TokenType::Delim, "|") {
            stream.next();
            let el = stream.next_ident()?;
            result = Tree::Element {
                namespace: None,
                element: Some(el),
            };
        } else if peek.matches(TokenType::Delim, "[") {
            stream.next();
            result = parse_attrib(result, stream)?;
        } else if peek.matches(TokenType::Delim, ":") {
            stream.next();
            if stream.peek().matches(TokenType::Delim, ":") {
                stream.next();
                let name = stream.next_ident()?;
                if stream.peek().matches(TokenType::Delim, "(") {
                    stream.next();
                    let args = parse_arguments(stream)?;
                    pseudo_element = Some(PseudoElement::Functional(FunctionalPseudoElement::new(
                        &name, args,
                    )));
                } else {
                    pseudo_element = Some(PseudoElement::Ident(name));
                }
                continue;
            }
            let ident = stream.next_ident()?;
            let lowered = ident.to_ascii_lowercase();
            if matches!(
                lowered.as_str(),
                "first-line" | "first-letter" | "before" | "after"
            ) {
                pseudo_element = Some(PseudoElement::Ident(ident));
                continue;
            }
            if !stream.peek().matches(TokenType::Delim, "(") {
                result = Tree::Pseudo {
                    selector: Box::new(result),
                    ident: lowered.clone(),
                };
                if result.repr() == "Pseudo[Element[*]:scope]" && !scope_allowed(stream) {
                    return Err(SelectorError::Syntax(
                        "Got immediate child pseudo-element \":scope\" not at the start of a selector"
                            .to_string(),
                    ));
                }
                continue;
            }
            stream.next();
            stream.skip_whitespace();
            match lowered.as_str() {
                "not" => {
                    if inside_negation {
                        return Err(SelectorError::Syntax("Got nested :not()".to_string()));
                    }
                    let (argument, arg_pseudo) = parse_simple_selector(stream, true)?;
                    let next_ = stream.next();
                    if let Some(pe) = &arg_pseudo {
                        return Err(SelectorError::Syntax(format!(
                            "Got pseudo-element ::{} inside :not() at {}",
                            pseudo_element_name(pe),
                            next_.pos
                        )));
                    }
                    if !next_.matches(TokenType::Delim, ")") {
                        return Err(SelectorError::Syntax(format!("Expected ')', got {next_}")));
                    }
                    result = Tree::Negation {
                        selector: Box::new(result),
                        subselector: Box::new(argument),
                    };
                }
                "has" => {
                    let (combinator, arguments) = parse_relative_selector(stream)?;
                    result = Tree::Relation {
                        selector: Box::new(result),
                        combinator,
                        subselector: Box::new(arguments),
                    };
                }
                "matches" | "is" => {
                    let selectors = parse_simple_selector_arguments(stream)?;
                    result = Tree::Matching {
                        selector: Box::new(result),
                        selector_list: selectors,
                    };
                }
                "where" => {
                    let selectors = parse_simple_selector_arguments(stream)?;
                    result = Tree::SpecificityAdjustment {
                        selector: Box::new(result),
                        selector_list: selectors,
                    };
                }
                _ => {
                    let args = parse_arguments(stream)?;
                    result = Tree::Function {
                        selector: Box::new(result),
                        name: lowered,
                        arguments: args,
                    };
                }
            }
        } else {
            return Err(SelectorError::Syntax(format!(
                "Expected selector, got {peek}"
            )));
        }
    }

    if stream.used.len() == selector_start {
        return Err(SelectorError::Syntax(format!(
            "Expected selector, got {}",
            stream.peek()
        )));
    }
    Ok((result, pseudo_element))
}

/// Check the `:scope` placement rule against the consumed-token list.
///
/// `:scope` is allowed only at the very start of a selector or right after a
/// comma. The patterns match the grammar guard exactly.
fn scope_allowed(stream: &TokenStream) -> bool {
    let used = &stream.used;
    let n = used.len();
    if n == 2 {
        return true;
    }
    if n == 3 && used[0].ty == TokenType::S {
        return true;
    }
    if n >= 3 && used[n - 3].is_delim(&[","]) {
        return true;
    }
    if n >= 4 && used[n - 3].ty == TokenType::S && used[n - 4].is_delim(&[","]) {
        return true;
    }
    false
}

/// Collect tokens for a generic functional pseudo-class until the closing paren.
fn parse_arguments(stream: &mut TokenStream) -> Result<Vec<Token>, SelectorError> {
    let mut arguments = Vec::new();
    loop {
        stream.skip_whitespace();
        let next_ = stream.next();
        if matches!(
            next_.ty,
            TokenType::Ident | TokenType::String | TokenType::Number
        ) || next_.is_delim(&["+", "-"])
        {
            arguments.push(next_);
        } else if next_.matches(TokenType::Delim, ")") {
            return Ok(arguments);
        } else {
            return Err(SelectorError::Syntax(format!(
                "Expected an argument, got {next_}"
            )));
        }
    }
}

/// Parse the relative selector inside `:has()`.
///
/// The accumulated text is re-parsed through [`parse`] and the first selector is
/// returned. The combinator defaults to a space delimiter token.
fn parse_relative_selector(stream: &mut TokenStream) -> Result<(Token, Selector), SelectorError> {
    stream.skip_whitespace();
    let mut subselector = String::new();
    let mut next_ = stream.next();

    let combinator = if next_.is_delim(&["+", "-", ">", "~"]) {
        let c = next_.clone();
        stream.skip_whitespace();
        next_ = stream.next();
        c
    } else {
        Token::new(TokenType::Delim, " ", 0)
    };

    loop {
        if matches!(
            next_.ty,
            TokenType::Ident | TokenType::String | TokenType::Number
        ) || next_.is_delim(&[".", "*"])
        {
            subselector.push_str(next_.value_str());
        } else if next_.matches(TokenType::Delim, ")") {
            let result = parse(&subselector)?;
            let first = result
                .into_iter()
                .next()
                .expect("parse yields at least one selector or errors");
            return Ok((combinator, first));
        } else {
            return Err(SelectorError::Syntax(format!(
                "Expected an argument, got {next_}"
            )));
        }
        next_ = stream.next();
    }
}

/// Parse the selector list inside `:is()`, `:matches()`, or `:where()`.
fn parse_simple_selector_arguments(stream: &mut TokenStream) -> Result<Vec<Tree>, SelectorError> {
    let mut arguments = Vec::new();
    loop {
        let (result, pseudo_element) = parse_simple_selector(stream, true)?;
        if let Some(pe) = &pseudo_element {
            return Err(SelectorError::Syntax(format!(
                "Got pseudo-element ::{} inside function",
                pseudo_element_name(pe)
            )));
        }
        stream.skip_whitespace();
        let next_ = stream.next();
        if next_.matches(TokenType::Eof, "") || next_.matches(TokenType::Delim, ",") {
            stream.skip_whitespace();
            arguments.push(result);
        } else if next_.matches(TokenType::Delim, ")") {
            arguments.push(result);
            break;
        } else {
            return Err(SelectorError::Syntax(format!(
                "Expected an argument, got {next_}"
            )));
        }
    }
    Ok(arguments)
}

/// Parse the body of an attribute selector after the opening bracket.
fn parse_attrib(selector: Tree, stream: &mut TokenStream) -> Result<Tree, SelectorError> {
    stream.skip_whitespace();
    let mut attrib = stream.next_ident_or_star()?;
    if attrib.is_none() && !stream.peek().matches(TokenType::Delim, "|") {
        return Err(SelectorError::Syntax(format!(
            "Expected '|', got {}",
            stream.peek()
        )));
    }
    let mut namespace: Option<String> = None;
    let mut op: Option<String> = None;
    if stream.peek().matches(TokenType::Delim, "|") {
        stream.next();
        if stream.peek().matches(TokenType::Delim, "=") {
            namespace = None;
            stream.next();
            op = Some("|=".to_string());
        } else {
            namespace = attrib.take();
            attrib = Some(stream.next_ident()?);
            op = None;
        }
    }

    if op.is_none() {
        stream.skip_whitespace();
        let next_ = stream.next();
        if next_.matches(TokenType::Delim, "]") {
            return Ok(Tree::Attrib {
                selector: Box::new(selector),
                namespace,
                attrib: attrib.unwrap_or_default(),
                operator: "exists".to_string(),
                value: None,
            });
        }
        if next_.matches(TokenType::Delim, "=") {
            op = Some("=".to_string());
        } else if next_.is_delim(&["^", "$", "*", "~", "|", "!"])
            && stream.peek().matches(TokenType::Delim, "=")
        {
            op = Some(format!("{}=", next_.value_str()));
            stream.next();
        } else {
            return Err(SelectorError::Syntax(format!(
                "Operator expected, got {next_}"
            )));
        }
    }

    stream.skip_whitespace();
    let value = stream.next();
    if !matches!(value.ty, TokenType::Ident | TokenType::String) {
        return Err(SelectorError::Syntax(format!(
            "Expected string or ident, got {value}"
        )));
    }
    stream.skip_whitespace();
    let next_ = stream.next();
    if !next_.matches(TokenType::Delim, "]") {
        return Err(SelectorError::Syntax(format!("Expected ']', got {next_}")));
    }
    Ok(Tree::Attrib {
        selector: Box::new(selector),
        namespace,
        attrib: attrib.unwrap_or_default(),
        operator: op.expect("an operator was set before reaching the value"),
        value: Some(value),
    })
}
