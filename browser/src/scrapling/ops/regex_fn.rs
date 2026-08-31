//! scrapling::regex — regex over the visible text of the document.

use serde_json::{json, Value};

use crate::scrapling::require_str;
use crate::scrapling::{dom, text};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let pattern = require_str(payload, "pattern")?;
    let doc = dom::parse(html);
    let all_text = dom::get_all_text(doc.root(), "\n", false, &["script", "style"], true);
    let re = text::compile(pattern, false)?;
    if payload
        .get("first")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Ok(json!({ "result": re.find_first(&all_text) }))
    } else {
        Ok(json!({ "result": re.findall(&all_text) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<html><body><p>price 42 usd then 99</p></body></html>"#;

    #[test]
    fn all_matches_by_default() {
        let out = op(&json!({"html": HTML, "pattern": r"\d+"})).unwrap();
        assert_eq!(out, json!({"result": ["42", "99"]}));
    }

    #[test]
    fn first_returns_scalar_or_null() {
        assert_eq!(
            op(&json!({"html": HTML, "pattern": r"price (\d+)", "first": true})).unwrap(),
            json!({"result": "42"})
        );
        assert_eq!(
            op(&json!({"html": HTML, "pattern": "zzz", "first": true})).unwrap(),
            json!({"result": null})
        );
    }

    #[test]
    fn missing_html_or_pattern_is_an_error() {
        assert!(op(&json!({"pattern": "x"})).is_err());
        assert!(op(&json!({"html": HTML})).is_err());
    }
}
