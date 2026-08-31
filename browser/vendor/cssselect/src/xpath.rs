//! XPath string builder and the translators.
//!
//! [`XpathExpr`] assembles an XPath 1.0 expression from a path, an element
//! test, and a condition. The translation engine walks a parsed [`Tree`] and
//! produces one expression per selector. [`GenericTranslator`] targets generic
//! XML. The HTML variant lives in [`crate::html`] and reuses this engine with a
//! different [`Config`].

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::SelectorError;
use crate::parser::{parse, parse_series, PseudoElement, Selector, Tree};
use crate::tokenizer::TokenType;

/// An XPath 1.0 expression under construction.
///
/// The string form is `path + element`, plus `[condition]` when a condition is
/// present. Conditions chain through [`XpathExpr::add_condition`], which wraps
/// each side in parentheses.
///
/// The parts are private so the type stays consistent. The condition never
/// carries its own brackets, and the path and element stay well formed. Build
/// and read the expression through the methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XpathExpr {
    /// The location path prefix.
    path: String,
    /// The element or node test.
    element: String,
    /// The predicate condition, without the surrounding brackets.
    condition: String,
}

impl XpathExpr {
    /// Build an expression from its parts.
    pub fn new(path: &str, element: &str, condition: &str) -> XpathExpr {
        XpathExpr {
            path: path.to_string(),
            element: element.to_string(),
            condition: condition.to_string(),
        }
    }

    /// Build an expression with only the element test set.
    fn element(element: &str) -> XpathExpr {
        XpathExpr::new("", element, "")
    }

    /// The string form: `path + element`, plus `[condition]` when present.
    pub fn to_xpath(&self) -> String {
        let mut out = format!("{}{}", self.path, self.element);
        if !self.condition.is_empty() {
            out.push('[');
            out.push_str(&self.condition);
            out.push(']');
        }
        out
    }

    /// Add a predicate condition, joined with `and` by default.
    ///
    /// When a condition already exists, both sides are wrapped in parentheses.
    pub fn add_condition(&mut self, condition: &str, conjunction: &str) -> &mut Self {
        if self.condition.is_empty() {
            self.condition = condition.to_string();
        } else {
            self.condition = format!("({}) {} ({})", self.condition, conjunction, condition);
        }
        self
    }

    /// Add an `and`-joined condition. Shorthand for the common case.
    fn and_condition(&mut self, condition: &str) -> &mut Self {
        self.add_condition(condition, "and")
    }

    /// Fold a non-universal element test into a `name()` condition.
    fn add_name_test(&mut self) {
        if self.element == "*" {
            return;
        }
        let mut parts = self.element.splitn(2, ':');
        let prefix = parts.next().unwrap_or("");
        let local = parts.next();
        let safe =
            is_safe_name(prefix) && local.is_none_or(|value| value == "*" || is_safe_name(value));
        let cond = if safe {
            format!("self::{}", self.element)
        } else {
            format!("name() = {}", xpath_literal(&self.element))
        };
        self.and_condition(&cond);
        self.element = "*".to_string();
    }

    /// Join this expression to another with a combiner string.
    ///
    /// Shorthand for [`XpathExpr::join_full`] with no closing combiner and no
    /// inner condition folding.
    pub fn join(&mut self, combiner: &str, other: &XpathExpr) -> &mut Self {
        self.join_full(combiner, other, None, false)
    }

    /// Join this expression to another with full control.
    ///
    /// When `has_inner_condition` is set, the other expression's condition is
    /// folded into the element test rather than kept separate. The optional
    /// closing combiner is appended to the element test.
    pub fn join_full(
        &mut self,
        combiner: &str,
        other: &XpathExpr,
        closing_combiner: Option<&str>,
        has_inner_condition: bool,
    ) -> &mut Self {
        let mut path = format!("{}{}", self.to_xpath(), combiner);
        if other.path != "*/" {
            path.push_str(&other.path);
        }
        self.path = path;
        if !has_inner_condition {
            self.element = match closing_combiner {
                Some(cc) => format!("{}{}", other.element, cc),
                None => other.element.clone(),
            };
            self.condition = other.condition.clone();
        } else {
            self.element = other.element.clone();
            if !other.condition.is_empty() {
                self.element = format!("{}[{}]", self.element, other.condition);
            }
            if let Some(cc) = closing_combiner {
                self.element.push_str(cc);
            }
        }
        self
    }
}

