//! Selection tier: run the generated XPath against a real engine.
//!
//! These cases confirm the generated XPath selects the right nodes, not only
//! that the string matches a golden. They run against the clean XML fixtures,
//! which a standard XML parser accepts. Build with
//! `--features xpath-engine-tests` to include this file.

use cssselect::GenericTranslator;
use sxd_document::parser;
use sxd_xpath::{Context, Factory, Value};

const OPERATOR_PRECEDENCE: &str = include_str!("fixtures/operator_precedence.xml");

/// A small clean-XML corpus for structural and attribute selection. The engine
/// here lacks the XPath `lang()` function, so the generic `:lang()` cases live
/// in the string-parity tests instead.
const CORPUS: &str = r#"
<root>
  <ol id="list" class="a b c">
    <li id="one">first</li>
    <li id="two" class="x">second</li>
    <li id="three" class="x y">third</li>
    <li id="four"></li>
  </ol>
  <p id="para">text</p>
  <a id="link" href="http://example.org/page" hreflang="en-US">link</a>
</root>
"#;

/// Run a CSS selector against an XML fixture and return the matched `id`s in
/// document order.
fn select_ids(fixture: &str, selector: &str) -> Vec<String> {
    let package = parser::parse(fixture).expect("fixture parses as XML");
    let document = package.as_document();
    let xpath_str = GenericTranslator::new()
        .css_to_xpath(selector)
        .expect("selector translates");

    let factory = Factory::new();
    let xpath = factory
        .build(&xpath_str)
        .expect("xpath compiles")
        .expect("xpath is not empty");
    let context = Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .expect("xpath evaluates");

    let mut ids = Vec::new();
    if let Value::Nodeset(nodes) = value {
        for node in nodes.document_order() {
            if let Some(element) = node.element() {
                let id = element.attribute_value("id").unwrap_or("nil").to_string();
                ids.push(id);
            }
        }
    }
    ids
}

#[test]
fn structural_selection() {
    assert_eq!(select_ids(CORPUS, "li"), ["one", "two", "three", "four"]);
    assert_eq!(select_ids(CORPUS, "li:first-child"), ["one"]);
    assert_eq!(select_ids(CORPUS, "li:last-child"), ["four"]);
    assert_eq!(select_ids(CORPUS, "li:nth-child(2)"), ["two"]);
    assert_eq!(select_ids(CORPUS, "li:nth-child(odd)"), ["one", "three"]);
    assert_eq!(select_ids(CORPUS, "li:empty"), ["four"]);
    assert_eq!(select_ids(CORPUS, "ol > li:nth-of-type(3)"), ["three"]);
}

#[test]
fn attribute_and_class_selection() {
    assert_eq!(select_ids(CORPUS, ".x"), ["two", "three"]);
    assert_eq!(select_ids(CORPUS, "li.x.y"), ["three"]);
    assert_eq!(select_ids(CORPUS, "[href]"), ["link"]);
    assert_eq!(select_ids(CORPUS, "[href^=\"http\"]"), ["link"]);
    assert_eq!(select_ids(CORPUS, "[href$=\"page\"]"), ["link"]);
    assert_eq!(select_ids(CORPUS, "[href*=\"example\"]"), ["link"]);
    assert_eq!(select_ids(CORPUS, "[class~=\"y\"]"), ["three"]);
    assert_eq!(select_ids(CORPUS, "[hreflang|=\"en\"]"), ["link"]);
    assert!(select_ids(CORPUS, "[hreflang|=\"e\"]").is_empty());
    assert!(select_ids(CORPUS, "[href*=\"\"]").is_empty());
}

#[test]
fn combinator_selection() {
    assert_eq!(select_ids(CORPUS, "li + li"), ["two", "three", "four"]);
    assert_eq!(select_ids(CORPUS, "li ~ li"), ["two", "three", "four"]);
    assert_eq!(
        select_ids(CORPUS, "ol > li"),
        ["one", "two", "three", "four"]
    );
    assert_eq!(select_ids(CORPUS, "ol li:not(.x)"), ["one", "four"]);
    assert_eq!(select_ids(CORPUS, "ol:has(li.y)"), ["list"]);
    assert_eq!(select_ids(CORPUS, "li:is(#one, #four)"), ["one", "four"]);
}

#[test]
fn operator_precedence() {
    // `:first-or-second` is not a built-in pseudo-class. Use plain selectors
    // that the engine can evaluate and check the selected ids.
    assert_eq!(
        select_with_id_predicate(":has(*)", "a"),
        Vec::<String>::new()
    );
    assert_eq!(select_with_id_predicate("[href]", "a"), ["second", "third"]);
}

/// Select `tag` elements that match `selector`, returning their ids.
fn select_with_id_predicate(selector: &str, tag: &str) -> Vec<String> {
    let combined = format!("{tag}{selector}");
    select_ids(OPERATOR_PRECEDENCE, &combined)
}
