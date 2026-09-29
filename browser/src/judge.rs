//! Optional `judge::evaluate` client for `browser::run`. The judge worker is
//! never a dependency: the call itself is the availability probe, and any
//! outage (not registered, no provider or key, transport, deadline, a reply
//! that is not a valid answer) pauses judge calls for `PAUSE_MS` so a run
//! without one costs one quick refusal, not a timeout per call. The caller
//! falls back to the agent-driven `elements` + `act` flow.
//!
//! Each call goes to the calling session's provider (the
//! `iii.judge.provider` baggage the harness stamps per turn), and the pause
//! is per provider, so one session's failing judge never stops another's.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use judge_contract::{EvaluateRequest, Evaluation};
use serde::Deserialize;
use serde_json::Value;

/// After a failure, skip the judge for this long (the directory's policy).
pub const PAUSE_MS: i64 = 30_000;

/// Epoch ms until which each provider is skipped (`""` = the hub default).
static PAUSED_UNTIL: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

/// The calling session's judge provider from the handler's OTel baggage,
/// when set and well-formed; `None` routes to the hub's default. Read it in
/// the handler task: a spawned task without the context sees nothing.
pub fn session_provider() -> Option<String> {
    use opentelemetry::baggage::BaggageExt;
    let provider = opentelemetry::Context::current()
        .baggage()
        .get(judge_contract::PROVIDER_BAGGAGE_KEY)?
        .to_string();
    judge_contract::is_valid_provider(&provider).then_some(provider)
}

fn paused(provider: &str, now: i64) -> bool {
    PAUSED_UNTIL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|pauses| pauses.get(provider))
        .is_some_and(|until| now < *until)
}

fn pause(provider: &str, until: i64) {
    PAUSED_UNTIL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(provider.to_string(), until);
}

/// A `choice` answer, read leniently: fields a later judge adds are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub choice: String,
    pub probabilities: BTreeMap<String, f64>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JudgeError {
    /// Not deployed, no provider, transport, deadline or a bad reply; the
    /// judge is paused.
    Unavailable(String),
    /// The judge rejected the request as malformed: this worker's bug, not an
    /// outage, so no pause.
    Rejected(String),
}

impl std::fmt::Display for JudgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "judge unavailable: {reason}"),
            Self::Rejected(reason) => write!(f, "judge rejected the request: {reason}"),
        }
    }
}

fn now_ms() -> i64 {
    crate::session::now_ms()
}

/// Ask one evaluation of `provider` (`None` = the hub's default); returns
/// its answers by question id plus the round trip in ms.
pub async fn evaluate(
    iii: &IIIClient,
    evaluation: Evaluation,
    timeout_ms: u64,
    provider: Option<&str>,
) -> Result<(BTreeMap<String, Value>, u64), JudgeError> {
    let key = provider.unwrap_or_default();
    if paused(key, now_ms()) {
        return Err(JudgeError::Unavailable(
            "paused after a recent failure".into(),
        ));
    }
    let id = evaluation.id.clone();
    let request = EvaluateRequest {
        options: Default::default(),
        request_id: None,
        model: None,
        timeout_ms: timeout_ms.max(1),
        expires_at_unix_ms: None,
        evaluations: vec![evaluation],
    };
    if judge_contract::validate_request(&request).is_err() {
        return Err(JudgeError::Rejected(
            "request fails the judge contract".into(),
        ));
    }
    let mut payload = serde_json::to_value(&request)
        .map_err(|e| JudgeError::Rejected(format!("request does not serialize: {e}")))?;
    if let Some(provider) = provider {
        payload["provider"] = Value::String(provider.to_string());
    }
    let started = Instant::now();
    // The bus deadline sits past the judge's own so its `deadline` reply
    // wins over a bare bus timeout.
    let wait = timeout_ms + 1_000;
    let call = iii.trigger(TriggerRequest {
        function_id: judge_contract::FUNCTION_ID.into(),
        payload,
        action: None,
        timeout_ms: Some(wait),
    });
    let reply = match tokio::time::timeout(Duration::from_millis(wait), call).await {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err("timeout".to_string()),
    };
    let answers = classify(reply, &id);
    if let Err(JudgeError::Unavailable(reason)) = &answers {
        tracing::debug!(%reason, "judge unavailable for browser::run");
        pause(key, now_ms() + PAUSE_MS);
    }
    Ok((answers?, started.elapsed().as_millis() as u64))
}