/// Render a value as an XPath 1.0 string literal.
///
/// XPath 1.0 has no string escapes, so a value with both quote kinds is built
/// with `concat(...)`.
pub fn xpath_literal(s: &str) -> String {
    if !s.contains('\'') {
        format!("'{s}'")
    } else if !s.contains('"') {
        format!("\"{s}\"")
    } else {
        let parts = split_at_single_quotes(s);
        let mut quoted: Vec<String> = Vec::new();
        for part in parts {
            if part.is_empty() {
                continue;
            }
            if part.contains('\'') {
                quoted.push(format!("\"{part}\""));
            } else {
                quoted.push(format!("'{part}'"));
            }
        }
        format!("concat({})", quoted.join(","))
    }
}

/// Split a string keeping runs of single quotes as separate parts.
///
/// This mirrors Python `re.split("('+)", s)`, where the capturing group keeps
/// the separator runs. Empty parts can appear and are dropped by the caller.
fn split_at_single_quotes(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '\'' {
            parts.push(core::mem::take(&mut current));
            let mut run = String::new();
            while let Some(&c2) = chars.peek() {
                if c2 == '\'' {
                    run.push('\'');
                    chars.next();
                } else {
                    break;
                }
            }
            parts.push(run);
        } else {
            current.push(c);
            chars.next();
        }
    }
    parts.push(current);
    parts
}

/// True when a name is safe to drop straight into an XPath name position.
fn is_safe_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// True when a value is non-empty and holds no whitespace.
fn is_non_whitespace(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{0c}'))
}

/// Case-folding and document-language settings for a translator.
///
/// Build one with [`Config::generic`] or [`Config::html`]. The fields are
/// crate-internal so a caller cannot mix settings into an invalid state, such
/// as the HTML pseudo-classes paired with the XML `lang` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The attribute name used by `:lang()`.
    pub(crate) lang_attribute: &'static str,
    /// Whether to fold element names to lower case.
    pub(crate) lower_case_element_names: bool,
    /// Whether to fold attribute names to lower case.
    pub(crate) lower_case_attribute_names: bool,
    /// Whether to fold attribute values to lower case.
    pub(crate) lower_case_attribute_values: bool,
    /// Whether to apply the HTML-specific pseudo-class implementations.
    pub(crate) html: bool,
}

impl Config {
    /// The generic XML configuration: fully case sensitive, `xml:lang`.
    pub fn generic() -> Config {
        Config {
            lang_attribute: "xml:lang",
            lower_case_element_names: false,
            lower_case_attribute_names: false,
            lower_case_attribute_values: false,
            html: false,
        }
    }

    /// The HTML configuration. With `xhtml`, names stay case sensitive.
    pub fn html(xhtml: bool) -> Config {
        Config {
            lang_attribute: "lang",
            lower_case_element_names: !xhtml,
            lower_case_attribute_names: !xhtml,
            lower_case_attribute_values: false,
            html: true,
        }
    }
}

/// Translate a parsed group of selectors to an XPath string.
///
/// Per-selector results are joined with `" | "`. Pseudo-elements are translated
/// here, which raises an expression error in the built-in translators.
pub(crate) fn css_to_xpath(cfg: &Config, css: &str, prefix: &str) -> Result<String, SelectorError> {
    let selectors = parse(css)?;
    let mut parts: Vec<String> = Vec::with_capacity(selectors.len());
    for selector in &selectors {
        parts.push(selector_to_xpath(
            cfg,
            selector,
            prefix,
            PseudoElements::Translate,
        )?);
    }
    Ok(parts.join(" | "))
}

