//! scrapling::find — BeautifulSoup-style tag/attrs/text-regex search.

use serde_json::Value;

use crate::scrapling::{dom, query, text};
use crate::scrapling::{ops::common, require_str};

pub(crate) fn py_str(v: &Value) -> String {
    match v {
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::String(s) => s.clone(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    // Python filters on truthiness (`if tag:`, `if text_regex:`): an empty
    // string or empty list is the same as absent. `attrs: {}` already falls
    // out empty below with no extra guard needed.
    let tags: Vec<String> = match payload.get("tag") {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        _ => vec![],
    };
    let attrs: Vec<(String, String)> = payload
        .get("attrs")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), py_str(v))).collect())
        .unwrap_or_default();
    let text_regex = payload
        .get("text_regex")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());

    if tags.is_empty() && attrs.is_empty() && text_regex.is_none() {
        return Err("provide at least one of `tag`, `attrs`, `text_regex`".to_string());
    }

    let doc = dom::parse(html);
    let mut matches: Vec<dom::ElementRef<'_>> = if !tags.is_empty() || !attrs.is_empty() {
        let base: Vec<String> = if tags.is_empty() {
            vec!["*".into()]
        } else {
            tags
        };
        let selector_strs: Vec<String> = base
            .iter()
            .map(|t| {
                let mut s = t.clone();
                for (k, v) in &attrs {
                    s.push_str(&format!("[{}=\"{}\"]", k, v.replace('"', "\\\"")));
                }
                s
            })
            .filter(|s| s != "*")
            .collect();
        if selector_strs.is_empty() {
            // Python's `find_all` builds one selector per tag, drops any that's
            // exactly the bare wildcard `"*"` (a lone `tag: "*"` with no attrs),
            // and only calls `self.css(...)` if anything survives; an all-"*"
            // request leaves the list empty and falls through to
            // `below_elements` (XPath `.//*`, every descendant) rather than
            // handing cssselect an empty, invalid selector string.
            dom::descendant_elements(doc.root())
        } else {
            let selector_str = selector_strs.join(", ");
            query::css_query(&doc, None, &selector_str)?
                .into_iter()
                .filter_map(|result| match result {
                    query::QueryResult::Element(element) => Some(element),
                    query::QueryResult::Text { .. } => None,
                })
                .collect()
        }
    } else {
        dom::descendant_elements(doc.root())
    };

    if let Some(pattern) = text_regex {
        let re = text::compile(pattern, false)?;
        matches.retain(|el| re.check_match(&dom::leading_text(*el)));
    }

    Ok(common::bounded_items_response(&matches, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_values_coerce_like_python_str() {
        assert_eq!(py_str(&serde_json::json!(true)), "True");
        assert_eq!(py_str(&serde_json::json!(false)), "False");
        assert_eq!(py_str(&serde_json::json!(5)), "5");
        assert_eq!(py_str(&serde_json::json!("x")), "x");
    }
}
