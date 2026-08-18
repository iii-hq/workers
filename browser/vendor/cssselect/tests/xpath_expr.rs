//! Direct `XpathExpr` building, `argument_types`, and pseudo-element repr.
//!
//! Custom pseudo-element handlers are not part of this crate's public API, so
//! the cases here cover the building blocks such handlers rely on: `join`,
//! `add_condition` precedence, and the functional pseudo-element accessors.

use cssselect::{parse, xpath_literal, FunctionalPseudoElement, PseudoElement, XpathExpr};

#[test]
fn str_form_of_bare_condition() {
    let expr = XpathExpr::new("", "", "@href");
    assert_eq!(expr.to_xpath(), "[@href]");
}

#[test]
fn add_condition_precedence() {
    let mut expr = XpathExpr::new("", "*", "@id = 'first' or @id = 'second'");
    expr.add_condition("@href", "and");
    assert_eq!(
        expr.to_xpath(),
        "*[(@id = 'first' or @id = 'second') and (@href)]"
    );
}

#[test]
fn join_builds_path() {
    let mut left = XpathExpr::new("", "*", "");
    let other = XpathExpr::new("@href", "", "");
    left.join("/", &other);
    assert_eq!(left.to_xpath(), "*/@href");
}

#[test]
fn literal_quoting() {
    assert_eq!(xpath_literal("plain"), "'plain'");
    assert_eq!(xpath_literal("it's"), "\"it's\"");
    assert_eq!(xpath_literal("a'b\"c"), "concat('a',\"'\",'b\"c')");
}

#[test]
fn functional_pseudo_element_argument_types() {
    let cases: [(&str, &[&str]); 4] = [
        ("", &[]),
        ("ident", &["IDENT"]),
        ("\"string\"", &["STRING"]),
        ("1", &["NUMBER"]),
    ];
    for (arg, expected) in cases {
        let css = build_pe(arg);
        let selectors = parse(&css).unwrap();
        let pe = selectors[0].pseudo_element.as_ref().unwrap();
        let f = match pe {
            PseudoElement::Functional(f) => f,
            other => panic!("expected functional pseudo-element, got {other:?}"),
        };
        assert_eq!(f.argument_types(), expected.to_vec(), "for arg {arg:?}");
    }
}

/// Build `::pseudo_element(<arg>)`.
fn build_pe(arg: &str) -> String {
    let mut s = String::from("::pseudo_element(");
    s.push_str(arg);
    s.push(')');
    s
}

#[test]
fn functional_pseudo_element_canonical() {
    let f = FunctionalPseudoElement::new("ATTR", parse("::x(name)").unwrap()[0].pseudo_args());
    assert_eq!(f.name, "attr");
    assert_eq!(f.canonical(), "attr(name)");
}

/// Helper to expose a parsed functional pseudo-element's arguments.
trait PseudoArgs {
    fn pseudo_args(&self) -> Vec<cssselect::Token>;
}

impl PseudoArgs for cssselect::Selector {
    fn pseudo_args(&self) -> Vec<cssselect::Token> {
        match &self.pseudo_element {
            Some(PseudoElement::Functional(f)) => f.arguments.clone(),
            _ => Vec::new(),
        }
    }
}
