//! iii engine client: function registry reads (`engine::functions::list` /
//! `engine::functions::info`) for runtime discovery post-filtering and native
//! exposure, and generic function dispatch (`iii.trigger`) for the target of
//! an `agent_trigger` call.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

/// The message a dispatch interrupted by an engine restart resolves with —
/// it becomes the synthesized error `function_result` that closes the call
/// in the transcript (iii-hq/workers#507).
pub const ENGINE_RESTART_INTERRUPTED: &str =
    "execution interrupted by engine restart; the call ran at most once and its result is unknown";

/// A registry function descriptor (id + optional schemas/description). Only
/// the fields the harness reads are typed; the rest pass through as `extra`.
#[derive(Debug, Clone)]
pub struct FunctionDescriptor {
    pub function_id: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchError {
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{code}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

#[derive(Clone)]
pub struct EngineClient {
    iii: Arc<IIIClient>,
    timeout_ms: u64,
}

impl EngineClient {
    pub fn new(iii: Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { iii, timeout_ms }
    }

    /// Dispatch an arbitrary iii function and return its raw result. This is
    /// the target invocation of an unwrapped `agent_trigger` call.
    ///
    /// An engine restart while the call is in flight fails the dispatch with
    /// `engine_restart` instead of waiting out the full timeout: the engine's
    /// in-memory invocation routing died with it, so the result can never be
    /// delivered, and the caller must close the interrupted call promptly to
    /// keep the session usable (iii-hq/workers#507).
    pub async fn dispatch(
        &self,
        function_id: &str,
        payload: Value,
    ) -> Result<Value, DispatchError> {
        // Capture the epoch immediately before the invocation instead of
        // comparing against process-global state. A global baseline becomes
        // stale when the engine restarts while this worker is idle and would
        // falsely interrupt the first slow call made afterwards.
        let baseline = engine_epoch_ms(&self.iii).await;
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(self.timeout_ms),
        };
        let request: iii_sdk::protocol::TriggerRequestWithMetadata =
            match crate::trigger::route_namespace(&self.iii, function_id) {
                Some(namespace) => request.namespace(namespace),
                None => request.into(),
            };
        let call = self.iii.trigger(request);
        let result = match baseline {
            Some(baseline) => {
                tokio::select! {
                    result = call => result,
                    () = engine_link_interrupted(&self.iii, baseline) => {
                        return Err(DispatchError {
                            code: Some("engine_restart".to_string()),
                            message: format!("{function_id}: {ENGINE_RESTART_INTERRUPTED}"),
                        });
                    },
                }
            }
            // If the preflight epoch cannot be read, do not guess. The normal
            // SDK timeout/error is safer than declaring an invocation
            // interrupted from an untrusted baseline and re-running it.
            None => call.await,
        };
        result.map_err(|e| {
            let raw = e.to_string();
            let mut parsed = parse_dispatch_error_message(&raw);
            parsed.message = format!("{function_id}: {}", parsed.message);
            parsed
        })
    }

    /// List registry function descriptors (best-effort; empty on failure).
    pub async fn functions_list(&self) -> Vec<FunctionDescriptor> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        match resp {
            Ok(v) => parse_descriptor_list(&v),
            Err(e) => {
                tracing::warn!(error = %e, "engine::functions::list failed");
                Vec::new()
            }
        }
    }

    /// Read one function's descriptor (`None` when unknown).
    pub async fn functions_info(&self, function_id: &str) -> Option<FunctionDescriptor> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::info".into(),
                payload: json!({ "function_id": function_id }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .ok()?;
        if resp.is_null() {
            return None;
        }
        descriptor_of(&resp)
    }

