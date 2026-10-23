//! `fp::batch`: N independent calls in one request, each answered in place.
//!
//! Unlike `fp::pipe`, nothing is threaded: every call carries its own
//! payload, they run at once (or `concurrency` at a time), and the response
//! lists one entry per call, `ok` with the result or `error` with the code
//! and message. A failing call never hides the others. Results come back
//! whole, so each is capped (`max_entry_bytes`, default 16 KB) and the call
//! count past `max_calls` (default 12) only warns: the whole response still
//! has to fit the caller's delivery budget.
//!
//! Steps run with THIS worker's authority, exactly as pipe steps do, so the
//! same refusals apply (see `pipe::forbidden_step`) and the batch call is the
//! approval surface.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use iii_sdk::protocol::{TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::IIIClient;

use crate::pipe;
use crate::util;

pub const BATCH_ID: &str = "fp::batch";
pub const BATCH_DESC: &str =
    "Run several INDEPENDENT calls in one request and get every answer in place: \
     { calls: [{ id?, function, payload?, namespace?, timeout_ms? }], concurrency?, \
     stop_on_error?, timeout_ms? (default 10000 per call), max_entry_bytes? (default 16384), \
     max_calls? (default 12; more only warns) } → { ok, results: [{ id, function, status: \
     \"ok\", result, chars, truncated? } | { id, function, status: \"error\", code, message }], \
     warnings }. Nothing is threaded between calls; for that use fp::pipe.";

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_MAX_ENTRY_BYTES: usize = 16 * 1024;
const DEFAULT_MAX_CALLS: usize = 12;
/// ponytail: hard ceiling so one call cannot fan out without bound; raise
/// when a real caller needs more than this many at once.
const HARD_MAX_CALLS: usize = 64;

