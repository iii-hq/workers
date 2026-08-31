//! The shared fetch response envelope, mirroring `scrapling/src/core.py`'s
//! `serialize_page` (core.py:185-208).
//!
//! Every outbound function — `fetch`, `dynamic-fetch`, `stealthy-fetch`,
//! `session-fetch` and each page of `crawl` — returns this shape, so the
//! post-fetch half (inline extraction, markdown/text rendering, raw HTML) is
//! written once here and reuses the parse modules the worker already has.

use serde_json::{json, Map, Value};

use crate::scrapling::{dom, markdown, ops::extract::apply_selectors};

pub const SAFE_BODY_BUDGET: usize = 65_536;
const TRUNCATION_MARKER: &str = "[truncated; narrow with css_selector/selectors or fetch one URL]";

/// What a fetch tier produces, independent of how it fetched.
///
/// `cookies` is a name -> value map for HTTP. Browser tiers preserve the
/// frozen worker's observable serialization quirk and return `{}`.
#[derive(Debug, Default, Clone)]
pub struct PageData {
    pub status: Option<u16>,
    pub url: String,
    pub headers: Map<String, Value>,
    pub cookies: Map<String, Value>,
    pub encoding: Option<String>,
    /// Outer HTML of the fetched document.
    pub html: String,
    pub captured_xhr: Vec<Value>,
}

