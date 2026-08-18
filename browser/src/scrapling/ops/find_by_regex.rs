//! scrapling::find-by-regex — leading-text regex search.

use serde_json::Value;

use crate::scrapling::{dom, text};
use crate::scrapling::{ops::common, require_str};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let pattern = require_str(payload, "pattern")?;
    let case_sensitive = payload
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let clean_match = payload
        .get("clean_match")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let re = text::compile(pattern, !case_sensitive)?;
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
        if re.check_match(&node_text) {
            matches.push(el);
        }
    }
    Ok(common::bounded_items_response(&matches, payload))
}
