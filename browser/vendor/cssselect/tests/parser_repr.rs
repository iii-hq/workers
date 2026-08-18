//! Parse-tree `repr` output and fast-path equivalence.

use cssselect::parse;

/// The `repr` of each parsed tree, asserting every pseudo-element is absent.
fn repr_parse(css: &str) -> Vec<String> {
    let selectors = parse(css).unwrap();
    for s in &selectors {
        assert!(
            s.pseudo_element.is_none(),
            "unexpected pseudo-element in {css:?}"
        );
    }
    selectors.iter().map(|s| s.parsed_tree.repr()).collect()
}

/// Assert every spelling parses to the same tree reprs as the first.
fn parse_many(first: &str, others: &[&str]) -> Vec<String> {
    let result = repr_parse(first);
    for other in others {
        assert_eq!(repr_parse(other), result, "mismatch for {other:?}");
    }
    result
}

#[test]
fn universal_and_namespaces() {
    assert_eq!(parse_many("*", &["*|*"]), vec!["Element[*]"]);
    assert_eq!(parse_many("*|foo", &["|foo"]), vec!["Element[foo]"]);
    assert_eq!(parse_many("foo|*", &[]), vec!["Element[foo|*]"]);
    assert_eq!(parse_many("foo|bar", &[]), vec!["Element[foo|bar]"]);
}

#[test]
fn stacked_hash() {
    assert_eq!(
        parse_many("#foo#bar", &[]),
        vec!["Hash[Hash[Element[*]#foo]#bar]"]
    );
}

#[test]
fn combinator_whitespace_equivalence() {
    assert_eq!(
        parse_many(
            "div>.foo",
            &[
                "div> .foo",
                "div >.foo",
                "div > .foo",
                "div \n>  \t \t .foo",
                "div\r>\n\n\n.foo",
                "div\u{0c}>\u{0c}.foo",
            ],
        ),
        vec!["CombinedSelector[Element[div] > Class[Element[*].foo]]"]
    );
}

#[test]
fn selector_groups() {
    assert_eq!(
        parse_many(
            "td.foo,.bar",
            &["td.foo, .bar", "td.foo\t\r\n\u{0c} ,\t\r\n\u{0c} .bar"]
        ),
        vec!["Class[Element[td].foo]", "Class[Element[*].bar]"]
    );
    assert_eq!(
        parse_many("div, td.foo, div.bar span", &[]),
        vec![
            "Element[div]",
            "Class[Element[td].foo]",
            "CombinedSelector[Class[Element[div].bar] <followed> Element[span]]",
        ]
    );
}

#[test]
fn attributes() {
    assert_eq!(
        parse_many("a[name]", &["a[ name\t]"]),
        vec!["Attrib[Element[a][name]]"]
    );
    assert_eq!(
        parse_many("a [name]", &[]),
        vec!["CombinedSelector[Element[a] <followed> Attrib[Element[*][name]]]"]
    );
    assert_eq!(
        parse_many("a[rel=\"include\"]", &["a[rel = include]"]),
        vec!["Attrib[Element[a][rel = 'include']]"]
    );
    assert_eq!(
        parse_many("a[hreflang |= 'en']", &["a[hreflang|=en]"]),
        vec!["Attrib[Element[a][hreflang |= 'en']]"]
    );
}

