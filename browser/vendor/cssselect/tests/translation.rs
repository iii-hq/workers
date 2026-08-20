//! CSS to XPath translation strings produced by the generic translator.
//!
//! Each case asserts the exact XPath string with an empty prefix, pinning the
//! output character for character.

use cssselect::{GenericTranslator, SelectorError};

/// Translate with an empty prefix.
fn xpath(css: &str) -> String {
    GenericTranslator::new()
        .css_to_xpath_with_prefix(css, "")
        .unwrap()
}

/// Translate and expect an expression error.
fn err(css: &str) {
    match GenericTranslator::new().css_to_xpath_with_prefix(css, "") {
        Err(SelectorError::Expression(_)) => {}
        other => panic!("expected ExpressionError for {css:?}, got {other:?}"),
    }
}

/// Translate and return the expression error text.
fn err_text(css: &str) -> String {
    match GenericTranslator::new().css_to_xpath_with_prefix(css, "") {
        Err(SelectorError::Expression(msg)) => msg,
        other => panic!("expected ExpressionError for {css:?}, got {other:?}"),
    }
}

#[test]
fn elements_and_namespaces() {
    assert_eq!(xpath("*"), "*");
    assert_eq!(xpath("e"), "e");
    assert_eq!(xpath("*|e"), "e");
    assert_eq!(xpath("e|f"), "e:f");
}

#[test]
fn attribute_operators() {
    assert_eq!(xpath("e[foo]"), "e[@foo]");
    assert_eq!(xpath("e[foo|bar]"), "e[@foo:bar]");
    assert_eq!(xpath("e[foo=\"bar\"]"), "e[@foo = 'bar']");
    assert_eq!(
        xpath("e[foo~=\"bar\"]"),
        "e[@foo and contains(concat(' ', normalize-space(@foo), ' '), ' bar ')]"
    );
    assert_eq!(
        xpath("e[foo^=\"bar\"]"),
        "e[@foo and starts-with(@foo, 'bar')]"
    );
    assert_eq!(
        xpath("e[foo$=\"bar\"]"),
        "e[@foo and substring(@foo, string-length(@foo)-2) = 'bar']"
    );
    assert_eq!(
        xpath("e[foo*=\"bar\"]"),
        "e[@foo and contains(@foo, 'bar')]"
    );
    assert_eq!(
        xpath("e[hreflang|=\"en\"]"),
        "e[@hreflang and (@hreflang = 'en' or starts-with(@hreflang, 'en-'))]"
    );
}

#[test]
fn attribute_different_operator() {
    assert_eq!(xpath("e[foo!=\"bar\"]"), "e[not(@foo) or @foo != 'bar']");
    assert_eq!(xpath("e[foo!=\"\"]"), "e[@foo != '']");
}

#[test]
fn matching_keeps_the_outer_selector_conjunctive() {
    assert_eq!(
        xpath("#root:is(.a, .b)"),
        "*[(@id = 'root') and ((@class and contains(concat(' ', normalize-space(@class), ' '), ' a ')) or (@class and contains(concat(' ', normalize-space(@class), ' '), ' b ')))]"
    );
}

#[test]
fn nth_child_family() {
    assert_eq!(
        xpath("e:nth-child(1)"),
        "e[count(preceding-sibling::*) = 0]"
    );
    assert_eq!(xpath("e:nth-child(n)"), "e");
    assert_eq!(xpath("e:nth-child(n+1)"), "e");
    assert_eq!(xpath("e:nth-child(n-10)"), "e");
    assert_eq!(
        xpath("e:nth-child(n+2)"),
        "e[count(preceding-sibling::*) >= 1]"
    );
    assert_eq!(xpath("e:nth-child(-n)"), "e[0]");
    assert_eq!(
        xpath("e:nth-child(-n+1)"),
        "e[count(preceding-sibling::*) <= 0]"
    );
    assert_eq!(
        xpath("e:nth-child(3n+2)"),
        "e[(count(preceding-sibling::*) >= 1) and ((count(preceding-sibling::*) +2) mod 3 = 0)]"
    );
    assert_eq!(
        xpath("e:nth-child(3n-2)"),
        "e[count(preceding-sibling::*) mod 3 = 0]"
    );
    assert_eq!(
        xpath("e:nth-child(-n+6)"),
        "e[count(preceding-sibling::*) <= 5]"
    );
}