    /// Read several descriptors in ONE `engine::functions::info` call
    /// (`function_ids` batch). `None` when the engine predates batch support
    /// (the caller falls back to per-id [`Self::functions_info`]); unknown ids
    /// come back as marker entries and are simply skipped by `descriptor_of`
    /// parsing (no `request_schema`/`parameters` on them is fine — hydration
    /// keeps such descriptors schema-less).
    pub async fn functions_info_batch(
        &self,
        function_ids: &[String],
    ) -> Option<Vec<FunctionDescriptor>> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::info".into(),
                payload: json!({ "function_ids": function_ids }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .ok()?;
        let items = resp.get("functions").and_then(Value::as_array)?;
        Some(items.iter().filter_map(descriptor_of).collect())
    }
}

/// How often an in-flight dispatch samples the engine epoch, and how long
/// one sample may take. Sampling is cheap relative to the dispatched calls
/// it guards; dispatches shorter than the first interval never even probe.
const ENGINE_EPOCH_PROBE_INTERVAL_MS: u64 = 1_000;
const ENGINE_EPOCH_PROBE_TIMEOUT_MS: u64 = 3_000;

/// The engine's boot identity: the earliest `connected_at_ms` among its
/// in-process (`runtime == "engine"`) workers, which attach once at engine
/// startup. A restarted engine reports a new epoch — the signal that every
/// invocation in flight across the restart lost its result routing. `None`
/// when the engine cannot be reached (outage in progress) or the response
/// shape is unrecognized.
async fn engine_epoch_ms(iii: &IIIClient) -> Option<u64> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(ENGINE_EPOCH_PROBE_TIMEOUT_MS),
        })
        .await
        .ok()?;
    parse_engine_epoch(&response)
}

fn parse_engine_epoch(response: &Value) -> Option<u64> {
    response
        .get("workers")?
        .as_array()?
        .iter()
        .filter(|worker| worker.get("runtime").and_then(Value::as_str) == Some("engine"))
        .filter_map(|worker| worker.get("connected_at_ms").and_then(Value::as_u64))
        .min()
}

/// Pend until the engine under an in-flight dispatch is observed to have
/// RESTARTED (its boot epoch changed). Neither the SDK connection state nor
/// plain liveness probes can see a fast restart: the reconnect loop reports
/// `Connected` through its silent 2s retry sleep, and outbound messages
/// buffered during the outage are answered by the NEW engine as if nothing
/// happened (both verified against a SIGKILLed-and-respawned engine). An
/// epoch read that succeeds with a changed value doubles as proof the
/// engine is answering again, so the caller can immediately persist the
/// synthesized "interrupted" result without tripping over the same outage.
async fn engine_link_interrupted(iii: &IIIClient, baseline: u64) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(
            ENGINE_EPOCH_PROBE_INTERVAL_MS,
        ))
        .await;
        // An unreadable epoch (outage in progress) never trips by itself:
        // the first successful sample afterwards decides.
        if let Some(epoch) = engine_epoch_ms(iii).await {
            if epoch != baseline {
                return;
            }
        }
    }
}

fn parse_dispatch_error_message(raw: &str) -> DispatchError {
    if let Some(parsed) = parse_dispatch_error_value(raw) {
        return parsed;
    }

    for index in raw.char_indices().map(|(index, _)| index) {
        let suffix = &raw[index..];
        if let Some(parsed) = parse_dispatch_error_value(suffix) {
            return parsed;
        }
    }

    if let Some(code) = parse_remote_error_code(raw) {
        return DispatchError {
            code: Some(code),
            message: raw.to_string(),
        };
    }

    DispatchError {
        code: None,
        message: raw.to_string(),
    }
}

fn parse_remote_error_code(raw: &str) -> Option<String> {
    const MARKER: &str = "remote error (";
    let start = raw.find(MARKER)? + MARKER.len();
    let rest = &raw[start..];
    let end = rest.find("):")?;
    let code = &rest[..end];
    if code.is_empty() {
        return None;
    }
    if !code
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/'))
    {
        return None;
    }
    Some(code.to_string())
}

