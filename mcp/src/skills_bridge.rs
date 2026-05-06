//! Delegates `resources/*` and `prompts/*` to the `skills` worker.
//!
//! Direct port of `src/mcp/skills-bridge.ts`.

use std::sync::Arc;

use iii_sdk::{IIIError, TriggerRequest, III};
use regex::Regex;
use serde_json::{json, Map, Value};

/// Detects the engine error variants that mean "function not registered".
/// Mirrors the TS `isMissingDelegate` predicate.
fn is_missing_delegate(err: &IIIError) -> bool {
    match err {
        IIIError::Remote { code, message, .. } => {
            let upper = code.to_uppercase();
            if upper == "NOT_FOUND" || upper == "FUNCTION_NOT_FOUND" {
                return true;
            }
            let lower = message.to_lowercase();
            lower.contains("not found")
                || lower.contains("unknown function")
                || lower.contains("no such function")
        }
        IIIError::Runtime(message) | IIIError::Handler(message) => {
            let lower = message.to_lowercase();
            lower.contains("not found")
                || lower.contains("unknown function")
                || lower.contains("no such function")
        }
        _ => false,
    }
}

/// Mirrors the TS bridge's `payload ?? {}` coercion: when an MCP request has
/// no `params` field the JSON-RPC parser materialises `Value::Null`, but the
/// `skills::*` / `prompts::*` workers expect a JSON object and short-circuit
/// to an empty list on `null`.
fn normalize_delegate_payload(payload: Value) -> Value {
    if payload.is_null() {
        json!({})
    } else {
        payload
    }
}

/// Trigger `function_id` and return the result as a JSON object. If the
/// engine reports the function as missing, swap in `empty` so MCP clients see
/// an empty list instead of an error. Other errors propagate.
async fn delegate_or_empty(
    iii: &Arc<III>,
    function_id: &str,
    payload: Value,
    empty: Value,
) -> Result<Value, IIIError> {
    // Mirror the TS bridge's `payload ?? {}` coercion: skills/prompts workers
    // expect a JSON object payload and treat `null` as missing/invalid input,
    // which would silently turn populated MCP resources/prompts lists empty.
    let payload = normalize_delegate_payload(payload);
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(result) => {
            if result.is_object() {
                Ok(result)
            } else {
                Ok(empty)
            }
        }
        Err(e) => {
            if is_missing_delegate(&e) {
                Ok(empty)
            } else {
                Err(e)
            }
        }
    }
}

/// Anchored regex used by [`is_nested_iii_resource_uri`] -- the input is a
/// full URI string, so we require the haystack to start with `iii://`.
fn nested_iii_uri_regex() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^iii://[^/]+/.+").expect("nested-iii uri regex compiles"))
}

/// Un-anchored regex used by [`strip_nested_iii_lines_from_markdown`] -- it
/// scans for nested `iii://...` URIs anywhere inside a line, even when the
/// line is a markdown bullet (e.g. `- iii://skills/emails`).
fn nested_iii_line_regex() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"iii://[^/]+/.+").expect("nested-iii line regex compiles"))
}

/// True when the resource URI has a path after the first segment (nested skill
/// or section URI root). Hides e.g. `iii://resend/emails` from
/// `resources/list` while keeping `iii://resend`, `iii://skills`.
pub fn is_nested_iii_resource_uri(uri: &str) -> bool {
    nested_iii_uri_regex().is_match(uri)
}

fn filter_resources_list_response(mut result: Value) -> Value {
    let Some(obj) = result.as_object_mut() else {
        return result;
    };
    let Some(resources) = obj.get_mut("resources").and_then(Value::as_array_mut) else {
        return result;
    };
    resources.retain(|r| {
        let Some(uri) = r.get("uri").and_then(Value::as_str) else {
            return true;
        };
        !is_nested_iii_resource_uri(uri)
    });
    result
}