#[test]
fn nth_child_negative_step() {
    // A negative `a` keeps its sign in the emitted `mod` and uses a
    // non-negative offset.
    assert_eq!(
        xpath("e:nth-child(-3n+2)"),
        "e[(count(preceding-sibling::*) <= 1) and ((count(preceding-sibling::*) +2) mod -3 = 0)]"
    );
    assert_eq!(
        xpath("e:nth-child(-2n+4)"),
        "e[(count(preceding-sibling::*) <= 3) and ((count(preceding-sibling::*) +1) mod -2 = 0)]"
    );
    // A negative step with a negative offset can never match.
    assert_eq!(xpath("e:nth-child(-3n-2)"), "e[0]");
    assert_eq!(
        xpath("e:nth-child(10n+5)"),
        "e[(count(preceding-sibling::*) >= 4) and ((count(preceding-sibling::*) +6) mod 10 = 0)]"
    );
    assert_eq!(
        xpath("e:nth-child(2n-1)"),
        "e[count(preceding-sibling::*) mod 2 = 0]"
    );
}

#[test]
fn nth_child_minimum_index_does_not_panic() {
    assert_eq!(xpath("p:nth-child(-9223372036854775808)"), "p[0]");
}

#[test]
fn nth_child_minimum_step_does_not_panic() {
    assert_eq!(
        xpath("p:nth-child(-9223372036854775808n+1)"),
        "p[count(preceding-sibling::*) = 0]"
    );
}

#[test]
fn suffix_match_unicode_offset() {
    // The offset is the code-point length minus one, not the byte length.
    assert_eq!(
        xpath("e[foo$=\"é\"]"),
        "e[@foo and substring(@foo, string-length(@foo)-0) = 'é']"
    );
    assert_eq!(
        xpath("e[foo$=\"ab\"]"),
        "e[@foo and substring(@foo, string-length(@foo)-1) = 'ab']"
    );
    assert_eq!(
        xpath("e[foo$=\"résumé\"]"),
        "e[@foo and substring(@foo, string-length(@foo)-5) = 'résumé']"
    );
}

#[test]
fn generic_lang_output() {
    // The generic translator emits the XPath `lang()` function.
    assert_eq!(xpath(":lang(fr)"), "*[lang('fr')]");
    assert_eq!(xpath(":lang(\"en-US\")"), "*[lang('en-US')]");
}

#[test]
fn nth_last_child_family() {
    assert_eq!(
        xpath("e:nth-last-child(1)"),
        "e[count(following-sibling::*) = 0]"
    );
    assert_eq!(
        xpath("e:nth-last-child(2n)"),
        "e[(count(following-sibling::*) +1) mod 2 = 0]"
    );
    assert_eq!(
        xpath("e:nth-last-child(2n+1)"),
        "e[count(following-sibling::*) mod 2 = 0]"
    );
    assert_eq!(
        xpath("e:nth-last-child(2n+2)"),
        "e[(count(following-sibling::*) >= 1) and ((count(following-sibling::*) +1) mod 2 = 0)]"
    );
    assert_eq!(
        xpath("e:nth-last-child(3n+1)"),
        "e[count(following-sibling::*) mod 3 = 0]"
    );
    assert_eq!(
        xpath("e:nth-last-child(-n+2)"),
        "e[count(following-sibling::*) <= 1]"
    );
}

#[test]
fn nth_of_type() {
    assert_eq!(
        xpath("e:nth-of-type(1)"),
        "e[count(preceding-sibling::e) = 0]"
    );
    assert_eq!(
        xpath("e:nth-last-of-type(1)"),
        "e[count(following-sibling::e) = 0]"
    );
    assert_eq!(
        xpath("div e:nth-last-of-type(1) .aclass"),
        "div/descendant-or-self::*/e[count(following-sibling::e) = 0]\
/descendant-or-self::*/*[@class and contains(\
concat(' ', normalize-space(@class), ' '), ' aclass ')]"
    );
}

#[test]
fn structural_pseudos() {
    assert_eq!(xpath("e:first-child"), "e[count(preceding-sibling::*) = 0]");
    assert_eq!(xpath("e:last-child"), "e[count(following-sibling::*) = 0]");
    assert_eq!(
        xpath("e:first-of-type"),
        "e[count(preceding-sibling::e) = 0]"
    );
    assert_eq!(
        xpath("e:last-of-type"),
        "e[count(following-sibling::e) = 0]"
    );
    assert_eq!(xpath("e:only-child"), "e[count(parent::*/child::*) = 1]");
    assert_eq!(xpath("e:only-of-type"), "e[count(parent::*/child::e) = 1]");
    assert_eq!(xpath("e:empty"), "e[not(*) and not(string-length())]");
    assert_eq!(xpath("e:EmPTY"), "e[not(*) and not(string-length())]");
    assert_eq!(xpath("e:root"), "e[not(parent::*)]");
    assert_eq!(xpath("e:hover"), "e[0]");
}

