//! The harness's `judge::evaluate` client, used by call reconciliation
//! (MOT-4847). It routes to the session's provider (the turn's
//! `iii.judge.provider` baggage), pauses a failing provider for
//! [`PAUSE_MS`] like the directory does, and fails open: on any error the
//! caller carries on as if no judge were deployed.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::types::message::AgentMessage;

/// After a judge failure, skip that provider for this long.
const PAUSE_MS: i64 = 30_000;
/// Longest string kept verbatim in the judge's view of a value.
const MAX_STRING_CHARS: usize = 512;
/// Argument keys whose values never leave for the judge (approval-gate's
/// list): exact, or as a `_<key>` suffix, case-insensitive.
const SECRET_KEYS: [&str; 11] = [
    "password",
    "token",
    "api_key",
    "apikey",
    "secret",
    "auth",
    "authorization",
    "access_key",
    "access_token",
    "refresh_token",
    "private_key",
];

/// Epoch ms until which each provider is skipped after a failure, keyed by
/// provider (`""` = the hub's default). Per provider, so one session's
/// failing judge never pauses another session's.
static PAUSED_UNTIL: Mutex<BTreeMap<String, i64>> = Mutex::new(BTreeMap::new());

/// The current turn's judge provider (`judge_contract::PROVIDER_BAGGAGE_KEY`
/// baggage stamped by the turn step), when set and well-formed.
pub(crate) fn current_provider() -> Option<String> {
    use iii_helpers::observability::opentelemetry::baggage::BaggageExt as _;
    let context = iii_helpers::observability::opentelemetry::Context::current();
    let provider = context
        .baggage()
        .get(judge_contract::PROVIDER_BAGGAGE_KEY)?
        .to_string();
    judge_contract::is_valid_provider(&provider).then_some(provider)
}

fn paused(provider: &str, now: i64) -> bool {
    PAUSED_UNTIL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(provider)
        .is_some_and(|until| now < *until)
}

fn pause(provider: &str, until: i64) {
    PAUSED_UNTIL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(provider.to_string(), until);
}

/// Whether a failure `code` pauses the provider. A rejected or oversized
/// request is about this request, not the provider: another caller's
/// request may still fit.
fn pauses(code: &str) -> bool {
    !matches!(code, "invalid_request" | "payload_too_large")
}

pub(crate) fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SECRET_KEYS
        .iter()
        .any(|secret| lower == *secret || lower.ends_with(&format!("_{secret}")))
}

/// A value as the judge sees it: secret-keyed values masked, long strings
/// (file contents, page text) cut to a preview, so an evaluation stays small
/// and never carries credentials to a hosted judge.
pub fn bounded(value: &Value) -> Value {
    match value {
        Value::String(text) if text.chars().nth(MAX_STRING_CHARS).is_some() => json!(format!(
            "{}…",
            crate::trigger::truncate_chars(text, MAX_STRING_CHARS)
        )),
        Value::Array(values) => Value::Array(values.iter().map(bounded).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let shown = if is_secret_key(key) && !value.is_null() {
                        json!("<redacted>")
                    } else {
                        bounded(value)
                    };
                    (key.clone(), shown)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Whether a judge call could run now: `judge::evaluate` is deployed and the
/// session's provider is not paused. Lets a caller skip building questions.
pub async fn available(deps: &Deps) -> bool {
    let key = current_provider().unwrap_or_default();
    !paused(&key, AgentMessage::now_ms())
        && deps
            .functions()
            .await
            .functions
            .iter()
            .any(|f| f.function_id == judge_contract::FUNCTION_ID)
}

/// Ask the judge one evaluation and return its answers, or why there are
/// none: `unavailable` (not deployed or paused), the provider's error code,
/// `timeout`, or the transport error. The session's provider travels in the
/// request itself, so the hub routes it even if some hop dropped the turn's
/// context.
pub async fn evaluate(deps: &Deps, evaluation: Value, timeout_ms: u64) -> Result<Value, String> {
    if !available(deps).await {
        return Err("unavailable".into());
    }
    let provider = current_provider();
    let key = provider.as_deref().unwrap_or_default();
    let id = evaluation["id"].as_str().unwrap_or_default().to_string();
    let wait = timeout_ms + 1_000;
    let mut payload = json!({ "timeout_ms": timeout_ms, "evaluations": [evaluation] });
    if let Some(provider) = provider.as_deref() {
        payload["provider"] = json!(provider);
    }
    let call = deps.iii.trigger(TriggerRequest {
        function_id: judge_contract::FUNCTION_ID.into(),
        payload,
        action: None,
        timeout_ms: Some(wait),
    });
    match tokio::time::timeout(Duration::from_millis(wait), call).await {
        Ok(Ok(reply)) if reply["status"] == "ok" => {
            Ok(reply["results"][id.as_str()]["answers"].clone())
        }
        failure => {
            let reason = match failure {
                Ok(Ok(reply)) => reply["code"].as_str().unwrap_or("error").to_string(),
                Ok(Err(error)) => error.to_string(),
                Err(_) => "timeout".to_string(),
            };
            if pauses(&reason) {
                pause(key, AgentMessage::now_ms() + PAUSE_MS);
            }
            Err(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_provider_pauses_only_itself() {
        pause("pause-test-a", 2_000);
        assert!(paused("pause-test-a", 1_000));
        assert!(!paused("pause-test-a", 2_000));
        assert!(!paused("pause-test-b", 1_000));
    }

    #[test]
    fn only_provider_failures_pause() {
        assert!(pauses("deadline"));
        assert!(pauses("timeout"));
        assert!(pauses("provider_unavailable"));
        assert!(!pauses("invalid_request"));
        assert!(!pauses("payload_too_large"));
    }

    #[test]
    fn the_turn_provider_comes_from_baggage_and_must_be_well_formed() {
        use iii_helpers::observability::opentelemetry::baggage::BaggageExt as _;
        use iii_helpers::observability::opentelemetry::{Context, KeyValue};
        let with = |value: &'static str| {
            Context::current_with_baggage(vec![KeyValue::new(
                judge_contract::PROVIDER_BAGGAGE_KEY,
                value,
            )])
        };
        {
            let _guard = with("semif").attach();
            assert_eq!(current_provider().as_deref(), Some("semif"));
        }
        {
            let _guard = with("Not A Provider").attach();
            assert_eq!(current_provider(), None);
        }
    }

    #[test]
    fn bounded_masks_secret_keys_and_cuts_long_strings() {
        let long = "z".repeat(MAX_STRING_CHARS + 10);
        let out = bounded(&json!({
            "query": "rust",
            "API_KEY": "sk-1",
            "db_password": "hunter2",
            "headers": { "Authorization": "Bearer x", "accept": "json" },
            "token": null,
            "tokens": 5,
            "body": long,
        }));
        assert_eq!(out["query"], "rust");
        assert_eq!(out["API_KEY"], "<redacted>");
        assert_eq!(out["db_password"], "<redacted>");
        assert_eq!(out["headers"]["Authorization"], "<redacted>");
        assert_eq!(out["headers"]["accept"], "json");
        assert_eq!(out["token"], Value::Null);
        assert_eq!(out["tokens"], 5);
        assert_eq!(
            out["body"].as_str().unwrap().chars().count(),
            MAX_STRING_CHARS + 1
        );
    }
}
