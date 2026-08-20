//! Tokenizer output and escape handling.

use cssselect::{tokenize, SelectorError};

/// Render a token list as their `Display` (Python `repr`) strings.
fn token_strs(css: &str) -> Vec<String> {
    tokenize(css)
        .unwrap()
        .iter()
        .map(|t| t.to_string())
        .collect()
}

#[test]
fn rich_case() {
    // The byte between `f` and `[` is a no-break space U+00A0, not a regular
    // space. CSS does not treat it as whitespace, so it folds into the ident.
    let input = "E\\ é > f\u{a0}[a~=\"y\\\"x\"]:nth(/* fu /]* */-3.7)";
    let tokens = token_strs(input);
    assert_eq!(
        tokens,
        vec![
            "<IDENT 'E é' at 0>",
            "<S ' ' at 4>",
            "<DELIM '>' at 5>",
            "<S ' ' at 6>",
            // the no-break space is not whitespace in CSS
            "<IDENT 'f\u{a0}' at 7>",
            "<DELIM '[' at 9>",
            "<IDENT 'a' at 10>",
            "<DELIM '~' at 11>",
            "<DELIM '=' at 12>",
            "<STRING 'y\"x' at 13>",
            "<DELIM ']' at 19>",
            "<DELIM ':' at 20>",
            "<IDENT 'nth' at 21>",
            "<DELIM '(' at 24>",
            "<NUMBER '-3.7' at 37>",
            "<DELIM ')' at 41>",
            "<EOF at 42>",
        ]
    );
}

#[test]
fn unclosed_string() {
    match tokenize("'foo") {
        Err(SelectorError::Syntax(msg)) => assert_eq!(msg, "Unclosed string at 0"),
        other => panic!("expected unclosed string error, got {other:?}"),
    }
}

#[test]
fn unicode_escape_over_max() {
    // A code point above the Unicode maximum clamps to U+FFFD.
    let tokens = tokenize(r"\110000").unwrap();
    assert_eq!(tokens[0].value_str(), "\u{FFFD}");
}

#[test]
fn negative_ident_vs_number() {
    // `-foo` is an identifier, `-3` is a number.
    assert_eq!(tokenize("-foo").unwrap()[0].value_str(), "-foo");
    assert_eq!(tokenize("-3").unwrap()[0].value_str(), "-3");
}

#[test]
fn unterminated_comment_consumes_to_end() {
    // A `/*` with no close swallows the rest and leaves only EOF.
    let tokens = tokenize("a /* unterminated").unwrap();
    assert_eq!(tokens.last().unwrap().to_string(), "<EOF at 17>");
}

#[test]
fn surrogate_escape_folds_to_replacement() {
    // A lone surrogate escape cannot live in a Rust string, so it folds to
    // U+FFFD, the same value an over-max escape produces.
    assert_eq!(tokenize(r"\D800").unwrap()[0].value_str(), "\u{FFFD}");
    assert_eq!(tokenize(r"\DFFF").unwrap()[0].value_str(), "\u{FFFD}");
    // A zero escape still maps to the null character.
    assert_eq!(tokenize(r"\0").unwrap()[0].value_str(), "\u{0}");
}

#[test]
fn comment_between_idents_is_skipped() {
    // The comment splits two idents and contributes no token.
    let tokens = token_strs("a/* c */b");
    assert_eq!(
        tokens,
        vec!["<IDENT 'a' at 0>", "<IDENT 'b' at 8>", "<EOF at 9>"]
    );
}

#[test]
fn line_continuation_in_string_is_removed() {
    // A backslash before a newline inside a string drops both characters.
    let value = tokenize("'line\\\na end'").unwrap()[0]
        .value_str()
        .to_string();
    assert_eq!(value, "linea end");
}

#[test]
fn hex_escape_in_ident() {
    // `\41` is the hex escape for 'A'.
    assert_eq!(tokenize(r"foo\41").unwrap()[0].value_str(), "fooA");
}

#[test]
fn leading_dot_and_signed_numbers() {
    // The number matcher accepts a leading dot and an optional sign.
    assert_eq!(tokenize("+.5").unwrap()[0].value_str(), "+.5");
    assert_eq!(tokenize("-.5").unwrap()[0].value_str(), "-.5");
    assert_eq!(tokenize(".5").unwrap()[0].value_str(), ".5");
    assert_eq!(tokenize("12.5").unwrap()[0].value_str(), "12.5");
}
