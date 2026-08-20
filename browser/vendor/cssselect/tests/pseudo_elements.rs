//! Pseudo-element parsing, `repr`, and default translation behavior.

use cssselect::{parse, GenericTranslator, PseudoElement, PseudoElements, SelectorError};

/// The tree `repr` and the pseudo-element as a display string for each selector.
fn parse_pseudo(css: &str) -> Vec<(String, Option<String>)> {
    parse(css)
        .unwrap()
        .iter()
        .map(|s| {
            let pe = s.pseudo_element.as_ref().map(pe_display);
            (s.parsed_tree.repr(), pe)
        })
        .collect()
}

/// The display form of a pseudo-element: the ident, or the functional repr.
fn pe_display(pe: &PseudoElement) -> String {
    match pe {
        PseudoElement::Ident(name) => name.clone(),
        PseudoElement::Functional(f) => f.repr(),
    }
}

/// Parse one selector and return its (tree repr, pseudo-element) pair.
fn parse_one(css: &str) -> (String, Option<String>) {
    let result = parse_pseudo(css);
    assert_eq!(result.len(), 1, "expected one selector for {css:?}");
    result.into_iter().next().unwrap()
}

#[test]
fn pseudo_classes_have_no_pseudo_element() {
    assert_eq!(parse_one("foo"), ("Element[foo]".into(), None));
    assert_eq!(parse_one("*"), ("Element[*]".into(), None));
    assert_eq!(
        parse_one(":empty"),
        ("Pseudo[Element[*]:empty]".into(), None)
    );
    assert_eq!(
        parse_one(":scope"),
        ("Pseudo[Element[*]:scope]".into(), None)
    );
}

#[test]
fn css21_single_colon_pseudo_elements() {
    assert_eq!(
        parse_one(":BEfore"),
        ("Element[*]".into(), Some("before".into()))
    );
    assert_eq!(
        parse_one(":aftER"),
        ("Element[*]".into(), Some("after".into()))
    );
    assert_eq!(
        parse_one(":First-Line"),
        ("Element[*]".into(), Some("first-line".into()))
    );
    assert_eq!(
        parse_one(":First-Letter"),
        ("Element[*]".into(), Some("first-letter".into()))
    );

    assert_eq!(
        parse_one("::befoRE"),
        ("Element[*]".into(), Some("before".into()))
    );
    assert_eq!(
        parse_one("::AFter"),
        ("Element[*]".into(), Some("after".into()))
    );
    assert_eq!(
        parse_one("::firsT-linE"),
        ("Element[*]".into(), Some("first-line".into()))
    );
    assert_eq!(
        parse_one("::firsT-letteR"),
        ("Element[*]".into(), Some("first-letter".into()))
    );
}

#[test]
fn arbitrary_pseudo_elements() {
    assert_eq!(
        parse_one("::text-content"),
        ("Element[*]".into(), Some("text-content".into()))
    );
    assert_eq!(
        parse_one("::attr(name)"),
        (
            "Element[*]".into(),
            Some("FunctionalPseudoElement[::attr(['name'])]".into())
        )
    );
    assert_eq!(
        parse_one("::Selection"),
        ("Element[*]".into(), Some("selection".into()))
    );
    assert_eq!(
        parse_one("foo:after"),
        ("Element[foo]".into(), Some("after".into()))
    );
    assert_eq!(
        parse_one("foo::selection"),
        ("Element[foo]".into(), Some("selection".into()))
    );
}

#[test]
fn pseudo_element_at_end_of_chain() {
    assert_eq!(
        parse_one("lorem#ipsum ~ a#b.c[href]:empty::selection"),
        (
            "CombinedSelector[Hash[Element[lorem]#ipsum] ~ \
Pseudo[Attrib[Class[Hash[Element[a]#b].c][href]]:empty]]"
                .into(),
            Some("selection".into())
        )
    );
}

#[test]
fn per_selector_in_group() {
    assert_eq!(
        parse_pseudo(":scope > div, foo bar"),
        vec![
            (
                "CombinedSelector[Pseudo[Element[*]:scope] > Element[div]]".into(),
                None
            ),
            (
                "CombinedSelector[Element[foo] <followed> Element[bar]]".into(),
                None
            ),
        ]
    );
    assert_eq!(
        parse_pseudo("foo:before, bar, baz:after"),
        vec![
            ("Element[foo]".into(), Some("before".into())),
            ("Element[bar]".into(), None),
            ("Element[baz]".into(), Some("after".into())),
        ]
    );
}

#[test]
fn css21_pseudo_elements_ignored_by_default() {
    for pseudo in ["after", "before", "first-line", "first-letter"] {
        let css = alloc_format(pseudo);
        let selectors = parse(&css).unwrap();
        assert_eq!(selectors.len(), 1);
        let sel = &selectors[0];
        assert_eq!(pe_display(sel.pseudo_element.as_ref().unwrap()), pseudo);
        assert_eq!(
            GenericTranslator::new()
                .selector_to_xpath_with(sel, "", PseudoElements::Ignore)
                .unwrap(),
            "e"
        );
    }
}

/// Build `e:<pseudo>`.
fn alloc_format(pseudo: &str) -> String {
    let mut s = String::from("e:");
    s.push_str(pseudo);
    s
}

#[test]
fn pseudo_elements_unsupported_when_translated() {
    let tr = GenericTranslator::new();
    let selectors = parse("e::foo").unwrap();
    let sel = &selectors[0];
    assert_eq!(pe_display(sel.pseudo_element.as_ref().unwrap()), "foo");
    assert_eq!(
        tr.selector_to_xpath_with(sel, "", PseudoElements::Ignore)
            .unwrap(),
        "e"
    );
    match tr.selector_to_xpath_with(sel, "descendant-or-self::", PseudoElements::Translate) {
        Err(SelectorError::Expression(_)) => {}
        other => panic!("expected ExpressionError, got {other:?}"),
    }
}

#[test]
fn unicode_repr_regression() {
    // The dotted capital I is preserved. ASCII lowercasing leaves it alone.
    let selectors = parse(":fİrst-child").unwrap();
    assert_eq!(
        selectors[0].parsed_tree.repr(),
        "Pseudo[Element[*]:fİrst-child]"
    );
    let scope = parse(":scope").unwrap();
    assert_eq!(scope[0].parsed_tree.repr(), "Pseudo[Element[*]:scope]");
}
