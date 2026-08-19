//! scrapling::find-similar — structural auto-match: given one example
//! element (the first css match), return it plus elements judged alike by
//! `similar::find_similar`'s depth/tag-chain/attribute scoring (core.py
//! `op_find_similar`). No adaptive refusal: Python's `op_find_similar` never
//! reads `adaptive` either (same precedent as `describe`).

use serde_json::{json, Value};

use crate::scrapling::{dom, query, similar};
use crate::scrapling::{ops::extract, require_str};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let anchor_query = require_str(payload, "anchor")?;
    let doc = dom::parse(html);
    let results = query::css_query(&doc, None, anchor_query)?;
    let Some(first) = results.first() else {
        return Ok(json!({"count": 0, "items": []}));
    };

    // A `::text`/`::attr()` anchor has no subtree to search: no similars,
    // and — per the ledger — always the plain default item, even when
    // `selectors` is given. (Python's `Selector` wraps text nodes too and
    // would run `apply_selectors` against the bare string instead, but
    // `apply_selectors` here takes an `ElementRef` ctx with nothing text-node
    // shaped to pass it, no fixture exercises that combination, and
    // `extract.rs` is outside this task's touched files.)
    let anchor = match first {
        query::QueryResult::Text { value, .. } => {
            return Ok(json!({"count": 1, "items": [{"text": value, "html": value}]}));
        }
        query::QueryResult::Element(el) => *el,
    };

    let threshold = payload
        .get("similarity_threshold")
        .and_then(Value::as_f64)
        .unwrap_or(0.2);
    let match_text = payload
        .get("match_text")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut elements = vec![anchor];
    elements.extend(similar::find_similar(&doc, anchor, threshold, match_text));

    // Python truthiness: an empty `selectors` list is the same as absent.
    let specs = payload
        .get("selectors")
        .and_then(Value::as_array)
        .filter(|specs| !specs.is_empty());

    let items = elements
        .iter()
        .map(|&el| match specs {
            Some(specs) => extract::apply_selectors(&doc, el, specs),
            None => Ok(default_item(el)),
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(json!({"count": items.len(), "items": items}))
}

/// `_default_item`: note `strip=false` here — unlike `common::serialize_element`.
fn default_item(el: dom::ElementRef<'_>) -> Value {
    json!({
        "text": dom::get_all_text(el, "\n", false, &["script", "style"], true),
        "html": dom::outer_html(el),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_anchor_has_no_similars_and_uses_the_value_as_both_fields() {
        // Ledger: a `::text` anchor short-circuits `find_similar` (no
        // subtree to search) and always reports one default item built from
        // the extracted string itself. Oracle-checked directly against
        // `core.op_find_similar({"html": ..., "anchor": "p::text"})`.
        let out = op(&json!({"html": "<p>hi</p>", "anchor": "p::text"})).unwrap();
        assert_eq!(
            out,
            json!({"count": 1, "items": [{"text": "hi", "html": "hi"}]})
        );
    }

    #[test]
    fn missing_attr_anchor_is_no_match_not_a_text_result() {
        // `p::attr(class)` on a `<p>` with no `class`: css_query yields no
        // result at all (nothing to extract), so this is the ordinary
        // anchor-missing path, not the text-node short-circuit.
        let out = op(&json!({"html": "<p>hi</p>", "anchor": "p::attr(class)"})).unwrap();
        assert_eq!(out, json!({"count": 0, "items": []}));
    }
}
