//! scrapling::css — one CSS query over HTML; first-or-all; `attr` pulls an
//! attribute else text (core.py `op_query(payload, "css")`).

use serde_json::{json, Value};

use crate::scrapling::{adaptive, dom, query};
use crate::scrapling::{ops::common, require_str};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let q = require_str(payload, "query")?;
    let doc = dom::parse(html);
    let adaptive_enabled = payload
        .get("adaptive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let results = if adaptive_enabled {
        let identifier = payload
            .get("identifier")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(q);
        let auto_save = match payload.get("auto_save") {
            None => true,
            Some(value) => value.as_bool().unwrap_or(false),
        };
        adaptive::css_query(
            &doc,
            None,
            q,
            payload.get("adaptive_domain").and_then(Value::as_str),
            identifier,
            auto_save,
        )?
    } else {
        query::css_query(&doc, None, q)?
    };
    let attr = payload
        .get("attr")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if payload
        .get("first")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Ok(json!({ "result": results.first().map(|r| common::pull(r, attr, false)) }))
    } else {
        Ok(
            json!({ "result": results.iter().map(|r| common::pull(r, attr, false)).collect::<Vec<_>>() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adaptive_adjacent_fields_without_adaptive_are_ignored() {
        let out = op(&json!({
            "html": "<html><body><p>x</p></body></html>",
            "query": "p", "first": true,
            "auto_save": true, "adaptive_domain": "example.com", "identifier": "k"
        }))
        .unwrap();
        assert_eq!(out, json!({"result": "x"}));
    }
}
