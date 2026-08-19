//! Canonical CSS serialization round-trips.

use cssselect::parse;

/// Parse `css` and assert its canonical form equals `expected`.
fn css2css(css: &str, expected: &str) {
    let selectors = parse(css).unwrap();
    assert_eq!(selectors.len(), 1, "expected one selector for {css:?}");
    assert_eq!(selectors[0].canonical(), expected, "for {css:?}");
}

#[test]
fn normalizations() {
    css2css("*", "*");
    css2css(" foo", "foo");
    css2css("Foo", "Foo");
    css2css(":empty ", ":empty");
    css2css(":before", "::before");
    css2css(":beFOre", "::before");
    css2css("*:before", "::before");
    css2css(":nth-child(2)", ":nth-child(2)");
    css2css(".bar", ".bar");
    css2css("[baz]", "[baz]");
    css2css("[baz=\"4\"]", "[baz='4']");
    css2css("[baz^=\"4\"]", "[baz^='4']");
    css2css("[ns|attr='4']", "[ns|attr='4']");
    css2css("#lipsum", "#lipsum");
}

#[test]
fn logical_pseudos() {
    css2css(":not(*)", ":not(*)");
    css2css(":not(foo)", ":not(foo)");
    css2css(":not(*.foo)", ":not(.foo)");
    css2css(":not(*[foo])", ":not([foo])");
    css2css(":not(:empty)", ":not(:empty)");
    css2css(":not(#foo)", ":not(#foo)");
    css2css(":has(*)", ":has(*)");
    css2css(":has(foo)", ":has(foo)");
    css2css(":has(*.foo)", ":has(.foo)");
    css2css(":is(#bar, .foo)", ":is(#bar, .foo)");
    css2css(":is(a,b)", ":is(a, b)");
    css2css(":is(:focused, :visited)", ":is(:focused, :visited)");
    css2css(":where(:focused, :visited)", ":where(:focused, :visited)");
}

#[test]
fn pseudo_elements_and_combinators() {
    css2css("foo:empty", "foo:empty");
    css2css("foo::before", "foo::before");
    css2css("foo:empty::before", "foo:empty::before");
    css2css("::name(arg + \"val\" - 3)", "::name(arg+'val'-3)");
    css2css(
        "#lorem + foo#ipsum:first-child > bar::first-line",
        "#lorem + foo#ipsum:first-child > bar::first-line",
    );
    css2css("foo > *", "foo > *");
}

#[test]
fn string_value_escaping() {
    // The value repr emits short escapes for tab and newline.
    css2css("[a=\"\\9 tab\"]", "[a='\\ttab']");
    css2css("[a=\"line\\a break\"]", "[a='line\\nbreak']");
    // The quote choice flips to double quotes when the value holds a single
    // quote and no double quote.
    css2css("[a='it\\'s']", "[a=\"it's\"]");
    css2css("[a=\"b\\\"c\"]", "[a='b\"c']");
}

#[test]
fn matching_aliases_and_collapses() {
    // `:matches` canonicalizes to `:is`.
    css2css(":matches(a, b)", ":is(a, b)");
    // A universal-only `:where` collapses its argument to empty.
    css2css(":where(*)", ":where()");
    css2css(":is(div, .a, #b)", ":is(div, .a, #b)");
}
