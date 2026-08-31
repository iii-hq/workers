//! scrapling::to-markdown — HTML to markdown/text/html, with an optional
//! main-content sanitizer and CSS scope (core.py `op_to_markdown`). Always
//! operates on the whole document: no `adaptive` field in its schema (see
//! schemas.rs `markdown_request`), so no `common::refuse_adaptive` here.

use serde_json::{json, Value};

use crate::scrapling::require_str;
use crate::scrapling::{dom, markdown};

pub fn op(payload: &Value) -> Result<Value, String> {
    let html = require_str(payload, "html")?;
    let doc = dom::parse(html);
    let format = payload
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("markdown");
    let css_selector = payload.get("css_selector").and_then(Value::as_str);
    let main_content_only = payload
        .get("main_content_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let content = markdown::render(&doc, format, css_selector, main_content_only)?;
    Ok(json!({"format": format, "content": content}))
}