/// How a translator handles a selector's pseudo-element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElements {
    /// Translate the pseudo-element. The built-in translators reject it with an
    /// expression error, since XPath has no equivalent.
    Translate,
    /// Ignore the pseudo-element and translate the rest of the selector.
    Ignore,
}

/// Translate a single parsed selector to an XPath string.
///
/// With [`PseudoElements::Ignore`], the selector's pseudo-element is dropped.
pub(crate) fn selector_to_xpath(
    cfg: &Config,
    selector: &Selector,
    prefix: &str,
    pseudo_elements: PseudoElements,
) -> Result<String, SelectorError> {
    let mut xpath = xpath_tree(cfg, &selector.parsed_tree)?;
    if pseudo_elements == PseudoElements::Translate {
        if let Some(pe) = &selector.pseudo_element {
            xpath = xpath_pseudo_element(xpath, pe)?;
        }
    }
    Ok(format!("{}{}", prefix, xpath.to_xpath()))
}

/// Scrapling's two Parsel-compatible pseudo-elements.
fn xpath_pseudo_element(
    xpath: XpathExpr,
    pseudo_element: &PseudoElement,
) -> Result<XpathExpr, SelectorError> {
    let mut path = xpath.to_xpath();
    match pseudo_element {
        PseudoElement::Ident(name) if name == "text" => {
            if path == "*" {
                path = "text()".to_string();
            } else if path.ends_with("::*/*") {
                path.truncate(path.len() - 3);
                path.push_str("text()");
            } else {
                path.push_str("/text()");
            }
        }
        PseudoElement::Functional(function)
            if function.name == "attr"
                && function.arguments.len() == 1
                && matches!(
                    function.arguments[0].ty,
                    TokenType::String | TokenType::Ident
                ) =>
        {
            if path.ends_with("::*/*") {
                path.truncate(path.len() - 2);
            }
            path.push_str("/@");
            path.push_str(function.arguments[0].value_str());
        }
        PseudoElement::Functional(function) if function.name == "attr" => {
            return Err(SelectorError::Expression(format!(
                "Expected a single string or ident for ::attr(), got {:?}",
                function.arguments
            )));
        }
        PseudoElement::Functional(function) => {
            return Err(SelectorError::Expression(format!(
                "The functional pseudo-element ::{}() is unknown",
                function.name
            )));
        }
        PseudoElement::Ident(name) => {
            return Err(SelectorError::Expression(format!(
                "The pseudo-element ::{name} is unknown"
            )));
        }
    }
    Ok(XpathExpr::new("", &path, ""))
}