/// Drop markdown lines that reference nested `iii://...` URIs (lazy listing
/// for the `iii://skills` index).
fn strip_nested_iii_lines_from_markdown(text: &str) -> String {
    let re = nested_iii_line_regex();
    text.split('\n')
        .filter(|line| !re.is_match(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn post_process_skills_index_read(mut result: Value) -> Value {
    let Some(obj) = result.as_object_mut() else {
        return result;
    };
    let Some(contents) = obj.get_mut("contents").and_then(Value::as_array_mut) else {
        return result;
    };
    for item in contents.iter_mut() {
        if let Some(item_obj) = item.as_object_mut() {
            if let Some(text) = item_obj.get("text").and_then(Value::as_str) {
                let stripped = strip_nested_iii_lines_from_markdown(text);
                item_obj.insert("text".to_string(), Value::String(stripped));
            }
        }
    }
    result
}

fn read_uri_param(params: &Value) -> Option<String> {
    params
        .get("uri")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn empty_resources() -> Value {
    let mut m = Map::new();
    m.insert("resources".to_string(), Value::Array(vec![]));
    Value::Object(m)
}

fn empty_contents() -> Value {
    let mut m = Map::new();
    m.insert("contents".to_string(), Value::Array(vec![]));
    Value::Object(m)
}

fn empty_resource_templates() -> Value {
    let mut m = Map::new();
    m.insert("resourceTemplates".to_string(), Value::Array(vec![]));
    Value::Object(m)
}

fn empty_prompts() -> Value {
    let mut m = Map::new();
    m.insert("prompts".to_string(), Value::Array(vec![]));
    Value::Object(m)
}

fn empty_messages() -> Value {
    let mut m = Map::new();
    m.insert("messages".to_string(), Value::Array(vec![]));
    Value::Object(m)
}

pub async fn mcp_resources_list(iii: &Arc<III>, params: &Value) -> Result<Value, IIIError> {
    let result = delegate_or_empty(
        iii,
        "skills::resources-list",
        params.clone(),
        empty_resources(),
    )
    .await?;
    Ok(filter_resources_list_response(result))
}

pub async fn mcp_resources_read(iii: &Arc<III>, params: &Value) -> Result<Value, IIIError> {
    let uri = read_uri_param(params);
    let result = delegate_or_empty(
        iii,
        "skills::resources-read",
        params.clone(),
        empty_contents(),
    )
    .await?;
    if uri.as_deref() == Some("iii://skills") {
        return Ok(post_process_skills_index_read(result));
    }
    Ok(result)
}

pub async fn mcp_resources_templates_list(
    iii: &Arc<III>,
    params: &Value,
) -> Result<Value, IIIError> {
    delegate_or_empty(
        iii,
        "skills::resources-templates",
        params.clone(),
        empty_resource_templates(),
    )
    .await
}

pub async fn mcp_prompts_list(iii: &Arc<III>, params: &Value) -> Result<Value, IIIError> {
    delegate_or_empty(iii, "prompts::mcp-list", params.clone(), empty_prompts()).await
}

pub async fn mcp_prompts_get(iii: &Arc<III>, params: &Value) -> Result<Value, IIIError> {
    delegate_or_empty(iii, "prompts::mcp-get", params.clone(), empty_messages()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_uri_predicate_matches_subpaths_only() {
        assert!(is_nested_iii_resource_uri("iii://resend/emails"));
        assert!(is_nested_iii_resource_uri("iii://resend/emails/send-email"));
        assert!(!is_nested_iii_resource_uri("iii://resend"));
        assert!(!is_nested_iii_resource_uri("iii://skills"));
        assert!(!is_nested_iii_resource_uri("https://example.com/foo"));
    }

    #[test]
    fn filter_resources_list_drops_nested_uris() {
        let r = json!({
            "resources": [
                {"uri": "iii://resend"},
                {"uri": "iii://resend/emails"},
                {"uri": "iii://skills"},
                {"uri": "iii://skills/contacts"},
                {"uri": "https://example.com"},
            ]
        });
        let filtered = filter_resources_list_response(r);
        let uris: Vec<&str> = filtered["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        assert_eq!(
            uris,
            vec!["iii://resend", "iii://skills", "https://example.com"]
        );
    }

    #[test]
    fn strip_nested_lines_keeps_index_root_links() {
        let text = "Index:\n- iii://skills\n- iii://skills/emails\nfooter";
        let out = strip_nested_iii_lines_from_markdown(text);
        assert!(out.contains("iii://skills"));
        assert!(!out.contains("iii://skills/emails"));
        assert!(out.contains("footer"));
    }

    #[test]
    fn post_process_skills_index_filters_inline_text() {
        let r = json!({
            "contents": [
                {"text": "iii://skills\niii://skills/emails\nbody"}
            ]
        });
        let out = post_process_skills_index_read(r);
        let txt = out["contents"][0]["text"].as_str().unwrap();
        assert!(txt.contains("iii://skills\n") || txt.starts_with("iii://skills"));
        assert!(!txt.contains("iii://skills/emails"));
        assert!(txt.contains("body"));
    }

    #[test]
    fn read_uri_param_extracts_string() {
        assert_eq!(
            read_uri_param(&json!({"uri": "iii://skills"})),
            Some("iii://skills".to_string())
        );
        assert_eq!(read_uri_param(&json!({})), None);
        assert_eq!(read_uri_param(&Value::Null), None);
    }

    #[test]
    fn is_missing_delegate_recognises_remote_codes() {
        let e = IIIError::Remote {
            code: "FUNCTION_NOT_FOUND".to_string(),
            message: String::new(),
            stacktrace: None,
        };
        assert!(is_missing_delegate(&e));
        let e = IIIError::Remote {
            code: "not_found".to_string(),
            message: String::new(),
            stacktrace: None,
        };
        assert!(is_missing_delegate(&e));
        let e = IIIError::Remote {
            code: "OTHER".to_string(),
            message: "function not found".to_string(),
            stacktrace: None,
        };
        assert!(is_missing_delegate(&e));
        let e = IIIError::Remote {
            code: "OTHER".to_string(),
            message: "Unknown function: foo".to_string(),
            stacktrace: None,
        };
        assert!(is_missing_delegate(&e));
    }

    #[test]
    fn is_missing_delegate_rejects_other_errors() {
        let e = IIIError::Remote {
            code: "INVALID_ARGUMENT".to_string(),
            message: "bad input".to_string(),
            stacktrace: None,
        };
        assert!(!is_missing_delegate(&e));
        let e = IIIError::Timeout;
        assert!(!is_missing_delegate(&e));
    }

    #[test]
    fn normalize_delegate_payload_coerces_null_to_empty_object() {
        let coerced = normalize_delegate_payload(Value::Null);
        assert!(coerced.is_object(), "null must become a JSON object");
        assert_eq!(
            coerced.as_object().map(|m| m.len()),
            Some(0),
            "coerced object must be empty"
        );
    }

    #[test]
    fn normalize_delegate_payload_passes_through_objects_and_other_values() {
        let obj = json!({"uri": "iii://skills"});
        assert_eq!(normalize_delegate_payload(obj.clone()), obj);

        let already_empty = json!({});
        assert_eq!(
            normalize_delegate_payload(already_empty.clone()),
            already_empty
        );

        // Non-null, non-object values still pass through unchanged; the
        // delegate worker is responsible for validating the shape.
        let arr = json!(["a", "b"]);
        assert_eq!(normalize_delegate_payload(arr.clone()), arr);

        let s = json!("hi");
        assert_eq!(normalize_delegate_payload(s.clone()), s);
    }
}
