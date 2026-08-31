//! Direct `parse_series` results on `:nth-child` arguments.

use cssselect::{parse, parse_series, Token, TokenType, Tree};

/// Parse `:nth-child(<css>)` and run `parse_series` on its arguments.
fn series(css: &str) -> Option<(i64, i64)> {
    let input = alloc_nth(css);
    let selectors = parse(&input).unwrap();
    let arguments = match &selectors[0].parsed_tree {
        Tree::Function { arguments, .. } => arguments.clone(),
        other => panic!("expected a function tree, got {other:?}"),
    };
    parse_series(&arguments).ok()
}

/// Build `:nth-child(<css>)`.
fn alloc_nth(css: &str) -> String {
    let mut s = String::from(":nth-child(");
    s.push_str(css);
    s.push(')');
    s
}

#[test]
fn positive_b() {
    assert_eq!(series("1n+3"), Some((1, 3)));
    assert_eq!(series("1n +3"), Some((1, 3)));
    assert_eq!(series("1n + 3"), Some((1, 3)));
    assert_eq!(series("1n+ 3"), Some((1, 3)));
}

#[test]
fn negative_b() {
    assert_eq!(series("1n-3"), Some((1, -3)));
    assert_eq!(series("1n -3"), Some((1, -3)));
    assert_eq!(series("1n - 3"), Some((1, -3)));
    assert_eq!(series("1n- 3"), Some((1, -3)));
    assert_eq!(series("n-5"), Some((1, -5)));
}

#[test]
fn keywords_and_coefficients() {
    assert_eq!(series("odd"), Some((2, 1)));
    assert_eq!(series("even"), Some((2, 0)));
    assert_eq!(series("3n"), Some((3, 0)));
    assert_eq!(series("n"), Some((1, 0)));
    assert_eq!(series("+n"), Some((1, 0)));
    assert_eq!(series("-n"), Some((-1, 0)));
    assert_eq!(series("5"), Some((0, 5)));
}

#[test]
fn signed_coefficients() {
    assert_eq!(series("-n-2"), Some((-1, -2)));
    assert_eq!(series("-2n+4"), Some((-2, 4)));
    assert_eq!(series("10n-10"), Some((10, -10)));
    assert_eq!(series("2n-1"), Some((2, -1)));
    assert_eq!(series("0"), Some((0, 0)));
    assert_eq!(series("-0"), Some((0, 0)));
    assert_eq!(series("+5"), Some((0, 5)));
    assert_eq!(series("-5"), Some((0, -5)));
    assert_eq!(series("n+0"), Some((1, 0)));
}

#[test]
fn invalid() {
    assert_eq!(series("foo"), None);
    assert_eq!(series("n+"), None);
    assert_eq!(series("n-"), None);
    assert_eq!(series("-n+"), None);
}

#[test]
fn string_token_is_rejected() {
    // A string token anywhere in a series is an error.
    let tokens = vec![Token::new(TokenType::String, "2", 0)];
    assert!(parse_series(&tokens).is_err());
}