/// Dispatch a parsed tree node to its translator.
fn xpath_tree(cfg: &Config, tree: &Tree) -> Result<XpathExpr, SelectorError> {
    match tree {
        Tree::Element { namespace, element } => Ok(xpath_element(cfg, namespace, element)),
        Tree::Hash { selector, id } => {
            let mut xpath = xpath_tree(cfg, selector)?;
            xpath.and_condition(&format!("@id = {}", xpath_literal(id)));
            Ok(xpath)
        }
        Tree::Class {
            selector,
            class_name,
        } => {
            let mut xpath = xpath_tree(cfg, selector)?;
            attrib_includes(&mut xpath, "@class", Some(class_name));
            Ok(xpath)
        }
        Tree::Attrib { .. } => xpath_attrib(cfg, tree),
        Tree::Pseudo { selector, ident } => {
            let xpath = xpath_tree(cfg, selector)?;
            xpath_pseudo(cfg, xpath, ident)
        }
        Tree::Function {
            selector,
            name,
            arguments,
        } => {
            let xpath = xpath_tree(cfg, selector)?;
            xpath_function(cfg, xpath, name, arguments)
        }
        Tree::Negation {
            selector,
            subselector,
        } => {
            let mut xpath = xpath_tree(cfg, selector)?;
            let mut sub = xpath_tree(cfg, subselector)?;
            sub.add_name_test();
            if !sub.condition.is_empty() {
                xpath.and_condition(&format!("not({})", sub.condition));
            } else {
                xpath.and_condition("0");
            }
            Ok(xpath)
        }
        Tree::Relation {
            selector,
            combinator,
            subselector,
        } => {
            let xpath = xpath_tree(cfg, selector)?;
            let right = xpath_tree(cfg, &subselector.parsed_tree)?;
            xpath_relation(xpath, combinator.value_str(), right)
        }
        Tree::Matching {
            selector,
            selector_list,
        }
        | Tree::SpecificityAdjustment {
            selector,
            selector_list,
        } => {
            let mut xpath = xpath_tree(cfg, selector)?;
            let mut alternatives = Vec::new();
            for sel in selector_list {
                let mut e = xpath_tree(cfg, sel)?;
                e.add_name_test();
                alternatives.push(if e.condition.is_empty() {
                    "1".to_string()
                } else {
                    e.condition
                });
            }
            if alternatives.is_empty() {
                xpath.and_condition("0");
            } else if alternatives.len() == 1 {
                xpath.and_condition(&alternatives[0]);
            } else {
                xpath.and_condition(&format!("({})", alternatives.join(") or (")));
            }
            Ok(xpath)
        }
        Tree::Combined {
            selector,
            combinator,
            subselector,
        } => {
            let left = xpath_tree(cfg, selector)?;
            let right = xpath_tree(cfg, subselector)?;
            xpath_combinator(combinator, left, right)
        }
    }
}

/// Translate a type or universal selector.
fn xpath_element(cfg: &Config, namespace: &Option<String>, element: &Option<String>) -> XpathExpr {
    let mut safe;
    let mut name;
    match element {
        None => {
            name = "*".to_string();
            safe = true;
        }
        Some(el) => {
            name = el.clone();
            safe = is_safe_name(&name);
            if cfg.lower_case_element_names {
                name = name.to_ascii_lowercase();
            }
        }
    }
    if let Some(ns) = namespace {
        name = format!("{ns}:{name}");
        safe = safe && is_safe_name(ns);
    }
    let mut xpath = XpathExpr::element(&name);
    if !safe {
        xpath.add_name_test();
    }
    xpath
}

/// Translate an attribute selector.
fn xpath_attrib(cfg: &Config, tree: &Tree) -> Result<XpathExpr, SelectorError> {
    let (selector, namespace, attrib, operator, value) = match tree {
        Tree::Attrib {
            selector,
            namespace,
            attrib,
            operator,
            value,
        } => (selector, namespace, attrib, operator, value),
        _ => unreachable!("xpath_attrib called on a non-Attrib tree"),
    };

    let mut name = if cfg.lower_case_attribute_names {
        attrib.to_ascii_lowercase()
    } else {
        attrib.clone()
    };
    let mut safe = is_safe_name(&name);
    if let Some(ns) = namespace {
        name = format!("{ns}:{name}");
        safe = safe && is_safe_name(ns);
    }
    let attrib_xpath = if safe {
        format!("@{name}")
    } else {
        format!("attribute::*[name() = {}]", xpath_literal(&name))
    };

    let value_str: Option<String> = match value {
        None => None,
        Some(token) => {
            if cfg.lower_case_attribute_values {
                Some(token.value_str().to_ascii_lowercase())
            } else {
                Some(token.value_str().to_string())
            }
        }
    };

    let mut xpath = xpath_tree(cfg, selector)?;
    apply_attrib_operator(&mut xpath, operator, &attrib_xpath, value_str.as_deref())?;
    Ok(xpath)
}