/// Map a bus reply to the evaluation's answers or a `JudgeError`.
fn classify(
    reply: Result<Value, String>,
    evaluation_id: &str,
) -> Result<BTreeMap<String, Value>, JudgeError> {
    let reply = reply.map_err(JudgeError::Unavailable)?;
    if reply["status"] != "ok" {
        let code = reply["code"].as_str().unwrap_or("error").to_string();
        return Err(if code == "invalid_request" {
            JudgeError::Rejected(code)
        } else {
            JudgeError::Unavailable(code)
        });
    }
    reply["results"][evaluation_id]["answers"]
        .as_object()
        .map(|answers| answers.clone().into_iter().collect())
        .ok_or_else(|| JudgeError::Unavailable("reply carries no answers".into()))
}

/// A valid `choice` over exactly `keys`: known choice, a probability for
/// every key and no other, each in [0, 1], summing to 1, the choice the most
/// likely. Anything else is a bad reply and executes nothing.
pub fn choice(answer: Option<&Value>, keys: &[&str]) -> Result<Choice, JudgeError> {
    let bad = || JudgeError::Unavailable("invalid choice answer".into());
    let parsed: Choice = answer
        .cloned()
        .and_then(|a| serde_json::from_value(a).ok())
        .ok_or_else(bad)?;
    let unit = |p: f64| p.is_finite() && (0.0..=1.0).contains(&p);
    let top = parsed
        .probabilities
        .values()
        .copied()
        .fold(f64::MIN, f64::max);
    let valid = keys.contains(&parsed.choice.as_str())
        && parsed.probabilities.len() == keys.len()
        && keys.iter().all(|k| parsed.probabilities.contains_key(*k))
        && parsed.probabilities.values().all(|p| unit(*p))
        && unit(parsed.confidence)
        && (parsed.probabilities.values().sum::<f64>() - 1.0).abs() <= 0.02
        && parsed.probabilities[&parsed.choice] >= top - 1e-6;
    if valid {
        Ok(parsed)
    } else {
        Err(bad())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_failing_provider_pauses_only_itself() {
        pause("pause-test-a", 2_000);
        assert!(paused("pause-test-a", 1_000));
        assert!(!paused("pause-test-a", 2_000));
        assert!(!paused("pause-test-b", 1_000));
    }

    #[test]
    fn the_session_provider_comes_from_baggage_and_must_be_well_formed() {
        use opentelemetry::baggage::BaggageExt;
        let with = |value: &'static str| {
            opentelemetry::Context::current_with_baggage(vec![opentelemetry::KeyValue::new(
                judge_contract::PROVIDER_BAGGAGE_KEY,
                value,
            )])
        };
        assert_eq!(session_provider(), None);
        {
            let _guard = with("semif").attach();
            assert_eq!(session_provider().as_deref(), Some("semif"));
        }
        let _guard = with("Not Valid").attach();
        assert_eq!(session_provider(), None);
    }

    #[test]
    fn outages_and_rejections_are_told_apart() {
        assert_eq!(
            classify(Err("remote error (function_not_found): x".into()), "s"),
            Err(JudgeError::Unavailable(
                "remote error (function_not_found): x".into()
            ))
        );
        assert_eq!(
            classify(Ok(json!({"status": "error", "code": "missing_key"})), "s"),
            Err(JudgeError::Unavailable("missing_key".into()))
        );
        assert_eq!(
            classify(
                Ok(json!({"status": "error", "code": "invalid_request"})),
                "s"
            ),
            Err(JudgeError::Rejected("invalid_request".into()))
        );
        assert!(matches!(
            classify(Ok(json!({"status": "ok", "results": {}})), "s"),
            Err(JudgeError::Unavailable(_))
        ));
        let ok = classify(
            Ok(json!({"status": "ok", "results": {"s": {"answers": {"q": {"choice": "a"}}}}})),
            "s",
        )
        .unwrap();
        assert_eq!(ok["q"]["choice"], "a");
    }

    #[test]
    fn choice_must_be_a_valid_argmax_distribution_over_the_keys() {
        let answer = |choice: &str, a: f64, b: f64| {
            json!({"type": "choice", "choice": choice, "probabilities": {"a": a, "b": b},
                   "confidence": 0.9, "extra": true})
        };
        assert_eq!(
            choice(Some(&answer("a", 0.8, 0.2)), &["a", "b"])
                .unwrap()
                .choice,
            "a"
        );
        // not the argmax
        assert!(choice(Some(&answer("b", 0.8, 0.2)), &["a", "b"]).is_err());
        // unknown key / missing key
        assert!(choice(Some(&answer("a", 0.8, 0.2)), &["a", "c"]).is_err());
        assert!(choice(Some(&answer("a", 0.8, 0.2)), &["a", "b", "c"]).is_err());
        // not a distribution
        assert!(choice(Some(&answer("a", 0.8, 0.8)), &["a", "b"]).is_err());
        assert!(choice(None, &["a"]).is_err());
    }
}
