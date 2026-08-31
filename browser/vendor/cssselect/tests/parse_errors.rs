//! Exact `SelectorSyntaxError` message strings.

use cssselect::{parse, SelectorError};

/// The error message for `css`, or `None` when it parses.
fn get_error(css: &str) -> Option<String> {
    match parse(css) {
        Err(SelectorError::Syntax(msg)) => Some(msg),
        Err(other) => panic!("expected syntax error for {css:?}, got {other:?}"),
        Ok(_) => None,
    }
}

/// Assert the error for `css` equals `expected`.
fn assert_error(css: &str, expected: &str) {
    assert_eq!(get_error(css).as_deref(), Some(expected), "for {css:?}");
}

#[test]
fn structural_errors() {
    assert_error(
        "attributes(href)/html/body/a",
        "Expected selector, got <DELIM '(' at 10>",
    );
    assert_error(
        "attributes(href)",
        "Expected selector, got <DELIM '(' at 10>",
    );
    assert_error("html/body/a", "Expected selector, got <DELIM '/' at 4>");
    assert_error(" ", "Expected selector, got <EOF at 1>");
    assert_error("div, ", "Expected selector, got <EOF at 5>");
    assert_error(" , div", "Expected selector, got <DELIM ',' at 1>");
    assert_error("p, , div", "Expected selector, got <DELIM ',' at 3>");
    assert_error("div > ", "Expected selector, got <EOF at 6>");
    assert_error("  > div", "Expected selector, got <DELIM '>' at 2>");
}

#[test]
fn name_and_attribute_errors() {
    assert_error("foo|#bar", "Expected ident or '*', got <HASH 'bar' at 4>");
    assert_error("#.foo", "Expected selector, got <DELIM '#' at 0>");
    assert_error(".#foo", "Expected ident, got <HASH 'foo' at 1>");
    assert_error(":#foo", "Expected ident, got <HASH 'foo' at 1>");
    assert_error("[*]", "Expected '|', got <DELIM ']' at 2>");
    assert_error("[foo|]", "Expected ident, got <DELIM ']' at 5>");
    assert_error("[#]", "Expected ident or '*', got <DELIM '#' at 1>");
    assert_error("[foo=#]", "Expected string or ident, got <DELIM '#' at 5>");
    assert_error("[href]a", "Expected selector, got <IDENT 'a' at 6>");
    assert_eq!(get_error("[rel=stylesheet]"), None);
    assert_error(
        "[rel:stylesheet]",
        "Operator expected, got <DELIM ':' at 4>",
    );
    assert_error("[rel=stylesheet", "Expected ']', got <EOF at 15>");
}

#[test]
fn function_and_string_errors() {
    assert_eq!(get_error(":lang(fr)"), None);
    assert_error(":lang(fr", "Expected an argument, got <EOF at 8>");
    assert_error(":contains(\"foo", "Unclosed string at 10");
    assert_error("foo!", "Expected selector, got <DELIM '!' at 3>");
}

#[test]
fn misplaced_pseudo_elements() {
    assert_error(
        "a:before:empty",
        "Got pseudo-element ::before not at the end of a selector",
    );
    assert_error(
        "li:before a",
        "Got pseudo-element ::before not at the end of a selector",
    );
    assert_error(
        ":not(:before)",
        "Got pseudo-element ::before inside :not() at 12",
    );
    assert_error(":not(:not(a))", "Got nested :not()");
    assert_error(
        ":is(:before)",
        "Got pseudo-element ::before inside function",
    );
    assert_error(":is(a b)", "Expected an argument, got <IDENT 'b' at 6>");
    assert_error(
        ":where(:before)",
        "Got pseudo-element ::before inside function",
    );
    assert_error(":where(a b)", "Expected an argument, got <IDENT 'b' at 9>");
}

#[test]
fn scope_placement() {
    assert_error(
        ":scope > div :scope header",
        "Got immediate child pseudo-element \":scope\" not at the start of a selector",
    );
    assert_error(
        "div :scope header",
        "Got immediate child pseudo-element \":scope\" not at the start of a selector",
    );
    assert_error("> div p", "Expected selector, got <DELIM '>' at 0>");
}

#[test]
fn has_arity() {
    assert_error(":has(a, b)", "Expected an argument, got <DELIM ',' at 6>");
    assert_error(":has()", "Expected selector, got <EOF at 0>");
}

#[test]
fn is_where_empty_and_comma_edges() {
    assert_error(":is()", "Expected selector, got <DELIM ')' at 4>");
    assert_error(":where()", "Expected selector, got <DELIM ')' at 7>");
    assert_error(":is(a,)", "Expected selector, got <DELIM ')' at 6>");
    assert_error(":is(,a)", "Expected selector, got <DELIM ',' at 4>");
}