/// Apply the named attribute operator to the expression.
fn apply_attrib_operator(
    xpath: &mut XpathExpr,
    operator: &str,
    name: &str,
    value: Option<&str>,
) -> Result<(), SelectorError> {
    match operator {
        "exists" => {
            xpath.and_condition(name);
        }
        "=" => {
            xpath.and_condition(&format!(
                "{} = {}",
                name,
                xpath_literal(value.unwrap_or(""))
            ));
        }
        "!=" => {
            let v = value.unwrap_or("");
            if !v.is_empty() {
                xpath.and_condition(&format!("not({name}) or {name} != {}", xpath_literal(v)));
            } else {
                xpath.and_condition(&format!("{name} != {}", xpath_literal(v)));
            }
        }
        "~=" => attrib_includes(xpath, name, value),
        "|=" => {
            let v = value.unwrap_or("");
            let arg = xpath_literal(v);
            let arg_dash = xpath_literal(&format!("{v}-"));
            xpath.and_condition(&format!(
                "{name} and ({name} = {arg} or starts-with({name}, {arg_dash}))"
            ));
        }
        "^=" => {
            let v = value.unwrap_or("");
            if !v.is_empty() {
                xpath.and_condition(&format!(
                    "{name} and starts-with({name}, {})",
                    xpath_literal(v)
                ));
            } else {
                xpath.and_condition("0");
            }
        }
        "$=" => {
            let v = value.unwrap_or("");
            if !v.is_empty() {
                let len = v.chars().count() as i64 - 1;
                xpath.and_condition(&format!(
                    "{name} and substring({name}, string-length({name})-{len}) = {}",
                    xpath_literal(v)
                ));
            } else {
                xpath.and_condition("0");
            }
        }
        "*=" => {
            let v = value.unwrap_or("");
            if !v.is_empty() {
                xpath.and_condition(&format!(
                    "{name} and contains({name}, {})",
                    xpath_literal(v)
                ));
            } else {
                xpath.and_condition("0");
            }
        }
        other => {
            return Err(SelectorError::Expression(format!(
                "Unknown attribute operator: {other}"
            )));
        }
    }
    Ok(())
}

/// The `~=` includes operator, shared by class selectors and `[a~=b]`.
fn attrib_includes(xpath: &mut XpathExpr, name: &str, value: Option<&str>) {
    let v = value.unwrap_or("");
    if !v.is_empty() && is_non_whitespace(v) {
        let arg = xpath_literal(&format!(" {v} "));
        xpath.and_condition(&format!(
            "{name} and contains(concat(' ', normalize-space({name}), ' '), {arg})"
        ));
    } else {
        xpath.and_condition("0");
    }
}

/// Translate a combinator between a left and right expression.
fn xpath_combinator(
    combinator: &str,
    mut left: XpathExpr,
    right: XpathExpr,
) -> Result<XpathExpr, SelectorError> {
    match combinator {
        " " => {
            left.join_full("/descendant-or-self::*/", &right, None, false);
        }
        ">" => {
            left.join_full("/", &right, None, false);
        }
        "+" => {
            left.join_full("/following-sibling::", &right, None, false);
            left.add_name_test();
            left.and_condition("position() = 1");
        }
        "~" => {
            left.join_full("/following-sibling::", &right, None, false);
        }
        other => {
            return Err(SelectorError::Expression(format!(
                "Unknown combinator: {other}"
            )));
        }
    }
    Ok(left)
}

/// Translate a `:has()` relation by its combinator.
fn xpath_relation(
    mut left: XpathExpr,
    combinator: &str,
    mut right: XpathExpr,
) -> Result<XpathExpr, SelectorError> {
    match combinator {
        " " => {
            left.join_full("[descendant::", &right, Some("]"), true);
        }
        ">" => {
            left.join_full("[./", &right, Some("]"), true);
        }
        "+" => {
            right.add_name_test();
            right.and_condition("position() = 1");
            left.and_condition(&format!(
                "following-sibling::{}[{}]",
                right.element, right.condition
            ));
        }
        "~" => {
            left.join_full("[following-sibling::", &right, Some("]"), true);
        }
        other => {
            return Err(SelectorError::Expression(format!(
                "Unknown combinator: {other}"
            )));
        }
    }
    Ok(left)
}