fn parse_dispatch_error_value(input: &str) -> Option<DispatchError> {
    let value: Value = serde_json::from_str(input).ok()?;
    parse_dispatch_error_json(&value)
}

fn parse_dispatch_error_json(value: &Value) -> Option<DispatchError> {
    match value {
        Value::Object(map) => {
            let code = map.get("code").and_then(Value::as_str).map(str::to_string);
            let message = map
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (code, message) {
                (Some(code), Some(message)) => Some(DispatchError {
                    code: Some(code),
                    message,
                }),
                (None, Some(message)) => {
                    if let Ok(inner) = serde_json::from_str::<Value>(&message) {
                        parse_dispatch_error_json(&inner)
                    } else {
                        Some(DispatchError {
                            code: None,
                            message,
                        })
                    }
                }
                _ => None,
            }
        }
        Value::String(s) => parse_dispatch_error_value(s),
        _ => None,
    }
}

fn parse_descriptor_list(v: &Value) -> Vec<FunctionDescriptor> {
    let items: Vec<&Value> = match v {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .get("functions")
            .or_else(|| map.get("items"))
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_else(|| map.values().collect()),
        _ => return Vec::new(),
    };
    items.iter().filter_map(|d| descriptor_of(d)).collect()
}

/// Extract id + description + parameters schema from a descriptor in the
/// shapes engines return (`function_id` or `id`; parameters at the top level,
/// under `request_format`, or `request_schema`).
fn descriptor_of(v: &Value) -> Option<FunctionDescriptor> {
    let function_id = v
        .get("function_id")
        .or_else(|| v.get("id"))
        .or_else(|| v.get("name"))
        .and_then(Value::as_str)?
        .to_string();
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parameters = v
        .get("parameters")
        .or_else(|| v.get("request_format"))
        .or_else(|| v.get("request_schema"))
        .cloned();
    Some(FunctionDescriptor {
        function_id,
        description,
        parameters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_epoch_uses_the_oldest_in_process_engine_worker() {
        let response = json!({
            "workers": [
                { "runtime": "rust", "connected_at_ms": 1 },
                { "runtime": "engine", "connected_at_ms": 30 },
                { "runtime": "engine", "connected_at_ms": 20 },
                { "runtime": "engine", "connected_at_ms": "invalid" }
            ]
        });
        assert_eq!(parse_engine_epoch(&response), Some(20));
        assert_eq!(parse_engine_epoch(&json!({ "workers": [] })), None);
    }

    #[test]
    fn dispatch_error_parser_extracts_plain_json_code_and_message() {
        let err = parse_dispatch_error_message(
            "remote error: {\"code\":\"C215\",\"message\":\"outside jail\"}",
        );
        assert_eq!(err.code.as_deref(), Some("C215"));
        assert_eq!(err.message, "outside jail");
    }

    #[test]
    fn dispatch_error_parser_extracts_coder_string_error_json() {
        let err = parse_dispatch_error_message(
            "handler error: \"{\\\"code\\\":\\\"C218\\\",\\\"message\\\":\\\"outside session\\\"}\"",
        );
        assert_eq!(err.code.as_deref(), Some("C218"));
        assert_eq!(err.message, "outside session");
    }

    #[test]
    fn dispatch_error_parser_extracts_shell_remote_error_code() {
        let err = parse_dispatch_error_message(
            "remote error (S215): path escapes the fs jail roots [/private/tmp]: /Users/example filesystem_access_request={\"v\":1,\"requested_root\":\"/Users/example\",\"attempted_path\":\"/Users/example\",\"error_code\":\"S215\"}",
        );
        assert_eq!(err.code.as_deref(), Some("S215"));
        assert_eq!(
            err.message,
            "remote error (S215): path escapes the fs jail roots [/private/tmp]: /Users/example filesystem_access_request={\"v\":1,\"requested_root\":\"/Users/example\",\"attempted_path\":\"/Users/example\",\"error_code\":\"S215\"}"
        );
    }
}
