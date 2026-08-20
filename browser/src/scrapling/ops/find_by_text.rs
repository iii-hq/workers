//! scrapling::find-by-text — leading-text equality/containment search.

use serde_json::Value;

use crate::scrapling::{dom, text};
use crate::scrapling::{ops::common, require_str};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let query = require_str(payload, "text")?;
    let partial = payload
        .get("partial")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let case_sensitive = payload
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let clean_match = payload
        .get("clean_match")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let query = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let doc = dom::parse(html);
    let mut matches = Vec::new();
    for el in dom::descendant_elements(doc.root()) {
        if !dom::first_text_run_nonblank(el) {
            continue;
        }
        let mut node_text = dom::leading_text(el);
        if clean_match {
            node_text = text::clean(&node_text);
        }
        if !case_sensitive {
            node_text = node_text.to_lowercase();
        }
        let hit = if partial {
            node_text.contains(&query)
        } else {
            node_text == query
        };
        if hit {
            matches.push(el);
        }
    }
    Ok(common::bounded_items_response(&matches, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_leading_text_not_subtree() {
        // <li> has no leading text (its text lives in <a>); the <a> matches.
        let html = r#"<html><body><ul><li><a href="/a">Apple</a></li></ul></body></html>"#;
        let out = op(&json!({"html": html, "text": "Apple"})).unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["items"][0]["tag"], "a");
    }

    #[test]
    fn count_is_precap_total_when_first() {
        let html = r#"<html><body><p>x</p><p>x</p><p>x</p></body></html>"#;
        let out = op(&json!({"html": html, "text": "x", "first": true})).unwrap();
        assert_eq!(out["count"], 3);
        assert_eq!(out["items"].as_array().unwrap().len(), 1);
    }
}