/// Translate a functional pseudo-class.
fn xpath_function(
    cfg: &Config,
    xpath: XpathExpr,
    name: &str,
    arguments: &[crate::tokenizer::Token],
) -> Result<XpathExpr, SelectorError> {
    match name {
        "nth-child" => nth_child(xpath, arguments, false, true),
        "nth-last-child" => nth_child(xpath, arguments, true, true),
        "nth-of-type" => {
            if xpath.element == "*" {
                return Err(SelectorError::Expression(
                    "*:nth-of-type() is not implemented".to_string(),
                ));
            }
            nth_child(xpath, arguments, false, false)
        }
        "nth-last-of-type" => {
            if xpath.element == "*" {
                return Err(SelectorError::Expression(
                    "*:nth-of-type() is not implemented".to_string(),
                ));
            }
            nth_child(xpath, arguments, true, false)
        }
        "contains" => contains_function(xpath, arguments),
        "lang" => lang_function(cfg, xpath, arguments),
        other => Err(SelectorError::Expression(format!(
            "The pseudo-class :{other}() is unknown"
        ))),
    }
}

/// The `:nth-*` core algorithm.
fn nth_child(
    mut xpath: XpathExpr,
    arguments: &[crate::tokenizer::Token],
    last: bool,
    add_name_test: bool,
) -> Result<XpathExpr, SelectorError> {
    let invalid_series =
        || SelectorError::Expression(format!("Invalid series: '{}'", repr_token_list(arguments)));
    let (a, b) = match parse_series(arguments) {
        Ok(pair) => pair,
        Err(_) => return Err(invalid_series()),
    };
    let b_min_1 = match b.checked_sub(1) {
        Some(value) => value,
        None if a == 1 => return Ok(xpath),
        None if a <= 0 => {
            xpath.and_condition("0");
            return Ok(xpath);
        }
        None => return Err(invalid_series()),
    };
    if a == 1 && b_min_1 <= 0 {
        return Ok(xpath);
    }
    if a < 0 && b_min_1 < 0 {
        xpath.and_condition("0");
        return Ok(xpath);
    }
    let nodetest = if add_name_test {
        "*".to_string()
    } else {
        xpath.element.clone()
    };
    let siblings_count = if !last {
        format!("count(preceding-sibling::{nodetest})")
    } else {
        format!("count(following-sibling::{nodetest})")
    };
    if a == 0 {
        xpath.and_condition(&format!("{siblings_count} = {b_min_1}"));
        return Ok(xpath);
    }
    let mut expressions: Vec<String> = Vec::new();
    if a > 0 {
        if b_min_1 > 0 {
            expressions.push(format!("{siblings_count} >= {b_min_1}"));
        }
    } else {
        expressions.push(format!("{siblings_count} <= {b_min_1}"));
    }
    let a_abs = match a.checked_abs() {
        Some(value) => value,
        None if b_min_1 < 0 => {
            xpath.and_condition("0");
            return Ok(xpath);
        }
        None => {
            xpath.and_condition(&format!("{siblings_count} = {b_min_1}"));
            return Ok(xpath);
        }
    };
    if a_abs != 1 {
        let mut left = siblings_count.clone();
        let b_neg = (-(b_min_1 as i128)).rem_euclid(a_abs as i128);
        if b_neg != 0 {
            left = format!("({left} +{b_neg})");
        }
        expressions.push(format!("{left} mod {a} = 0"));
    }
    let condition = if expressions.len() > 1 {
        expressions
            .iter()
            .map(|e| format!("({e})"))
            .collect::<Vec<_>>()
            .join(" and ")
    } else {
        expressions.join(" and ")
    };
    xpath.and_condition(&condition);
    Ok(xpath)
}

/// Translate `:contains()`.
fn contains_function(
    mut xpath: XpathExpr,
    arguments: &[crate::tokenizer::Token],
) -> Result<XpathExpr, SelectorError> {
    let types = arg_types(arguments);
    if types != ["STRING"] && types != ["IDENT"] {
        return Err(SelectorError::Expression(format!(
            "Expected a single string or ident for :contains(), got {}",
            repr_token_list(arguments)
        )));
    }
    let value = arguments[0].value_str();
    xpath.and_condition(&format!("contains(., {})", xpath_literal(value)));
    Ok(xpath)
}

