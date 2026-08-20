//! scrapling::xpath — one XPath query over HTML; first-or-all; `attr` pulls an
//! attribute else text (core.py `op_query(payload, "xpath")`).

use serde_json::{json, Value};

use crate::scrapling::{adaptive, dom, xpath};
use crate::scrapling::{ops::common, require_str};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let q = require_str(payload, "query")?;
    let doc = dom::parse(html);
    let adaptive_enabled = payload
        .get("adaptive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let attr = payload
        .get("attr")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let results = (if adaptive_enabled {
        let identifier = payload
            .get("identifier")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(q);
        let auto_save = match payload.get("auto_save") {
            None => true,
            Some(value) => value.as_bool().unwrap_or(false),
        };
        adaptive::xpath_query(
            &doc,
            None,
            q,
            payload.get("adaptive_domain").and_then(Value::as_str),
            identifier,
            auto_save,
        )
    } else {
        xpath::xpath_query(&doc, None, q)
    })
    .map_err(|error| {
        if attr.is_some() && error == "'str' object has no attribute 'iter'" {
            "'str' object has no attribute 'attrib'".to_string()
        } else {
            error
        }
    })?;
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
    fn invalid_selector_reports_the_original_expression() {
        let err = op(&json!({"html": "<p>x</p>", "query": "//["})).unwrap_err();
        assert_eq!(err, "Invalid XPath selector: //[");
    }

    #[test]
    fn scalar_attribute_pull_keeps_the_python_error() {
        let err = op(&json!({
            "html": "<p>x</p>",
            "query": "string(//p)",
            "attr": "href"
        }))
        .unwrap_err();
        assert_eq!(err, "'str' object has no attribute 'attrib'");
    }
}