#[test]
fn has_relations() {
    assert_eq!(
        xpath("div:has(bar.foo)"),
        "div[descendant::bar[@class and contains(concat(' ', normalize-space(@class), ' '), ' foo ')]]"
    );
    assert_eq!(xpath("e:has(> f)"), "e[./f]");
    assert_eq!(
        xpath("e:has(> f.foo)"),
        "e[./f[@class and contains(concat(' ', normalize-space(@class), ' '), ' foo ')]]"
    );
    assert_eq!(xpath("e:has(f)"), "e[descendant::f]");
    assert_eq!(xpath("e:has(~ f)"), "e[following-sibling::f]");
    assert_eq!(
        xpath("e:has(~ f.foo)"),
        "e[following-sibling::f[@class and contains(concat(' ', normalize-space(@class), ' '), ' foo ')]]"
    );
    assert_eq!(
        xpath("e:has(+ f)"),
        "e[following-sibling::*[(self::f) and (position() = 1)]]"
    );
    assert_eq!(
        xpath("e:has(+ f.foo)"),
        "e[following-sibling::*[((@class and contains(concat(' ', normalize-space(@class), ' '), ' foo ')) and (self::f)) and (position() = 1)]]"
    );
    assert_eq!(
        xpath("e:has(+ .foo)"),
        "e[following-sibling::*[(@class and contains(concat(' ', normalize-space(@class), ' '), ' foo ')) and (position() = 1)]]"
    );
}

#[test]
fn contains_class_and_id() {
    assert_eq!(xpath("e:contains(\"foo\")"), "e[contains(., 'foo')]");
    assert_eq!(xpath("e:ConTains(foo)"), "e[contains(., 'foo')]");
    assert_eq!(
        xpath("e.warning"),
        "e[@class and contains(concat(' ', normalize-space(@class), ' '), ' warning ')]"
    );
    assert_eq!(xpath("e#myid"), "e[@id = 'myid']");
}

#[test]
fn negation() {
    assert_eq!(
        xpath("e:not(:nth-child(odd))"),
        "e[not(count(preceding-sibling::*) mod 2 = 0)]"
    );
    assert_eq!(xpath("e:nOT(*)"), "e[0]");
}

#[test]
fn combinators() {
    assert_eq!(xpath("e f"), "e/descendant-or-self::*/f");
    assert_eq!(xpath("e > f"), "e/f");
    assert_eq!(
        xpath("e + f"),
        "e/following-sibling::*[(self::f) and (position() = 1)]"
    );
    assert_eq!(xpath("e ~ f"), "e/following-sibling::f");
    assert_eq!(
        xpath("e ~ f:nth-child(3)"),
        "e/following-sibling::f[count(preceding-sibling::*) = 2]"
    );
    assert_eq!(
        xpath("div#container p"),
        "div[@id = 'container']/descendant-or-self::*/p"
    );
}

#[test]
fn where_matching() {
    assert_eq!(xpath("e:where(foo)"), "e[self::foo]");
    assert_eq!(xpath("e:where(foo, bar)"), "e[(self::foo) or (self::bar)]");
}

#[test]
fn unsafe_xpath_names() {
    assert_eq!(xpath(r"di\a0 v"), "*[name() = 'di\u{a0}v']");
    assert_eq!(xpath(r"di\[v"), "*[name() = 'di[v']");
    assert_eq!(
        xpath(r"[h\a0 ref]"),
        "*[attribute::*[name() = 'h\u{a0}ref']]"
    );
    assert_eq!(xpath(r"[h\]ref]"), "*[attribute::*[name() = 'h]ref']]");
}

#[test]
fn expression_errors() {
    err(":fİrst-child");
    err(":first-of-type");
    err(":only-of-type");
    err(":last-of-type");
    err(":nth-of-type(1)");
    err(":nth-last-of-type(1)");
    err(":nth-child(n-)");
    err(":after");
    err(":lorem-ipsum");
    err(":lorem(ipsum)");
    err("::lorem-ipsum");
}

#[test]
fn expression_error_text_echoes_tokens() {
    // The three expression errors that quote their argument tokens must keep
    // the token list character for character.
    assert_eq!(
        err_text(":nth-child(n-)"),
        "Invalid series: '[<IDENT 'n-' at 11>]'"
    );
    assert_eq!(
        err_text(":contains(1)"),
        "Expected a single string or ident for :contains(), got [<NUMBER '1' at 10>]"
    );
    assert_eq!(
        err_text(":lang(1)"),
        "Expected a single string or ident for :lang(), got [<NUMBER '1' at 6>]"
    );
}

#[test]
fn large_nth_child_coefficient_does_not_panic() {
    // A coefficient too large for i64 must error, not overflow.
    match GenericTranslator::new().css_to_xpath("p:nth-child(99999999999999999999n+1)") {
        Err(SelectorError::Expression(_)) => {}
        other => panic!("expected an expression error, got {other:?}"),
    }
}

#[test]
fn default_prefix() {
    assert_eq!(
        GenericTranslator::new().css_to_xpath("e").unwrap(),
        "descendant-or-self::e"
    );
}