/// Translate `:lang()` for the generic translator.
fn lang_function(
    cfg: &Config,
    mut xpath: XpathExpr,
    arguments: &[crate::tokenizer::Token],
) -> Result<XpathExpr, SelectorError> {
    let types = arg_types(arguments);
    if types != ["STRING"] && types != ["IDENT"] {
        return Err(SelectorError::Expression(format!(
            "Expected a single string or ident for :lang(), got {}",
            repr_token_list(arguments)
        )));
    }
    let value = arguments[0].value_str();
    if cfg.html {
        let arg = xpath_literal(&format!("{}-", value.to_ascii_lowercase()));
        xpath.and_condition(&format!(
            "ancestor-or-self::*[@lang][1][starts-with(concat(translate(@{}, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz'), '-'), {})]",
            cfg.lang_attribute, arg
        ));
    } else {
        xpath.and_condition(&format!("lang({})", xpath_literal(value)));
    }
    Ok(xpath)
}

/// The argument-type names for a token slice.
fn arg_types(arguments: &[crate::tokenizer::Token]) -> Vec<&'static str> {
    arguments.iter().map(|t| t.ty.name()).collect()
}

/// Render a token list the way a list of token reprs prints: `[<..>, <..>]`.
///
/// The expression errors echo the offending argument tokens, so the text has to
/// match the repr of the argument list character for character.
fn repr_token_list(arguments: &[crate::tokenizer::Token]) -> String {
    let mut out = String::from("[");
    for (i, t) in arguments.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{t}"));
    }
    out.push(']');
    out
}

/// Translate a simple pseudo-class.
fn xpath_pseudo(
    cfg: &Config,
    mut xpath: XpathExpr,
    ident: &str,
) -> Result<XpathExpr, SelectorError> {
    match ident {
        "root" => {
            xpath.and_condition("not(parent::*)");
        }
        "scope" => {
            xpath.and_condition("1");
        }
        "first-child" => {
            xpath.and_condition("count(preceding-sibling::*) = 0");
        }
        "last-child" => {
            xpath.and_condition("count(following-sibling::*) = 0");
        }
        "first-of-type" => {
            if xpath.element == "*" {
                return Err(SelectorError::Expression(
                    "*:first-of-type is not implemented".to_string(),
                ));
            }
            let cond = format!("count(preceding-sibling::{}) = 0", xpath.element);
            xpath.and_condition(&cond);
        }
        "last-of-type" => {
            if xpath.element == "*" {
                return Err(SelectorError::Expression(
                    "*:last-of-type is not implemented".to_string(),
                ));
            }
            let cond = format!("count(following-sibling::{}) = 0", xpath.element);
            xpath.and_condition(&cond);
        }
        "only-child" => {
            xpath.and_condition("count(parent::*/child::*) = 1");
        }
        "only-of-type" => {
            if xpath.element == "*" {
                return Err(SelectorError::Expression(
                    "*:only-of-type is not implemented".to_string(),
                ));
            }
            let cond = format!("count(parent::*/child::{}) = 1", xpath.element);
            xpath.and_condition(&cond);
        }
        "empty" => {
            xpath.and_condition("not(*) and not(string-length())");
        }
        "link" => {
            if cfg.html {
                xpath.and_condition(
                    "@href and (name(.) = 'a' or name(.) = 'link' or name(.) = 'area')",
                );
            } else {
                xpath.and_condition("0");
            }
        }
        "checked" => {
            if cfg.html {
                xpath.and_condition(
                    "(@selected and name(.) = 'option') or (@checked and (name(.) = 'input' or name(.) = 'command')and (@type = 'checkbox' or @type = 'radio'))",
                );
            } else {
                xpath.and_condition("0");
            }
        }
        "disabled" => {
            if cfg.html {
                xpath.and_condition(HTML_DISABLED);
            } else {
                xpath.and_condition("0");
            }
        }
        "enabled" => {
            if cfg.html {
                xpath.and_condition(HTML_ENABLED);
            } else {
                xpath.and_condition("0");
            }
        }
        "visited" | "hover" | "active" | "focus" | "target" => {
            xpath.and_condition("0");
        }
        other => {
            return Err(SelectorError::Expression(format!(
                "The pseudo-class :{other} is unknown"
            )));
        }
    }
    Ok(xpath)
}

