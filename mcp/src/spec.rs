// MCP 2025-06-18 spec helpers shared by stdio and HTTP dispatch paths.
//
// Lives in its own module so handler.rs stays focused on transport plumbing.
// Pagination, tool annotations, completion, logging level, subscription
// tracking, and templated-URI resolution are all routed through here.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iii_sdk::III;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PAGE_SIZE: usize = 50;

// MCP logging levels follow RFC 5424 ordering (debug=lowest, emergency=highest).
// Stored as a u8 in AtomicU8. A `notifications/message` is emitted only when
// the message level is >= the configured level. Default is `info`.
pub const LOG_DEBUG: u8 = 0;
pub const LOG_INFO: u8 = 1;
pub const LOG_NOTICE: u8 = 2;
pub const LOG_WARNING: u8 = 3;
pub const LOG_ERROR: u8 = 4;
pub const LOG_CRITICAL: u8 = 5;
pub const LOG_ALERT: u8 = 6;
pub const LOG_EMERGENCY: u8 = 7;

pub fn level_from_str(s: &str) -> Option<u8> {
    match s {
        "debug" => Some(LOG_DEBUG),
        "info" => Some(LOG_INFO),
        "notice" => Some(LOG_NOTICE),
        "warning" => Some(LOG_WARNING),
        "error" => Some(LOG_ERROR),
        "critical" => Some(LOG_CRITICAL),
        "alert" => Some(LOG_ALERT),
        "emergency" => Some(LOG_EMERGENCY),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

// Pull the four MCP behavior hints + optional title from a function's
// `metadata.mcp.{title,read_only_hint,destructive_hint,idempotent_hint,open_world_hint}`.
// Returns None if metadata is missing or carries none of those keys, so the
// resulting tool object stays compact.
pub fn make_tool_annotations(metadata: &Value) -> Option<ToolAnnotations> {
    let mcp = metadata.get("mcp")?;
    let title = mcp.get("title").and_then(|v| v.as_str()).map(String::from);
    let read_only = mcp.get("read_only_hint").and_then(|v| v.as_bool());
    let destructive = mcp.get("destructive_hint").and_then(|v| v.as_bool());
    let idempotent = mcp.get("idempotent_hint").and_then(|v| v.as_bool());
    let open_world = mcp.get("open_world_hint").and_then(|v| v.as_bool());

    if title.is_none()
        && read_only.is_none()
        && destructive.is_none()
        && idempotent.is_none()
        && open_world.is_none()
    {
        return None;
    }
    Some(ToolAnnotations {
        title,
        read_only_hint: read_only,
        destructive_hint: destructive,
        idempotent_hint: idempotent,
        open_world_hint: open_world,
    })
}

// Opaque cursor format: `base64url(JSON({"offset": N}))`. Clients treat it as
// opaque per spec. Encoding choice keeps it ASCII and URL-safe even though
// MCP transports it as a JSON string.
fn encode_cursor(offset: usize) -> String {
    let payload = json!({ "offset": offset }).to_string();
    URL_SAFE_NO_PAD.encode(payload.as_bytes())
}

fn decode_cursor(cursor: &str) -> Option<usize> {
    let bytes = URL_SAFE_NO_PAD.decode(cursor.as_bytes()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("offset").and_then(|o| o.as_u64()).map(|o| o as usize)
}

// Slice `items` starting at the offset decoded from `cursor` (or 0 if absent).
// Returns the page slice and a `nextCursor` only when there are more items.
// A bad/garbage cursor decodes to offset 0 — never error out, since cursors
// are spec-opaque and clients sometimes round-trip them across reconnects.
pub fn paginate<'a, T>(
    items: &'a [T],
    cursor: Option<&str>,
    page: usize,
) -> (Vec<&'a T>, Option<String>) {
    let offset = cursor.and_then(decode_cursor).unwrap_or(0);
    if offset >= items.len() {
        return (Vec::new(), None);
    }
    let end = (offset + page).min(items.len());
    let slice: Vec<&T> = items[offset..end].iter().collect();
    let next = if end < items.len() {
        Some(encode_cursor(end))
    } else {
        None
    };
    (slice, next)
}

// completion/complete handler. Two ref types:
//   - "ref/prompt": completes named prompt arguments. Today the only argument
//     with a finite value set is `register-function.language` (node|python).
//   - "ref/tool": prefix-matches against currently exposed function ids.
//
// The spec permits `total` larger than `values.len()`, but we always return
// the full match list (capped at 100), so total == values.len() and
// hasMore == false. Keeping it simple matches what MCP Inspector expects.
pub async fn handle_completion_complete(iii: &III, params: Option<Value>) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct Argument {
        name: String,
        #[serde(default)]
        value: String,
    }
    #[derive(Deserialize)]
    struct Ref {
        #[serde(rename = "type")]
        ref_type: String,
        #[serde(default)]
        name: String,
    }
    #[derive(Deserialize)]
    struct P {
        #[serde(rename = "ref")]
        reference: Ref,
        argument: Argument,
    }

    let p: P = match params {
        Some(p) => serde_json::from_value(p).map_err(|e| format!("Invalid params: {}", e))?,
        None => return Err("Missing params".into()),
    };

    let values: Vec<String> = match p.reference.ref_type.as_str() {
        "ref/prompt" => crate::prompts::list_prompt_candidates(&p.reference.name, &p.argument.name)
            .into_iter()
            .filter(|c| c.starts_with(&p.argument.value))
            .collect(),
        "ref/tool" => {
            let prefix = p.argument.value.clone();
            match iii.list_functions().await {
                Ok(fns) => fns
                    .into_iter()
                    .map(|f| f.function_id.replace("::", "__"))
                    .filter(|name| name.starts_with(&prefix))
                    .take(100)
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
    };

    let total = values.len();
    Ok(json!({
        "completion": {
            "values": values,
            "total": total,
            "hasMore": false
        }
    }))
}

pub fn handle_logging_set_level(
    level_atom: &AtomicU8,
    params: Option<Value>,
) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct P {
        level: String,
    }
    let p: P = match params {
        Some(p) => serde_json::from_value(p).map_err(|e| format!("Invalid params: {}", e))?,
        None => return Err("Missing params".into()),
    };
    let lvl = level_from_str(&p.level).ok_or_else(|| format!("Unknown log level: {}", p.level))?;
    level_atom.store(lvl, Ordering::SeqCst);
    Ok(json!({}))
}

pub fn handle_resources_subscribe(
    subs: &std::sync::Mutex<HashSet<String>>,
    params: Option<Value>,
) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct P {
        uri: String,
    }
    let p: P = match params {
        Some(p) => serde_json::from_value(p).map_err(|e| format!("Invalid params: {}", e))?,
        None => return Err("Missing params".into()),
    };
    if let Ok(mut g) = subs.lock() {
        g.insert(p.uri);
    }
    Ok(json!({}))
}

pub fn handle_resources_unsubscribe(
    subs: &std::sync::Mutex<HashSet<String>>,
    params: Option<Value>,
) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct P {
        uri: String,
    }
    let p: P = match params {
        Some(p) => serde_json::from_value(p).map_err(|e| format!("Invalid params: {}", e))?,
        None => return Err("Missing params".into()),
    };
    if let Ok(mut g) = subs.lock() {
        g.remove(&p.uri);
    }
    Ok(json!({}))
}