/// Build the response envelope for one fetched page.
///
/// Key order follows `serialize_page`'s insertion order, which is NOT the
/// property order its JSON Schema declares: the base five, then `extracted`,
/// then `captured_xhr`, then `format` *before* `content`, then `html`. With
/// `serde_json/preserve_order` on, that order is what goes on the wire.
pub fn serialize_page(
    page: &PageData,
    payload: &Value,
    include_html: bool,
    body_budget: Option<usize>,
) -> Result<Value, String> {
    let mut out = Map::new();
    out.insert(
        "status".into(),
        page.status.map(|s| json!(s)).unwrap_or(Value::Null),
    );
    out.insert("url".into(), json!(page.url));
    out.insert("headers".into(), Value::Object(page.headers.clone()));
    out.insert("cookies".into(), Value::Object(page.cookies.clone()));
    out.insert(
        "encoding".into(),
        page.encoding
            .as_ref()
            .map(|e| json!(e))
            .unwrap_or(Value::Null),
    );

    // Parse at most once, and only when something actually needs the tree.
    let selectors = payload
        .get("selectors")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty());
    let fmt = payload
        .get("format")
        .and_then(Value::as_str)
        .filter(|f| matches!(*f, "markdown" | "text"));
    let doc = if selectors.is_some() || fmt.is_some() || include_html {
        Some(dom::parse(&page.html))
    } else {
        None
    };
    let safe_doc = body_budget.and(doc.as_ref()).map(|doc| {
        let main_content_only = payload
            .get("main_content_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let explicit_selector = payload
            .get("css_selector")
            .and_then(Value::as_str)
            .is_some_and(|selector| !selector.is_empty());
        let mut cleaned = doc.clone();
        let inline_attributes: Vec<_> = cleaned
            .tree
            .descendants(cleaned.tree.root())
            .filter(|id| cleaned.tree.is_element(*id))
            .flat_map(|id| {
                cleaned
                    .tree
                    .attributes(id)
                    .iter()
                    .filter_map(move |attribute| {
                        let value = attribute.value.trim_start().to_ascii_lowercase();
                        let inline = value.starts_with("data:")
                            || value.starts_with("blob:")
                            || (attribute.name.eq_ignore_ascii_case("srcset")
                                && value.split(',').any(|candidate| {
                                    let candidate = candidate.trim_start();
                                    candidate.starts_with("data:") || candidate.starts_with("blob:")
                                }));
                        inline.then(|| (id, attribute.name.clone()))
                    })
            })
            .collect();
        for (id, name) in inline_attributes {
            cleaned.tree.remove_attribute(id, &name);
        }
        if main_content_only && !explicit_selector {
            let scope = cleaned
                .first_by_tag("main")
                .or_else(|| cleaned.first_by_tag("article"))
                .or_else(|| cleaned.first_by_tag("body"))
                .unwrap_or_else(|| cleaned.root());
            let scope_id = scope.id();
            markdown::sanitize_main_content(&mut cleaned, scope_id);
            let drop_ids: Vec<_> = cleaned
                .tree
                .descendants(scope_id)
                .filter(|id| {
                    matches!(
                        cleaned.tree.node_name(*id),
                        Some("nav" | "aside" | "footer")
                    )
                })
                .collect();
            for id in drop_ids {
                cleaned.tree.remove_node(id);
            }
            dom::parse(&dom::outer_html(
                cleaned.element(scope_id).expect("scope survives cleanup"),
            ))
        } else {
            if main_content_only {
                let root_id = cleaned.root().id();
                markdown::sanitize_main_content(&mut cleaned, root_id);
            }
            cleaned
        }
    });
    let doc = safe_doc.as_ref().or(doc.as_ref());

    if let Some(specs) = selectors {
        let doc = doc.expect("parsed when selectors present");
        out.insert("extracted".into(), apply_selectors(doc, doc.root(), specs)?);
    }
    if !page.captured_xhr.is_empty() {
        return Err("Object of type Response is not JSON serializable".to_string());
    }
    if let Some(fmt) = fmt {
        let doc = doc.expect("parsed when format present");
        let main_content_only = body_budget.is_none()
            && payload
                .get("main_content_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let css_selector = payload.get("css_selector").and_then(Value::as_str);
        out.insert("format".into(), json!(fmt));
        out.insert(
            "content".into(),
            json!(markdown::render(doc, fmt, css_selector, main_content_only)?),
        );
    }
    if include_html {
        let doc = doc.expect("parsed when HTML is included");
        out.insert("html".into(), json!(dom::outer_html(doc.root())));
    }
    if let Some(budget) = body_budget {
        budget_page_derived_fields(&mut out, budget);
    }
    Ok(Value::Object(out))
}

pub(crate) fn budget_page_derived_fields(out: &mut Map<String, Value>, budget: usize) {
    let mut remaining = budget;
    for key in ["extracted", "content", "html"] {
        if let Some(value) = out.get_mut(key) {
            budget_value(value, &mut remaining);
        }
    }
}

fn budget_value(value: &mut Value, remaining: &mut usize) {
    match value {
        Value::String(text) if text.len() > *remaining => {
            let prefix_budget = remaining.saturating_sub(TRUNCATION_MARKER.len());
            let mut end = prefix_budget.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if *remaining >= TRUNCATION_MARKER.len() {
                text.truncate(end);
                text.push_str(TRUNCATION_MARKER);
            } else {
                let mut end = (*remaining).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            *remaining = 0;
        }
        Value::String(text) => *remaining -= text.len(),
        Value::Array(values) => {
            for value in values {
                budget_value(value, remaining);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                budget_value(value, remaining);
            }
        }
        _ => {}
    }
}

/// `include_html` after the caller-specific default precedence has already
/// been applied. Crawl deliberately passes only its request value.
pub fn include_html(payload: &Value) -> bool {
    payload
        .get("include_html")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The bulk `urls` form: one entry per URL, in request order, where a failed
/// URL contributes `{url, error}` instead of sinking the whole batch
/// (handlers.py:26-42).
pub fn bulk_results(results: Vec<Result<Value, (String, String)>>) -> Value {
    let items: Vec<Value> = results
        .into_iter()
        .map(|r| match r {
            Ok(v) => v,
            Err((url, error)) => json!({"url": url, "error": error}),
        })
        .collect();
    json!({ "results": items })
}

/// Read the request's target(s): either a single `url` or a bulk `urls` list.
/// Mirrors handlers.py:20-22, including its error text.
pub fn targets(payload: &Value) -> Result<(Vec<String>, bool), String> {
    if let Some(list) = payload.get("urls").and_then(Value::as_array) {
        let urls: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if !urls.is_empty() {
            return Ok((urls, true));
        }
    }
    match payload.get("url").and_then(Value::as_str) {
        Some(u) if !u.is_empty() => Ok((vec![u.to_string()], false)),
        _ => Err("provide `url` or `urls`".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> PageData {
        PageData {
            status: Some(200),
            url: "https://example.com/".into(),
            headers: [("content-type".to_string(), json!("text/html"))]
                .into_iter()
                .collect(),
            cookies: [("sid".to_string(), json!("abc"))].into_iter().collect(),
            encoding: Some("utf-8".into()),
            html: "<html><head></head><body><h1>Hi</h1><a href=\"/x\">go</a></body></html>".into(),
            captured_xhr: vec![],
        }
    }

    #[test]
    fn base_envelope_has_exactly_the_five_keys_in_order() {
        let out = serialize_page(&page(), &json!({}), false, None).unwrap();
        let keys: Vec<&str> = out
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, ["status", "url", "headers", "cookies", "encoding"]);
    }

    #[test]
    fn selectors_add_extracted() {
        let payload = json!({"selectors": [{"name": "title", "css": "h1"}]});
        let out = serialize_page(&page(), &payload, false, None).unwrap();
        assert_eq!(out["extracted"]["title"], json!("Hi"));
    }

    #[test]
    fn safe_main_content_prefers_first_main_and_removes_page_chrome() {
        let page = PageData {
            html: concat!(
                "<html><body><nav>Nav</nav><main><header><h1>Title</h1></header>",
                "<p>Main</p><aside>Aside</aside><footer>Footer</footer></main></body></html>"
            )
            .into(),
            ..page()
        };
        let out = serialize_page(
            &page,
            &json!({"format": "text", "main_content_only": true}),
            false,
            Some(65_536),
        )
        .unwrap();

        assert_eq!(out["content"], "Title\nMain");
    }

    #[test]
    fn safe_html_strips_inline_data_and_blob_uri_attributes() {
        let page = PageData {
            html: concat!(
                r#"<img src="data:image/png;base64,AAAA" alt="keep">"#,
                r#"<video poster="blob:https://example.com/id"></video>"#,
                r#"<a href="https://example.com/ok">ok</a>"#,
            )
            .into(),
            ..page()
        };
        let out = serialize_page(&page, &json!({}), true, Some(65_536)).unwrap();
        let html = out["html"].as_str().unwrap();

        assert!(!html.contains("data:"), "{html}");
        assert!(!html.contains("blob:"), "{html}");
        assert!(html.contains("alt=\"keep\""), "{html}");
        assert!(html.contains("href=\"https://example.com/ok\""), "{html}");
    }

    #[test]
    fn safe_html_strips_srcset_when_any_candidate_has_an_inline_uri() {
        let page = PageData {
            html: concat!(
                r#"<img srcset="https://example.com/a.png 1x, DaTa:image/png;base64,AAAA 2x" alt="data">"#,
                r#"<img srcset="https://example.com/b.png 1x, BLOB:https://example.com/id 2x" alt="blob">"#,
            )
            .into(),
            ..page()
        };
        let out = serialize_page(&page, &json!({}), true, Some(65_536)).unwrap();
        let html = out["html"].as_str().unwrap();

        assert!(!html.contains("srcset="), "{html}");
        assert!(html.contains("alt=\"data\""), "{html}");
        assert!(html.contains("alt=\"blob\""), "{html}");
    }

    #[test]
    fn safe_explicit_selector_takes_precedence_over_main_content_scope() {
        let page = PageData {
            html: "<html><head><title>Document title</title></head><body><main>Body</main></body></html>"
                .into(),
            ..page()
        };
        let out = serialize_page(
            &page,
            &json!({
                "format": "text",
                "main_content_only": true,
                "css_selector": "title",
            }),
            false,
            Some(65_536),
        )
        .unwrap();

        assert_eq!(out["content"], "Document title");
    }

    #[test]
    fn safe_budget_prioritizes_extracted_and_truncates_utf8_with_guidance() {
        let page = PageData {
            html: format!("<h1>ok</h1><p>{}</p>", "é".repeat(100)),
            ..page()
        };
        let out = serialize_page(
            &page,
            &json!({
                "selectors": [{"name": "title", "css": "h1"}],
                "format": "text",
            }),
            true,
            Some(80),
        )
        .unwrap();
        let marker = "[truncated; narrow with css_selector/selectors or fetch one URL]";

        assert_eq!(out["extracted"]["title"], "ok");
        assert_eq!(out["content"], format!("ok\n{}{}", "é".repeat(5), marker));
        assert_eq!(out["html"], "");
        let used = out["extracted"]["title"].as_str().unwrap().len()
            + out["content"].as_str().unwrap().len()
            + out["html"].as_str().unwrap().len();
        assert!(used <= 80, "page-derived body used {used} bytes");
    }

    #[test]
    fn safe_budget_handles_zero_and_sub_marker_limits() {
        for budget in [0, TRUNCATION_MARKER.len() - 1] {
            let out = serialize_page(
                &page(),
                &json!({
                    "selectors": [{"name": "title", "css": "h1"}],
                    "format": "text",
                }),
                true,
                Some(budget),
            )
            .unwrap();
            let used = out["extracted"]["title"].as_str().unwrap().len()
                + out["content"].as_str().unwrap().len()
                + out["html"].as_str().unwrap().len();

            assert!(used <= budget, "budget {budget} used {used} bytes: {out}");
        }
    }

    #[test]
    fn empty_selector_list_adds_nothing() {
        let out = serialize_page(&page(), &json!({"selectors": []}), false, None).unwrap();
        assert!(out.get("extracted").is_none());
    }

    #[test]
    fn captured_browser_responses_preserve_the_public_serialization_error() {
        let mut page = page();
        page.captured_xhr.push(json!({"status": 200}));
        assert_eq!(
            serialize_page(&page, &json!({}), false, None).unwrap_err(),
            "Object of type Response is not JSON serializable"
        );
    }

    #[test]
    fn format_emits_format_before_content() {
        let out = serialize_page(&page(), &json!({"format": "text"}), false, None).unwrap();
        let keys: Vec<&str> = out
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        let f = keys.iter().position(|k| *k == "format").unwrap();
        let c = keys.iter().position(|k| *k == "content").unwrap();
        assert!(
            f < c,
            "serialize_page inserts format before content: {keys:?}"
        );
        // Verified against the reference implementation: `op_to_markdown` in
        // text mode yields "Hi\ngo" for this HTML (the h1 is a block, so it
        // ends the line).
        assert_eq!(out["content"], json!("Hi\ngo"));
    }

    #[test]
    fn unsupported_format_is_ignored_not_an_error() {
        // core.py only renders for markdown|text; anything else falls through.
        let out = serialize_page(&page(), &json!({"format": "html"}), false, None).unwrap();
        assert!(out.get("content").is_none());
        assert!(out.get("format").is_none());
    }

    #[test]
    fn include_html_appends_libxml_serialized_html_last() {
        let mut page = page();
        page.html = "<p>Hi</p>".into();
        let out = serialize_page(&page, &json!({}), true, None).unwrap();
        let keys: Vec<&str> = out
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(*keys.last().unwrap(), "html");
        assert_eq!(out["html"], json!("<html><body><p>Hi</p></body></html>"));
    }

    #[test]
    fn null_status_and_encoding_serialize_as_null() {
        let mut p = page();
        p.status = None;
        p.encoding = None;
        let out = serialize_page(&p, &json!({}), false, None).unwrap();
        assert_eq!(out["status"], Value::Null);
        assert_eq!(out["encoding"], Value::Null);
    }

    #[test]
    fn targets_prefers_urls_then_url_then_errors() {
        assert_eq!(
            targets(&json!({"urls": ["a", "b"]})).unwrap(),
            (vec!["a".to_string(), "b".to_string()], true)
        );
        assert_eq!(
            targets(&json!({"url": "a"})).unwrap(),
            (vec!["a".to_string()], false)
        );
        // an empty `urls` falls back to `url` rather than fetching nothing
        assert_eq!(
            targets(&json!({"urls": [], "url": "a"})).unwrap(),
            (vec!["a".to_string()], false)
        );
        assert_eq!(targets(&json!({})).unwrap_err(), "provide `url` or `urls`");
    }

    #[test]
    fn bulk_results_keeps_order_and_isolates_failures() {
        let out = bulk_results(vec![
            Ok(json!({"url": "a", "status": 200})),
            Err(("b".into(), "boom".into())),
        ]);
        assert_eq!(out["results"][0]["status"], json!(200));
        assert_eq!(out["results"][1], json!({"url": "b", "error": "boom"}));
    }
}