/// The `:disabled` condition for HTML, byte-for-byte with the document order.
const HTML_DISABLED: &str = "
        (
            @disabled and
            (
                (name(.) = 'input' and @type != 'hidden') or
                name(.) = 'button' or
                name(.) = 'select' or
                name(.) = 'textarea' or
                name(.) = 'command' or
                name(.) = 'fieldset' or
                name(.) = 'optgroup' or
                name(.) = 'option'
            )
        ) or (
            (
                (name(.) = 'input' and @type != 'hidden') or
                name(.) = 'button' or
                name(.) = 'select' or
                name(.) = 'textarea'
            )
            and ancestor::fieldset[@disabled]
        )
        ";

/// The `:enabled` condition for HTML, byte-for-byte with the document order.
const HTML_ENABLED: &str = "
        (
            @href and (
                name(.) = 'a' or
                name(.) = 'link' or
                name(.) = 'area'
            )
        ) or (
            (
                name(.) = 'command' or
                name(.) = 'fieldset' or
                name(.) = 'optgroup'
            )
            and not(@disabled)
        ) or (
            (
                (name(.) = 'input' and @type != 'hidden') or
                name(.) = 'button' or
                name(.) = 'select' or
                name(.) = 'textarea' or
                name(.) = 'keygen'
            )
            and not (@disabled or ancestor::fieldset[@disabled])
        ) or (
            name(.) = 'option' and not(
                @disabled or ancestor::optgroup[@disabled]
            )
        )
        ";

/// Translator that targets generic XML. Fully case sensitive.
#[derive(Debug, Clone)]
pub struct GenericTranslator {
    config: Config,
}

impl Default for GenericTranslator {
    fn default() -> Self {
        GenericTranslator::new()
    }
}

impl GenericTranslator {
    /// Create a translator with default settings.
    pub fn new() -> Self {
        GenericTranslator {
            config: Config::generic(),
        }
    }

    /// Translate a CSS group of selectors to an XPath string.
    ///
    /// The default prefix scopes selectors to the context node's subtree. Per
    /// selector results join with `" | "`.
    pub fn css_to_xpath(&self, css: &str) -> Result<String, SelectorError> {
        css_to_xpath(&self.config, css, "descendant-or-self::")
    }

    /// Translate a CSS group of selectors using an explicit prefix.
    ///
    /// An empty prefix yields an XPath with no leading axis.
    pub fn css_to_xpath_with_prefix(
        &self,
        css: &str,
        prefix: &str,
    ) -> Result<String, SelectorError> {
        css_to_xpath(&self.config, css, prefix)
    }

    /// Translate a single parsed selector to an XPath string.
    ///
    /// The pseudo-element is ignored. The default prefix scopes the selector to
    /// the context node's subtree.
    pub fn selector_to_xpath(&self, selector: &Selector) -> Result<String, SelectorError> {
        selector_to_xpath(
            &self.config,
            selector,
            "descendant-or-self::",
            PseudoElements::Ignore,
        )
    }

    /// Translate a single parsed selector with an explicit prefix and
    /// pseudo-element handling.
    pub fn selector_to_xpath_with(
        &self,
        selector: &Selector,
        prefix: &str,
        pseudo_elements: PseudoElements,
    ) -> Result<String, SelectorError> {
        selector_to_xpath(&self.config, selector, prefix, pseudo_elements)
    }
}
