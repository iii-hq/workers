//! Shared response-shaping helpers (core.py `_bounded`, `serialize_element`).

use serde_json::{json, Value};

use crate::scrapling::{dom, dom::ElementRef, selgen};

pub const MAX_FIND_ITEMS: usize = 100;

pub fn bounded(total: usize, payload: &Value) -> usize {
    if payload
        .get("first")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return total.min(1);
    }
    // `as_i64` is None for a JSON float or numeric string (an LLM emitting
    // `"limit": 5.0` is common), which would silently fall back to the 100
    // cap; coerce those too before defaulting.
    let limit = payload.get("limit").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
    });
    let ceiling = match limit {
        // Clamp to [0, MAX]: limit 0 → none; negative must NOT slice from the end.
        Some(l) => l.clamp(0, MAX_FIND_ITEMS as i64) as usize,
        None => MAX_FIND_ITEMS,
    };
    total.min(ceiling)
}

pub fn serialize_element(el: ElementRef) -> Value {
    json!({
        "tag": el.name(),
        "text": dom::get_all_text(el, "\n", true, &["script", "style"], true),
        "html": dom::outer_html(el),
        "attrs": dom::attrs_json(el),
        "css": selgen::css_selector(el),
        "xpath": selgen::xpath_selector(el),
    })
}

/// Shared `{"count", "items"}` tail for the find* ops: `count` is the
/// pre-cap total, `items` the `bounded` slice, serialized.
pub fn bounded_items_response(matches: &[ElementRef], payload: &Value) -> Value {
    let cap = bounded(matches.len(), payload);
    json!({
        "count": matches.len(),
        "items": matches[..cap].iter().map(|e| serialize_element(*e)).collect::<Vec<_>>(),
    })
}

/// core.py `_pull`: attr → attribute value or `null` (Text results: always
/// `null`, they have no attributes of their own); `html` → outer HTML (Text:
/// the string itself); else `get_all_text` (Text: the string itself).
pub fn pull(
    result: &crate::scrapling::query::QueryResult,
    attr: Option<&str>,
    html: bool,
) -> Value {
    match result {
        crate::scrapling::query::QueryResult::Element(el) => {
            if let Some(a) = attr {
                el.attr(a).map(Into::into).unwrap_or(Value::Null)
            } else if html {
                Value::String(crate::scrapling::dom::outer_html(*el))
            } else {
                Value::String(crate::scrapling::dom::get_all_text(
                    *el,
                    "\n",
                    false,
                    &["script", "style"],
                    true,
                ))
            }
        }
        crate::scrapling::query::QueryResult::Text { value, .. } => {
            if attr.is_some() {
                Value::Null
            } else {
                Value::String(value.clone())
            }
        }
    }
}
