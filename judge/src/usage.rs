//! What each `judge::evaluate` cost, by calling worker: the `judge.calls`
//! and `judge.tokens` counters and the `judge.duration_ms` histogram (read
//! with `engine::metrics::list`), plus the same facts on the call's span, so
//! a turn's trace shows what its judge calls took.
//!
//! The engine stamps the caller as a connection id; the hub names it from
//! `engine::workers::list`, once per connection.
use iii_sdk::{errors::Error, protocol::TriggerRequest, IIIClient};
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::trace::TraceContextExt;
use opentelemetry::{global, KeyValue};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const METER_NAME: &str = "judge";
const WORKERS_LIST_TIMEOUT_MS: u64 = 2_000;
/// The caller of an in-process or unnamed invocation.
const UNKNOWN: &str = "unknown";

static CALLS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_counter("judge.calls")
        .with_description("judge::evaluate calls by caller, provider, model and outcome")
        .build()
});
static TOKENS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .u64_counter("judge.tokens")
        .with_description("Tokens the provider reported, by caller, provider, model and direction")
        .build()
});
static DURATION_MS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    global::meter(METER_NAME)
        .f64_histogram("judge.duration_ms")
        .with_description(
            "Whole judge::evaluate latency through the hub, by caller, provider and outcome",
        )
        .with_unit("ms")
        .build()
});
/// Worker names by engine connection id.
// ponytail: grows by one entry per reconnect; prune if callers churn.
static NAMES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// The calling worker's name, looked up once per connection id.
pub async fn caller_name(iii: &IIIClient, caller_id: Option<&str>) -> String {
    let Some(id) = caller_id else {
        return UNKNOWN.into();
    };
    if let Some(name) = NAMES.lock().unwrap().get(id) {
        return name.clone();
    }
    let listed = iii
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(WORKERS_LIST_TIMEOUT_MS),
        })
        .await
        .unwrap_or_default();
    let mut names = NAMES.lock().unwrap();
    for worker in listed["workers"].as_array().into_iter().flatten() {
        if let (Some(id), Some(name)) = (worker["id"].as_str(), worker["name"].as_str()) {
            names.insert(id.into(), name.into());
        }
    }
    names
        .entry(id.into())
        .or_insert_with(|| UNKNOWN.into())
        .clone()
}

/// The outcome label, model and (input, output) tokens of one forwarded
/// `judge::evaluate`: `ok`, the judge error code, `provider_unavailable`
/// or `transport`.
pub fn summarize(reply: &Result<Value, Error>) -> (String, String, u64, u64) {
    let reply = match reply {
        Ok(reply) => reply,
        Err(Error::Remote { code, .. }) if code.eq_ignore_ascii_case("function_not_found") => {
            return ("provider_unavailable".into(), String::new(), 0, 0);
        }
        Err(_) => return ("transport".into(), String::new(), 0, 0),
    };
    let text = |pointer: &str| {
        reply
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let tokens = |pointer: &str| reply.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
    let outcome = match text("/status") {
        "ok" => "ok",
        _ => text("/code"),
    };
    (
        outcome.into(),
        text("/model").into(),
        tokens("/stats/input_tokens"),
        tokens("/stats/output_tokens"),
    )
}

/// Count one forwarded `judge::evaluate` and describe it on the current span.
pub fn record(caller: &str, provider: &str, reply: &Result<Value, Error>, elapsed: Duration) {
    let (outcome, model, input, output) = summarize(reply);
    let elapsed_ms = elapsed.as_secs_f64() * 1_000.0;
    let by = |last: KeyValue| {
        [
            KeyValue::new("caller", caller.to_string()),
            KeyValue::new("provider", provider.to_string()),
            KeyValue::new("model", model.clone()),
            last,
        ]
    };
    let by_outcome = by(KeyValue::new("outcome", outcome.clone()));
    CALLS.add(1, &by_outcome);
    DURATION_MS.record(elapsed_ms, &by_outcome);
    for (direction, count) in [("input", input), ("output", output)] {
        if count > 0 {
            TOKENS.add(count, &by(KeyValue::new("direction", direction)));
        }
    }
    let context = opentelemetry::Context::current();
    let span = context.span();
    span.set_attribute(KeyValue::new("judge.caller", caller.to_string()));
    span.set_attribute(KeyValue::new("judge.provider", provider.to_string()));
    span.set_attribute(KeyValue::new("judge.model", model));
    span.set_attribute(KeyValue::new("judge.outcome", outcome));
    span.set_attribute(KeyValue::new("judge.input_tokens", input as i64));
    span.set_attribute(KeyValue::new("judge.output_tokens", output as i64));
    span.set_attribute(KeyValue::new("judge.duration_ms", elapsed_ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_summarized_by_outcome_model_and_tokens() {
        let ok = Ok(json!({"status": "ok", "model": "jev-1.14.0", "results": {},
            "stats": {"input_tokens": 677, "output_tokens": 57}}));
        assert_eq!(summarize(&ok), ("ok".into(), "jev-1.14.0".into(), 677, 57));
        let refused = Ok(json!({"status": "error", "code": "http", "stats": {"input_tokens": 0}}));
        assert_eq!(summarize(&refused), ("http".into(), String::new(), 0, 0));
        let missing = Err(Error::Remote {
            code: "function_not_found".into(),
            message: "m".into(),
            stacktrace: None,
        });
        assert_eq!(summarize(&missing).0, "provider_unavailable");
    }
}