#[test]
fn functions_and_pseudos() {
    assert_eq!(
        parse_many("div:nth-child(10)", &[]),
        vec!["Function[Element[div]:nth-child(['10'])]"]
    );
    assert_eq!(
        parse_many(":nth-child(2n+2)", &[]),
        vec!["Function[Element[*]:nth-child(['2', 'n', '+2'])]"]
    );
    assert_eq!(
        parse_many("div:nth-of-type(10)", &[]),
        vec!["Function[Element[div]:nth-of-type(['10'])]"]
    );
    assert_eq!(
        parse_many("div div:nth-of-type(10) .aclass", &[]),
        vec![
            "CombinedSelector[CombinedSelector[Element[div] <followed> \
Function[Element[div]:nth-of-type(['10'])]] <followed> Class[Element[*].aclass]]"
        ]
    );
    assert_eq!(
        parse_many("label:only", &[]),
        vec!["Pseudo[Element[label]:only]"]
    );
    assert_eq!(
        parse_many("a:lang(fr)", &[]),
        vec!["Function[Element[a]:lang(['fr'])]"]
    );
    assert_eq!(
        parse_many("div:contains(\"foo\")", &[]),
        vec!["Function[Element[div]:contains(['foo'])]"]
    );
    assert_eq!(
        parse_many("div#foobar", &[]),
        vec!["Hash[Element[div]#foobar]"]
    );
    assert_eq!(
        parse_many("td:first", &[]),
        vec!["Pseudo[Element[td]:first]"]
    );
    assert_eq!(
        parse_many("td :first", &[]),
        vec!["CombinedSelector[Element[td] <followed> Pseudo[Element[*]:first]]"]
    );
}

#[test]
fn logical_combinators() {
    assert_eq!(
        parse_many("div:not(div.foo)", &[]),
        vec!["Negation[Element[div]:not(Class[Element[div].foo])]"]
    );
    assert_eq!(
        parse_many("div:has(div.foo)", &[]),
        vec!["Relation[Element[div]:has(Selector[Class[Element[div].foo]])]"]
    );
    assert_eq!(
        parse_many("div:is(.foo, #bar)", &[]),
        vec!["Matching[Element[div]:is(Class[Element[*].foo], Hash[Element[*]#bar])]"]
    );
    assert_eq!(
        parse_many(":is(:hover, :visited)", &[]),
        vec!["Matching[Element[*]:is(Pseudo[Element[*]:hover], Pseudo[Element[*]:visited])]"]
    );
    assert_eq!(
        parse_many(":where(:hover, :visited)", &[]),
        vec!["SpecificityAdjustment[Element[*]:where(Pseudo[Element[*]:hover], Pseudo[Element[*]:visited])]"]
    );
    assert_eq!(
        parse_many("td ~ th", &[]),
        vec!["CombinedSelector[Element[td] ~ Element[th]]"]
    );
}

#[test]
fn scope_placement() {
    assert_eq!(
        parse_many(":scope > foo", &[" :scope > foo"]),
        vec!["CombinedSelector[Pseudo[Element[*]:scope] > Element[foo]]"]
    );
    assert_eq!(
        parse_many(":scope > foo bar > div", &[]),
        vec![
            "CombinedSelector[CombinedSelector[CombinedSelector[Pseudo[Element[*]:scope] > \
Element[foo]] <followed> Element[bar]] > Element[div]]"
        ]
    );
    assert_eq!(
        parse_many(":scope > #foo #bar", &[]),
        vec![
            "CombinedSelector[CombinedSelector[Pseudo[Element[*]:scope] > \
Hash[Element[*]#foo]] <followed> Hash[Element[*]#bar]]"
        ]
    );
}

#[test]
fn fast_path_matches_slow_path() {
    // The fast-path regexes must yield the same trees as the tokenizer route.
    assert_eq!(repr_parse("foo"), repr_parse(" foo "));
    assert_eq!(repr_parse("#bar"), repr_parse(" #bar "));
    assert_eq!(repr_parse("foo#bar"), repr_parse(" foo#bar "));
    assert_eq!(repr_parse(".bar"), repr_parse(" .bar "));
    assert_eq!(repr_parse("foo.bar"), repr_parse(" foo.bar "));

    assert_eq!(repr_parse("foo"), vec!["Element[foo]"]);
    assert_eq!(repr_parse("#bar"), vec!["Hash[Element[*]#bar]"]);
    assert_eq!(repr_parse("foo#bar"), vec!["Hash[Element[foo]#bar]"]);
    assert_eq!(repr_parse(".bar"), vec!["Class[Element[*].bar]"]);
    assert_eq!(repr_parse("foo.bar"), vec!["Class[Element[foo].bar]"]);
}
