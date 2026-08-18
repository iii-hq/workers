//! Specificity triples for a range of selectors.

use cssselect::{parse, Specificity};

/// The specificity of a single-selector input.
fn specificity(css: &str) -> Specificity {
    let selectors = parse(css).unwrap();
    assert_eq!(selectors.len(), 1, "expected one selector for {css:?}");
    selectors[0].specificity()
}

#[test]
fn simple_selectors() {
    assert_eq!(specificity("*"), (0, 0, 0));
    assert_eq!(specificity(" foo"), (0, 0, 1));
    assert_eq!(specificity(":empty "), (0, 1, 0));
    assert_eq!(specificity(":before"), (0, 0, 1));
    assert_eq!(specificity("*:before"), (0, 0, 1));
    assert_eq!(specificity(":nth-child(2)"), (0, 1, 0));
    assert_eq!(specificity(".bar"), (0, 1, 0));
    assert_eq!(specificity("[baz]"), (0, 1, 0));
    assert_eq!(specificity("[baz=\"4\"]"), (0, 1, 0));
    assert_eq!(specificity("[baz^=\"4\"]"), (0, 1, 0));
    assert_eq!(specificity("#lipsum"), (1, 0, 0));
    assert_eq!(specificity("::attr(name)"), (0, 0, 1));
}

#[test]
fn negation_passes_through() {
    assert_eq!(specificity(":not(*)"), (0, 0, 0));
    assert_eq!(specificity(":not(foo)"), (0, 0, 1));
    assert_eq!(specificity(":not(.foo)"), (0, 1, 0));
    assert_eq!(specificity(":not([foo])"), (0, 1, 0));
    assert_eq!(specificity(":not(:empty)"), (0, 1, 0));
    assert_eq!(specificity(":not(#foo)"), (1, 0, 0));
}

#[test]
fn has_relation() {
    assert_eq!(specificity(":has(*)"), (0, 0, 0));
    assert_eq!(specificity(":has(foo)"), (0, 0, 1));
    assert_eq!(specificity(":has(.foo)"), (0, 1, 0));
    assert_eq!(specificity(":has(> foo)"), (0, 0, 1));
}

#[test]
fn matching_and_where() {
    assert_eq!(specificity(":is(.foo, #bar)"), (1, 0, 0));
    assert_eq!(specificity(":is(:hover, :visited)"), (0, 1, 0));
    assert_eq!(specificity(":where(:hover, :visited)"), (0, 0, 0));
    assert_eq!(specificity("div:is(.x)"), (0, 1, 1));
    assert_eq!(specificity("div:where(.x)"), (0, 0, 1));
}

#[test]
fn pseudo_element_bump() {
    assert_eq!(specificity("foo:empty"), (0, 1, 1));
    assert_eq!(specificity("foo:before"), (0, 0, 2));
    assert_eq!(specificity("foo::before"), (0, 0, 2));
    assert_eq!(specificity("foo:empty::before"), (0, 1, 2));
}

#[test]
fn combined() {
    assert_eq!(
        specificity("#lorem + foo#ipsum:first-child > bar:first-line"),
        (2, 1, 3)
    );
}