// NOTE: not `deny_unknown_fields` — the engine injects `_caller_worker_id`
// (see pipe.rs). `BatchCall` stays strict.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchRequest {
    /// Independent calls; results come back in this order.
    pub calls: Vec<BatchCall>,
    /// How many run at once (default: all).
    #[serde(default)]
    pub concurrency: Option<u32>,
    /// Run in order and stop after the first error (default false: every
    /// call runs and reports).
    #[serde(default)]
    pub stop_on_error: Option<bool>,
    /// Per-call budget when a call sets none (default 10000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Largest serialized result kept per entry (default 16384); a bigger
    /// one is cut to this many bytes of its JSON text and marked `truncated`.
    #[serde(default)]
    pub max_entry_bytes: Option<u64>,
    /// Calls per request before a warning (default 12).
    #[serde(default)]
    pub max_calls: Option<u32>,
    // Harness-stamped filesystem scope; forwarded onto scoped calls, never
    // model-supplied. `//` on purpose: a `///` would publish it.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchCall {
    /// Echoed on the result; defaults to the call's position (`"0"`, `"1"`, …).
    #[serde(default)]
    pub id: Option<String>,
    /// Function id to trigger.
    pub function: String,
    #[serde(default)]
    pub payload: Option<Value>,
    /// Namespace of the target function (default: this worker's; say
    /// `default` for the engine's).
    #[serde(default)]
    pub namespace: Option<String>,
    /// Budget for this call alone.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum BatchResult {
    Ok {
        id: String,
        function: String,
        result: Value,
        /// Size of the full result (chars of its string or serialized form).
        chars: usize,
        /// Set when `result` is a cut of the JSON text, not the value.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
    },
    Error {
        id: String,
        function: String,
        code: String,
        message: String,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BatchResponse {
    /// True when every call answered `ok`.
    pub ok: bool,
    pub results: Vec<BatchResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn forbidden_call(function_id: &str, approval_gate_running: bool) -> Option<String> {
    if function_id == BATCH_ID || function_id == pipe::PIPE_ID {
        return Some(
            "batches do not nest, and a pipe is not a batch call — call it directly".into(),
        );
    }
    pipe::forbidden_step(function_id, approval_gate_running).map(str::to_string)
}

fn validate_with_approval_gate(
    req: &BatchRequest,
    approval_gate_running: bool,
) -> Result<Vec<String>, String> {
    if req.calls.is_empty() || req.calls.len() > HARD_MAX_CALLS {
        return Err(format!("a batch takes 1..={HARD_MAX_CALLS} calls"));
    }
    for (i, call) in req.calls.iter().enumerate() {
        if let Some(reason) = forbidden_call(&call.function, approval_gate_running) {
            return Err(format!("call {} ({}): {reason}", i + 1, call.function));
        }
        if pipe::is_scoped_step(&call.function) && req.fs_scope.is_none() {
            return Err(format!(
                "call {} ({}): filesystem-scoped calls (shell::*/coder::*) run only under a \
                 harness-stamped filesystem scope, and this request carries none — call the \
                 function directly",
                i + 1,
                call.function
            ));
        }
    }
    let mut warnings = Vec::new();
    let max_calls = req
        .max_calls
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_MAX_CALLS);
    if req.calls.len() > max_calls {
        warnings.push(format!(
            "{} calls at once (limit {max_calls}): too many calls in one batch may make the \
             response too large to deliver — split the batch or lower max_entry_bytes",
            req.calls.len()
        ));
    }
    Ok(warnings)
}

pub fn validate(req: &BatchRequest) -> Result<Vec<String>, String> {
    validate_with_approval_gate(req, true)
}

fn error_parts(error: &iii_sdk::Error) -> (String, String) {
    match error {
        iii_sdk::Error::Remote { code, message, .. } => (code.clone(), message.clone()),
        iii_sdk::Error::Timeout => ("TIMEOUT".into(), error.to_string()),
        iii_sdk::Error::NotConnected => ("NOT_CONNECTED".into(), error.to_string()),
        iii_sdk::Error::Handler(message) => ("HANDLER".into(), message.clone()),
        other => ("ERROR".into(), other.to_string()),
    }
}

/// Keep a result whole up to `max_bytes` of its JSON text; past that, keep
/// the first `max_bytes` bytes (on a char boundary) as a string.
fn capped(value: Value, max_bytes: usize) -> (Value, usize, bool) {
    let chars = pipe::chars_of(&value);
    let text = match &value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.len() <= max_bytes {
        return (value, chars, false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (Value::String(text[..end].to_string()), chars, true)
}

async fn one(
    iii: Arc<IIIClient>,
    index: usize,
    call: BatchCall,
    args: Value,
    timeout_ms: u64,
    max_bytes: usize,
) -> BatchResult {
    let id = call.id.clone().unwrap_or_else(|| index.to_string());
    let function = call.function.clone();
    // Transforms are pure, so they run inline whatever the namespace: the
    // result is the same value, never the `{ value }` wrapper of a bus call.
    let outcome: Result<Value, (String, String)> = match util::apply(&function, &args) {
        Some(result) => result.map_err(|message| ("INVALID_ARGS".to_string(), message)),
        None => {
            let request = TriggerRequest {
                function_id: function.clone(),
                payload: args,
                action: None,
                timeout_ms: Some(timeout_ms),
            };
            let request: TriggerRequestWithMetadata = match call.namespace {
                Some(namespace) => request.namespace(namespace),
                None => request.into(),
            };
            iii.trigger(request).await.map_err(|e| error_parts(&e))
        }
    };
    match outcome {
        Ok(value) => {
            let (result, chars, truncated) = capped(value, max_bytes);
            BatchResult::Ok {
                id,
                function,
                result,
                chars,
                truncated,
            }
        }
        Err((code, message)) => BatchResult::Error {
            id,
            function,
            code,
            message,
        },
    }
}

pub async fn run(iii: Arc<IIIClient>, req: BatchRequest) -> Result<BatchResponse, String> {
    let warnings = match (
        validate_with_approval_gate(&req, false),
        validate_with_approval_gate(&req, true),
    ) {
        (Ok(w), Ok(_)) => w,
        (Err(e), Err(_)) => return Err(e),
        _ => validate_with_approval_gate(&req, pipe::approval_gate_running(&iii).await?)?,
    };
    let default_timeout = req.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let max_bytes = req
        .max_entry_bytes
        .map(|b| b as usize)
        .unwrap_or(DEFAULT_MAX_ENTRY_BYTES);
    let stop_on_error = req.stop_on_error.unwrap_or(false);
    let fs_scope = req.fs_scope.clone();
    // What a skipped entry reports, taken before the calls are consumed.
    let labels: Vec<(String, String)> = req
        .calls
        .iter()
        .enumerate()
        .map(|(i, call)| {
            let id = call.id.clone().unwrap_or_else(|| i.to_string());
            (id, call.function.clone())
        })
        .collect();

    let prepared: Vec<(usize, BatchCall, Result<Value, String>, u64)> = req
        .calls
        .into_iter()
        .enumerate()
        .map(|(i, call)| {
            let args = call.payload.clone().unwrap_or_else(|| json!({}));
            let args = pipe::bus_step_args(&call.function, args, fs_scope.as_ref());
            let timeout = call.timeout_ms.unwrap_or(default_timeout);
            (i, call, args, timeout)
        })
        .collect();

    let mut results: Vec<Option<BatchResult>> = (0..prepared.len()).map(|_| None).collect();

    if stop_on_error {
        for (i, call, args, timeout) in prepared {
            let result = match args {
                Ok(args) => one(iii.clone(), i, call, args, timeout, max_bytes).await,
                Err(message) => invalid(i, &call, message),
            };
            let failed = matches!(result, BatchResult::Error { .. });
            results[i] = Some(result);
            if failed {
                break;
            }
        }
    } else {
        let limit = req
            .concurrency
            .map(|n| n.max(1) as usize)
            .unwrap_or(prepared.len().max(1));
        let gate = Arc::new(Semaphore::new(limit));
        let mut set = tokio::task::JoinSet::new();
        for (i, call, args, timeout) in prepared {
            let iii = iii.clone();
            let gate = gate.clone();
            set.spawn(async move {
                let _permit = gate.acquire_owned().await;
                let result = match args {
                    Ok(args) => one(iii, i, call, args, timeout, max_bytes).await,
                    Err(message) => invalid(i, &call, message),
                };
                (i, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            let (i, result) = joined.map_err(|e| format!("batch task failed: {e}"))?;
            results[i] = Some(result);
        }
    }

    // A skipped entry keeps the caller's id and its function name.
    let results: Vec<BatchResult> = results
        .into_iter()
        .zip(labels)
        .map(|(r, (id, function))| {
            r.unwrap_or_else(|| BatchResult::Error {
                id,
                function,
                code: "SKIPPED".into(),
                message: "not run: an earlier call failed and stop_on_error is set".into(),
            })
        })
        .collect();
    let ok = results.iter().all(|r| matches!(r, BatchResult::Ok { .. }));
    Ok(BatchResponse {
        ok,
        results,
        warnings,
    })
}

fn invalid(index: usize, call: &BatchCall, message: String) -> BatchResult {
    BatchResult::Error {
        id: call.id.clone().unwrap_or_else(|| index.to_string()),
        function: call.function.clone(),
        code: "INVALID_ARGS".into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_takes_a_namespace_and_refuses_unknown_fields() {
        let call = json!({ "function": "state::get", "namespace": "other" });
        let call: BatchCall = serde_json::from_value(call).unwrap();
        assert_eq!(call.namespace.as_deref(), Some("other"));
        let typo = json!({ "function": "state::get", "namespce": "other" });
        assert!(serde_json::from_value::<BatchCall>(typo).is_err());
    }

    fn parse(v: Value) -> BatchRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn validates_count_nesting_and_warns_past_the_limit() {
        assert!(validate(&parse(json!({ "calls": [] }))).is_err());
        for nested in [BATCH_ID, pipe::PIPE_ID, "harness::send"] {
            assert!(
                validate(&parse(json!({ "calls": [{ "function": nested }] }))).is_err(),
                "{nested}"
            );
        }
        let twelve: Vec<Value> = (0..12)
            .map(|_| json!({ "function": "fp::size", "payload": { "value": [] } }))
            .collect();
        assert!(validate(&parse(json!({ "calls": twelve })))
            .unwrap()
            .is_empty());
        let thirteen: Vec<Value> = (0..13)
            .map(|_| json!({ "function": "fp::size", "payload": { "value": [] } }))
            .collect();
        let warnings = validate(&parse(json!({ "calls": thirteen }))).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("too large to deliver"));
        assert!(
            validate(&parse(json!({ "calls": thirteen, "max_calls": 20 })))
                .unwrap()
                .is_empty()
        );
        // Scoped calls need the harness stamp.
        assert!(validate(&parse(
            json!({ "calls": [{ "function": "coder::read-file" }] })
        ))
        .is_err());
    }

    #[test]
    fn caps_an_entry_at_max_bytes_and_keeps_the_full_size() {
        let (kept, chars, truncated) = capped(json!("abc"), 16);
        assert_eq!((kept, chars, truncated), (json!("abc"), 3, false));
        let big = "é".repeat(20); // 40 bytes, 20 chars
        let (cut, chars, truncated) = capped(Value::String(big), 7);
        assert_eq!(chars, 20);
        assert!(truncated);
        assert_eq!(cut, json!("ééé")); // 6 bytes: the 7th lands mid-char
        let (obj, _, truncated) = capped(json!({ "k": "v" }), 2);
        assert!(truncated);
        assert_eq!(obj, json!("{\""));
    }

    #[test]
    fn error_parts_keep_the_remote_code() {
        let (code, message) = error_parts(&iii_sdk::Error::Remote {
            code: "NOT_FOUND".into(),
            message: "no such thing".into(),
            stacktrace: None,
        });
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("NOT_FOUND", "no such thing")
        );
        assert_eq!(error_parts(&iii_sdk::Error::Timeout).0, "TIMEOUT");
    }

    #[tokio::test]
    async fn inline_transforms_answer_in_place_without_a_bus() {
        // Transforms never touch the bus, so a disconnected client is fine.
        let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:1"));
        let req = parse(json!({ "calls": [
            { "id": "a", "function": "fp::size", "payload": { "value": [1, 2, 3] } },
            { "function": "fp::get", "payload": { "value": { "x": 1 }, "path": "/nope" } },
        ]}));
        let response = run(iii, req).await.unwrap();
        assert!(!response.ok);
        assert_eq!(
            response.results[0],
            BatchResult::Ok {
                id: "a".into(),
                function: "fp::size".into(),
                result: json!(3),
                chars: 1,
                truncated: false
            }
        );
        match &response.results[1] {
            BatchResult::Error { id, code, .. } => {
                assert_eq!(id, "1");
                assert_eq!(code, "INVALID_ARGS");
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }
}