pub fn make_resource_templates() -> Vec<Value> {
    vec![
        json!({
            "uriTemplate": "iii://function/{id}",
            "name": "Function",
            "description": "A specific iii function by ID",
            "mimeType": "application/json"
        }),
        json!({
            "uriTemplate": "iii://worker/{id}",
            "name": "Worker",
            "description": "A specific iii worker by ID",
            "mimeType": "application/json"
        }),
        json!({
            "uriTemplate": "iii://trigger/{id}",
            "name": "Trigger",
            "description": "A specific iii trigger by ID",
            "mimeType": "application/json"
        }),
    ]
}

// Resolve a templated URI by listing the relevant collection and matching
// the trailing id segment. Returns the JSON body to embed in the
// resources/read `contents[0].text`. Caller wraps it in the standard
// `{contents:[...]}` envelope.
pub async fn resolve_templated_uri(uri: &str, iii: &III) -> Option<(String, &'static str)> {
    if let Some(id) = uri.strip_prefix("iii://function/") {
        let fns = iii.list_functions().await.ok()?;
        let f = fns.into_iter().find(|f| f.function_id == id)?;
        let body = serde_json::to_string_pretty(&f).unwrap_or_else(|_| "{}".into());
        return Some((body, "application/json"));
    }
    if let Some(id) = uri.strip_prefix("iii://worker/") {
        let ws = iii.list_workers().await.ok()?;
        let w = ws.into_iter().find(|w| w.id == id)?;
        let body = serde_json::to_string_pretty(&w).unwrap_or_else(|_| "{}".into());
        return Some((body, "application/json"));
    }
    if let Some(id) = uri.strip_prefix("iii://trigger/") {
        let ts = iii.list_triggers(true).await.ok()?;
        let t = ts.into_iter().find(|t| t.id == id)?;
        let body = serde_json::to_string_pretty(&t).unwrap_or_else(|_| "{}".into());
        return Some((body, "application/json"));
    }
    None
}

// Helper for `notifications/resources/updated` payloads.
pub fn resource_updated_notification(uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/resources/updated",
        "params": { "uri": uri }
    })
}

// Helper for `notifications/message` payloads.
pub fn log_message_notification(level: &str, data: &Value, logger: Option<&str>) -> Value {
    let mut params = json!({ "level": level, "data": data });
    if let Some(name) = logger {
        params["logger"] = json!(name);
    }
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": params
    })
}

pub fn progress_notification(
    token: &Value,
    progress: f64,
    total: Option<f64>,
    message: Option<&str>,
) -> Value {
    let mut params = json!({ "progressToken": token, "progress": progress });
    if let Some(t) = total {
        params["total"] = json!(t);
    }
    if let Some(m) = message {
        params["message"] = json!(m);
    }
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/progress",
        "params": params
    })
}
