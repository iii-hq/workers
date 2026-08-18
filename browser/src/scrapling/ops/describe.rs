//! scrapling::describe — full identity + structure of the first css/xpath
//! match (core.py `op_describe`).

use serde_json::{json, Value};

use crate::scrapling::{dom, query, selgen};
use crate::scrapling::{ops::common, require_str};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let q = require_str(payload, "query")?;
    let doc = dom::parse(html);
    // Python is `kind.get("kind", "css") == "css"`: the "css" default applies
    // ONLY to a MISSING key. An explicit `null` (or any non-"css" value) is
    // not equal to "css", so it takes the xpath branch — match that.
    let is_css = match payload.get("kind") {
        None => true,
        Some(value) => value.as_str() == Some("css"),
    };
    let results = if is_css {
        query::css_query(&doc, None, q)?
    } else {
        crate::scrapling::xpath::xpath_query(&doc, None, q)?
    };
    let Some(first) = results.first() else {
        return Ok(json!({"found": false}));
    };
    let element = match first {
        query::QueryResult::Element(el) => {
            let parent = dom::parent_element(*el);
            let mut e = common::serialize_element(*el);
            let obj = e.as_object_mut().unwrap();
            obj.insert("full_css".into(), json!(selgen::full_css_selector(*el)));
            obj.insert("full_xpath".into(), json!(selgen::full_xpath_selector(*el)));
            obj.insert(
                "classes".into(),
                json!(el
                    .attr("class")
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()),
            );
            obj.insert(
                "parent_tag".into(),
                parent.map(|p| json!(p.name())).unwrap_or(Value::Null),
            );
            obj.insert("children".into(), json!(dom::element_children(*el).len()));
            obj.insert(
                "siblings".into(),
                json!(parent
                    .map(|p| dom::element_children(p).len().saturating_sub(1))
                    .unwrap_or(0)),
            );
            e
        }
        query::QueryResult::Text { value, parent } => json!({
            "tag": "#text",
            "text": value, "html": value, "attrs": {},
            "css": "", "xpath": "",
            "full_css": "", "full_xpath": "",
            "classes": [],
            "parent_tag": parent.name(),
            "children": 0,
            "siblings": dom::element_children(*parent).len(),
        }),
    };
    Ok(json!({"found": true, "element": element}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_is_exactly_found_false() {
        let out = op(&serde_json::json!({"html": "<p>x</p>", "query": ".nope"})).unwrap();
        assert_eq!(out, serde_json::json!({"found": false}));
        assert!(out.get("element").is_none());
    }
}
