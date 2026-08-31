//! XPath 1.0 compatibility wrapper over the repository-owned xmloxide fork.

use xmloxide::tree::NodeKind;
use xmloxide::xpath::{XPathNode, XPathValue};

use crate::scrapling::dom::{Doc, ElementRef};
use crate::scrapling::query::QueryResult;

pub fn xpath_query<'a>(
    doc: &'a Doc,
    scope: Option<ElementRef<'a>>,
    expression: &str,
) -> Result<Vec<QueryResult<'a>>, String> {
    let context = scope.unwrap_or_else(|| doc.root());
    let value = xmloxide::xpath::evaluate(&doc.tree, context.id(), expression)
        .map_err(|_| format!("Invalid XPath selector: {expression}"))?;
    match value {
        XPathValue::NodeSet(nodes) => Ok(nodes
            .into_iter()
            .filter_map(|node| query_result(doc, node))
            .collect()),
        XPathValue::Boolean(false) => Ok(Vec::new()),
        XPathValue::Boolean(true) => Err("'bool' object is not iterable".to_string()),
        XPathValue::Number(0.0) => Ok(Vec::new()),
        XPathValue::Number(_) => Err("'float' object is not iterable".to_string()),
        XPathValue::String(value) if value.is_empty() => Ok(Vec::new()),
        XPathValue::String(_) => Err("'str' object has no attribute 'iter'".to_string()),
    }
}

fn query_result<'a>(doc: &'a Doc, node: XPathNode) -> Option<QueryResult<'a>> {
    match node {
        XPathNode::Attribute { owner, index } => {
            let attribute = doc.tree.attributes(owner).get(index as usize)?;
            Some(QueryResult::Text {
                value: attribute.value.clone(),
                parent: doc.element(owner)?,
            })
        }
        XPathNode::Node(id) => match &doc.tree.node(id).kind {
            NodeKind::Element { .. } => doc.element(id).map(QueryResult::Element),
            NodeKind::Text { content } | NodeKind::CData { content } => {
                let parent = element_parent(doc, id)?;
                Some(QueryResult::Text {
                    value: content.clone(),
                    parent,
                })
            }
            _ => None,
        },
    }
}

fn element_parent(doc: &Doc, mut node: xmloxide::NodeId) -> Option<ElementRef<'_>> {
    while let Some(parent) = doc.tree.parent(node) {
        if let Some(element) = doc.element(parent) {
            return Some(element);
        }
        node = parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrapling::dom;

    fn values(results: Vec<QueryResult<'_>>) -> Vec<String> {
        results
            .into_iter()
            .map(|result| match result {
                QueryResult::Element(element) => format!("<{}>", element.name()),
                QueryResult::Text { value, .. } => value,
            })
            .collect()
    }

    #[test]
    fn axes_functions_and_scalar_quirks_match_lxml_wrapper() {
        let doc = dom::parse("<main id='m'><section id='s'><p>A</p><p>B</p></section></main>");
        assert_eq!(
            values(xpath_query(&doc, None, "//p/text()").unwrap()),
            ["A", "B"]
        );
        assert_eq!(
            values(xpath_query(&doc, None, "//p/ancestor::*[1]/@id").unwrap()),
            ["s"]
        );
        assert!(xpath_query(&doc, None, "boolean(//nope)")
            .unwrap()
            .is_empty());
        assert_eq!(
            xpath_query(&doc, None, "boolean(//p)").unwrap_err(),
            "'bool' object is not iterable"
        );
    }
}
