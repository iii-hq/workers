//! scrapling::extract — a declarative selector list (css/xpath/regex,
//! attr/html/all) applied to one context element (core.py `apply_selectors`
//! + `op_extract`). Also the find-similar per-item projection (Task 15).

use serde_json::{json, Value};

use crate::scrapling::{adaptive, dom, query, text};
use crate::scrapling::{ops::common, require_str};

/// Precedence per spec: `regex` short-circuits css/xpath; else css beats
/// xpath; no query at all → `[]`/`null` by `all`. Empty-string
/// css/xpath/regex are falsy (Python `spec.get("css") or ...`), so they fall
/// through like an absent key rather than erroring on an empty selector.
pub fn apply_selectors(
    doc: &dom::Doc,
    ctx: dom::ElementRef<'_>,
    specs: &[Value],
) -> Result<Value, String> {
    apply_selectors_with_adaptive(doc, ctx, specs, false, None, false)
}

fn apply_selectors_with_adaptive(
    doc: &dom::Doc,
    ctx: dom::ElementRef<'_>,
    specs: &[Value],
    adaptive_enabled: bool,
    domain: Option<&str>,
    auto_save: bool,
) -> Result<Value, String> {
    let mut out = serde_json::Map::new();
    for spec in specs {
        let name = spec
            .get("name")
            .and_then(Value::as_str)
            .ok_or("'name'")?
            .to_string();
        let want_all = spec.get("all").and_then(Value::as_bool).unwrap_or(false);
        if let Some(pattern) = spec
            .get("regex")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            // regex specs run on the CONTEXT element's text, not the query result.
            let text = dom::get_all_text(ctx, "\n", false, &["script", "style"], true);
            let re = text::compile(pattern, false)?;
            out.insert(
                name,
                if want_all {
                    json!(re.findall(&text))
                } else {
                    json!(re.find_first(&text))
                },
            );
            continue;
        }
        let css = spec
            .get("css")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let xp = spec
            .get("xpath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let results = match (css, xp, adaptive_enabled) {
            (Some(q), _, true) => adaptive::css_query(doc, Some(ctx), q, domain, &name, auto_save)?,
            (None, Some(q), true) => {
                adaptive::xpath_query(doc, Some(ctx), q, domain, &name, auto_save)?
            }
            (Some(q), _, false) => query::css_query(doc, Some(ctx), q)?,
            (None, Some(q), false) => crate::scrapling::xpath::xpath_query(doc, Some(ctx), q)?,
            (None, None, _) => {
                out.insert(name, if want_all { json!([]) } else { Value::Null });
                continue;
            }
        };
        let attr = spec
            .get("attr")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let html_flag = spec.get("html").and_then(Value::as_bool).unwrap_or(false);
        out.insert(
            name,
            if want_all {
                json!(results
                    .iter()
                    .map(|r| common::pull(r, attr, html_flag))
                    .collect::<Vec<_>>())
            } else {
                results
                    .first()
                    .map(|r| common::pull(r, attr, html_flag))
                    .unwrap_or(Value::Null)
            },
        );
    }
    Ok(Value::Object(out))
}

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let doc = dom::parse(html);
    let specs = payload
        .get("selectors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let adaptive_enabled = payload
        .get("adaptive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let auto_save = match payload.get("auto_save") {
        None => adaptive_enabled,
        Some(value) => value.as_bool().unwrap_or(false),
    };
    Ok(json!({
        "extracted": apply_selectors_with_adaptive(
            &doc,
            doc.root(),
            &specs,
            adaptive_enabled,
            payload.get("adaptive_domain").and_then(Value::as_str),
            auto_save,
        )?
    }))
}
