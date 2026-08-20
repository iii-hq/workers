//! Scrapling CSS queries: cssselect translation evaluated by the shared XPath
//! engine, against the same libxml-compatible tree and context node.

use crate::scrapling::dom::{Doc, ElementRef};

#[derive(Debug, Clone)]
pub enum QueryResult<'a> {
    Element(ElementRef<'a>),
    Text {
        value: String,
        parent: ElementRef<'a>,
    },
}

pub fn css_query<'a>(
    doc: &'a Doc,
    scope: Option<ElementRef<'a>>,
    selector: &str,
) -> Result<Vec<QueryResult<'a>>, String> {
    let xpath = cssselect::HtmlTranslator::new()
        .css_to_xpath(selector)
        .map_err(|error| format!("Invalid CSS selector '{selector}': {error}"))?;
    crate::scrapling::xpath::xpath_query(doc, scope, &xpath)
        .map_err(|error| format!("Invalid CSS selector '{selector}': {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrapling::dom;

    fn values(results: &[QueryResult<'_>]) -> Vec<String> {
        results
            .iter()
            .map(|result| match result {
                QueryResult::Element(element) => format!("<{}>", element.name()),
                QueryResult::Text { value, .. } => value.clone(),
            })
            .collect()
    }

    #[test]
    fn pseudo_elements_and_scopes_use_xpath_semantics() {
        let doc = dom::parse("<main><section><a href='/a'>A<b>B</b>C</a></section></main>");
        assert_eq!(
            values(&css_query(&doc, None, "a::text").unwrap()),
            ["A", "C"]
        );
        assert_eq!(
            values(&css_query(&doc, None, "a ::text").unwrap()),
            ["A", "B", "C"]
        );
        assert_eq!(
            values(&css_query(&doc, None, "a::attr(href)").unwrap()),
            ["/a"]
        );

        let section = doc.first_by_tag("section").unwrap();
        assert!(css_query(&doc, Some(section), "body a").unwrap().is_empty());
    }
}
