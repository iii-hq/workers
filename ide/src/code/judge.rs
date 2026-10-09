//! Optional `judge::evaluate` client for `coder::find-relevant`, adapted
//! from `browser/src/judge.rs`. The judge worker is never a dependency: the
//! call itself is the availability probe. An outage (not registered, no
//! provider or key, transport, a reply that is not a valid answer) pauses
//! judge calls for [`PAUSE_MS`] so an ask without a judge costs one quick
//! refusal, not a timeout per call. A missed deadline, an oversized request,
//! a request the judge rejects or an evaluation the hub reports failed (its
//! `invalid_response` code, e.g. a local model's failed forward) is about
//! that call, never the judge, so it pauses nothing (the directory's rule,
//! `iii-directory/src/functions/search_judge.rs`, except `invalid_response`).
//! An `ok` reply without a valid answer for every question gets past the
//! hub's own checks, so it is an outage.
//!
//! Each call goes to the calling session's provider (the `iii.judge.provider`
//! baggage the harness stamps per turn), and the pause is per provider, so
//! one session's failing judge never stops another's.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use judge_contract::{Answer, EvaluateRequest, Evaluation, RequestOptions};
use serde_json::Value;
use tokio::sync::Semaphore;

/// After an outage, skip the judge for this long (the directory's policy).
pub const PAUSE_MS: i64 = 30_000;
/// Most a model listing waits for a local model to load; half the ask's
/// time left caps it further ([`listing_budget_ms`]). A loaded or hosted
/// provider lists at once.
const MODELS_TIMEOUT_MS: u64 = 60_000;
/// Most one evaluation waits, as the listing: under any sane provider
/// `max_timeout_ms` (300 s by default), which rejects a longer call, and
/// still three times the old 20 s for a serial local judge's queue.
const CALL_TIMEOUT_MS: u64 = 60_000;
/// Most one HTTP attempt of a hosted judge (judge-typesafe) waits, so a
/// stalled connection is retried instead of holding a slot for the whole
/// call; local judges ignore it.
const ATTEMPT_TIMEOUT_MS: u64 = 20_000;

/// Default worker-wide judge calls in flight (`code.find_relevant_judge_slots`):
/// judge-typesafe's default `concurrency` is 4, and 3 leaves one for the
/// harness reconcile (2 s, pauses the judge on a deadline) and directory
/// search (3 s). Raise both together. The headroom only exists on a parallel
/// provider: a serial local one (judge-clef) runs one pass at a time, so
/// those short calls wait behind an ask's passes at any slot count.
pub const DEFAULT_SLOTS: usize = 3;
pub const MAX_SLOTS: usize = 64;
/// The worker's slot pool and its size; a new size replaces the pool and
/// each ask keeps the pool it started with.
// ponytail: asks started under the old size keep its pool while they run,
// so a resize allows old + new in flight until they end; resize in place
// if that overlap matters.
static SLOTS: Mutex<Option<(usize, Arc<Semaphore>)>> = Mutex::new(None);

/// The pool at `count` slots, for an ask's [`evaluator`].
pub fn slots(count: usize) -> Arc<Semaphore> {
    let count = count.clamp(1, MAX_SLOTS);
    let mut slots = SLOTS.lock().unwrap_or_else(|p| p.into_inner());
    match &*slots {
        Some((size, pool)) if *size == count => pool.clone(),
        _ => {
            let pool = Arc::new(Semaphore::new(count));
            *slots = Some((count, pool.clone()));
            pool
        }
    }
}

/// Epoch ms until which each provider is skipped (`""` = the hub default).
static PAUSED_UNTIL: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

pub type Scores = BTreeMap<String, f64>;
pub type EvalFuture = Pin<Box<dyn Future<Output = Result<(Scores, u64), JudgeError>> + Send>>;
/// One evaluation under the ask's deadline → its `noul` probabilities by
/// question id plus the input tokens it cost. The seam `find_relevant`
/// runs over: production is [`evaluator`], tests inject a fake.
pub type Evaluator = Arc<dyn Fn(Evaluation, Instant) -> EvalFuture + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum JudgeError {
    /// Not deployed, no provider or key, transport or an `ok` reply without
    /// a valid answer; the provider is paused.
    Unavailable(String),
    /// `deadline`, `attempt_timeout`, `cancelled` or our own wait ran out.
    Deadline,
    /// The hub's `invalid_response` code: the judge failed this one
    /// evaluation.
    Invalid,
    /// `payload_too_large`: the request was too big for this judge.
    TooLarge,
    /// `invalid_request`: this worker's bug, not an outage.
    Rejected(String),
    /// Refused locally: a recent outage paused this provider. Never sent.
    Paused,
}

impl std::fmt::Display for JudgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "judge unavailable: {reason}"),
            Self::Deadline => f.write_str("judge deadline exceeded"),
            Self::Invalid => f.write_str("judge failed the evaluation"),
            Self::TooLarge => f.write_str("judge request too large"),
            Self::Rejected(reason) => write!(f, "judge rejected the request: {reason}"),
            Self::Paused => f.write_str("judge unavailable: paused after a recent failure"),
        }
    }
}

impl JudgeError {
    /// Only an outage pauses the provider.
    pub fn pauses(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }

    /// The short reason an ask reports.
    pub fn reason(&self) -> String {
        match self {
            Self::Unavailable(reason) | Self::Rejected(reason) => reason.clone(),
            Self::Paused => PAUSED.into(),
            other => other.to_string(),
        }
    }
}

/// The `reason` of a call refused during a pause after a recent failure.
pub const PAUSED: &str = "paused";

/// The calling session's judge provider from the handler's OTel baggage,
/// when set and well-formed; `None` routes to the hub's default. Read it in
/// the handler task: a spawned task without the context sees nothing.
pub fn session_provider() -> Option<String> {
    use iii_helpers::observability::opentelemetry::baggage::BaggageExt as _;
    let context = iii_helpers::observability::opentelemetry::Context::current();
    let provider = context
        .baggage()
        .get(judge_contract::PROVIDER_BAGGAGE_KEY)?
        .to_string();
    judge_contract::is_valid_provider(&provider).then_some(provider)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn paused(provider: &str, now: i64) -> bool {
    PAUSED_UNTIL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|pauses| pauses.get(provider))
        .is_some_and(|until| now < *until)
}

/// Start a [`PAUSE_MS`] pause unless one is running: concurrent failures
/// never extend it (the directory's rule).
fn pause(provider: &str, now: i64) {
    let mut pauses = PAUSED_UNTIL.lock().unwrap_or_else(|p| p.into_inner());
    let pauses = pauses.get_or_insert_with(HashMap::new);
    if pauses.get(provider).is_none_or(|until| now >= *until) {
        pauses.insert(provider.to_string(), now + PAUSE_MS);
    }
}

/// Production [`Evaluator`] over the bus for one ask, bound to one provider
/// and that ask's slot `pool`.
pub fn evaluator(iii: IIIClient, provider: Option<String>, pool: Arc<Semaphore>) -> Evaluator {
    Arc::new(move |evaluation, deadline| {
        let iii = iii.clone();
        let provider = provider.clone();
        let pool = pool.clone();
        Box::pin(
            async move { evaluate(&iii, evaluation, deadline, provider.as_deref(), &pool).await },
        )
    })
}

/// Ask one evaluation of `provider` (`None` = the hub's default) once a
/// slot of `pool` frees up. The call gets what is left of the ask deadline
/// up to [`CALL_TIMEOUT_MS`]: a serial local judge queues it behind other
/// passes.
pub async fn evaluate(
    iii: &IIIClient,
    evaluation: Evaluation,
    deadline: Instant,
    provider: Option<&str>,
    pool: &Semaphore,
) -> Result<(Scores, u64), JudgeError> {
    let key = provider.unwrap_or_default();
    if paused(key, now_ms()) {
        return Err(JudgeError::Paused);
    }
    let _slot = tokio::time::timeout_at(deadline.into(), pool.acquire())
        .await
        .map_err(|_| JudgeError::Deadline)?
        .expect("judge slots are never closed");
    // Another call may have found the judge down while this one queued.
    if paused(key, now_ms()) {
        return Err(JudgeError::Paused);
    }
    let timeout_ms = call_timeout_ms(deadline).ok_or(JudgeError::Deadline)?;
    let id = evaluation.id.clone();
    let keys: Vec<String> = evaluation.questions.keys().cloned().collect();
    let request = evaluate_request(evaluation, timeout_ms);
    if judge_contract::validate_request(&request).is_err() {
        tracing::warn!("coder::find-relevant built a request that fails the judge contract");
        return Err(JudgeError::Rejected(
            "request fails the judge contract".into(),
        ));
    }
    let mut payload = serde_json::to_value(&request)
        .map_err(|e| JudgeError::Rejected(format!("request does not serialize: {e}")))?;
    if let Some(provider) = provider {
        payload["provider"] = Value::String(provider.to_string());
    }
    // The bus deadline sits past the judge's own so its `deadline` reply
    // wins over a bare bus timeout.
    let wait = timeout_ms + 1_000;
    let call = iii.trigger(TriggerRequest {
        function_id: judge_contract::FUNCTION_ID.into(),
        payload,
        action: None,
        timeout_ms: Some(wait),
    });
    let reply = tokio::time::timeout(Duration::from_millis(wait), call)
        .await
        .unwrap_or(Err(iii_sdk::Error::Timeout));
    let result = classify(reply, &id, &keys);
    if let Err(error) = &result {
        if error.pauses() {
            tracing::debug!(%error, "judge unavailable for coder::find-relevant");
            pause(key, now_ms());
        } else if matches!(error, JudgeError::Rejected(_)) {
            tracing::warn!(%error, "judge rejected a coder::find-relevant request");
        }
    }
    result
}

fn evaluate_request(evaluation: Evaluation, timeout_ms: u64) -> EvaluateRequest {
    EvaluateRequest {
        options: RequestOptions {
            attempt_timeout_ms: Some(ATTEMPT_TIMEOUT_MS),
        },
        request_id: None,
        model: None,
        timeout_ms,
        expires_at_unix_ms: None,
        evaluations: vec![evaluation],
    }
}

/// The ms one call may wait: those left before `deadline`, at most
/// [`CALL_TIMEOUT_MS`]; `None` once it passed.
fn call_timeout_ms(deadline: Instant) -> Option<u64> {
    let left = deadline
        .saturating_duration_since(Instant::now())
        .as_millis() as u64;
    (left > 0).then_some(left.min(CALL_TIMEOUT_MS))
}

/// Hub error codes, read as plain strings so a code added by a later judge
/// release degrades to an outage instead of an unparseable reply.
fn code_error(code: &str) -> JudgeError {
    match code {
        "deadline" | "attempt_timeout" | "cancelled" => JudgeError::Deadline,
        "invalid_response" => JudgeError::Invalid,
        "payload_too_large" => JudgeError::TooLarge,
        "invalid_request" => JudgeError::Rejected(code.into()),
        other => JudgeError::Unavailable(other.chars().take(64).collect()),
    }
}

fn bus_error(error: iii_sdk::Error) -> JudgeError {
    match error {
        iii_sdk::Error::Timeout => JudgeError::Deadline,
        iii_sdk::Error::Remote { code, .. } if code.eq_ignore_ascii_case("function_not_found") => {
            JudgeError::Unavailable("not registered".into())
        }
        _ => JudgeError::Unavailable("transport".into()),
    }
}

/// Map a bus reply to the `noul` probability of every key in `keys` plus
/// the reported input tokens. A missing, non-`noul`, non-finite or
/// out-of-range answer makes the whole reply invalid; nothing is clamped.
fn classify(
    reply: Result<Value, iii_sdk::Error>,
    evaluation_id: &str,
    keys: &[String],
) -> Result<(Scores, u64), JudgeError> {
    let reply = reply.map_err(bus_error)?;
    if reply["status"] != "ok" {
        return Err(code_error(reply["code"].as_str().unwrap_or("error")));
    }
    let invalid = || JudgeError::Unavailable("invalid_response".into());
    let answers: BTreeMap<String, Answer> =
        serde_json::from_value(reply["results"][evaluation_id]["answers"].clone())
            .map_err(|_| invalid())?;
    let mut scores = Scores::new();
    for key in keys {
        let p = answers
            .get(key)
            .and_then(Answer::as_noul)
            .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
            .ok_or_else(invalid)?;
        scores.insert(key.clone(), p);
    }
    Ok((scores, reply["stats"]["input_tokens"].as_u64().unwrap_or(0)))
}

/// What a provider's model listing tells an ask.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    /// The smallest advertised context window in tokens, if any card
    /// advertises one (typesafe cards carry none).
    pub window: Option<u64>,
    /// The listed model names; `None` when the hub is absent or the
    /// listing failed.
    pub models: Option<Vec<String>>,
}

/// The provider's model listing, with [`listing_budget_ms`] of the time left
/// before the ask's `deadline`. One that times out (a local provider still
/// loading its model, or a stalled one) makes the ask unavailable.
pub async fn window(
    iii: &IIIClient,
    provider: Option<&str>,
    deadline: Instant,
) -> Result<Listing, JudgeError> {
    let budget = listing_budget_ms(
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64,
    );
    let mut payload = serde_json::json!({ "timeout_ms": budget });
    if let Some(provider) = provider {
        payload["provider"] = Value::String(provider.to_owned());
    }
    // As in `evaluate`, the bus waits past the provider's own deadline so
    // its typed reply wins over a bare bus timeout.
    let wait = budget + 1_000;
    let call = iii.trigger(TriggerRequest {
        function_id: judge_contract::MODELS_FUNCTION_ID.into(),
        payload,
        action: None,
        timeout_ms: Some(wait),
    });
    let reply = tokio::time::timeout(Duration::from_millis(wait), call)
        .await
        .unwrap_or(Err(iii_sdk::Error::Timeout));
    window_from(reply)
}

/// A listing's budget with `remaining_ms` left of the ask:
/// [`MODELS_TIMEOUT_MS`] at most, and never over half of what is left.
fn listing_budget_ms(remaining_ms: u64) -> u64 {
    MODELS_TIMEOUT_MS.min(remaining_ms / 2)
}

fn window_from(reply: Result<Value, iii_sdk::Error>) -> Result<Listing, JudgeError> {
    match reply {
        Err(iii_sdk::Error::Timeout) => Err(JudgeError::Unavailable(LISTING_TIMEOUT.into())),
        Ok(reply) if reply["status"] == "ok" => Ok(Listing {
            window: smallest(&reply, "context_window"),
            models: reply["models"].as_array().map(|cards| {
                cards
                    .iter()
                    .filter_map(|card| Some(card.get("name")?.as_str()?.to_owned()))
                    .collect()
            }),
        }),
        // The provider ran out of its own time, as a bus timeout would.
        Ok(reply)
            if code_error(reply["code"].as_str().unwrap_or("error")) == JudgeError::Deadline =>
        {
            Err(JudgeError::Unavailable(LISTING_TIMEOUT.into()))
        }
        _ => Ok(Listing::default()),
    }
}

/// The `reason` of a listing the judge did not answer in time.
pub const LISTING_TIMEOUT: &str = "listing_timeout";

/// The smallest `field` among a model listing's cards, if any card
/// advertises one.
fn smallest(reply: &Value, field: &str) -> Option<u64> {
    reply
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|card| card.get(field)?.as_u64())
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|k| k.to_string()).collect()
    }

    fn ok(answers: Value) -> Result<Value, iii_sdk::Error> {
        Ok(
            json!({"status": "ok", "model": "m", "results": {"e": {"answers": answers}},
                  "stats": {"input_tokens": 42}}),
        )
    }

    #[test]
    fn a_failing_provider_pauses_only_itself_and_never_extends() {
        pause("find-relevant-pause-a", 0);
        assert!(paused("find-relevant-pause-a", PAUSE_MS - 1));
        assert!(!paused("find-relevant-pause-b", 1));
        // a second failure during the pause does not push it out
        pause("find-relevant-pause-a", PAUSE_MS - 1);
        assert!(!paused("find-relevant-pause-a", PAUSE_MS));
        // once over, the next failure starts a new one
        pause("find-relevant-pause-a", PAUSE_MS);
        assert!(paused("find-relevant-pause-a", 2 * PAUSE_MS - 1));
        assert!(!JudgeError::Paused.pauses());
        assert_eq!(JudgeError::Paused.reason(), PAUSED);
    }

    #[test]
    fn outages_pause_and_per_call_failures_do_not() {
        let code = |c: &str| classify(Ok(json!({"status": "error", "code": c})), "e", &[]);
        for c in [
            "provider_unavailable",
            "missing_key",
            "http",
            "transport",
            "something_new",
        ] {
            let error = code(c).unwrap_err();
            assert_eq!(error, JudgeError::Unavailable(c.into()));
            assert!(error.pauses(), "{c}");
        }
        for c in ["deadline", "attempt_timeout", "cancelled"] {
            assert_eq!(code(c).unwrap_err(), JudgeError::Deadline, "{c}");
        }
        // a local model's failed forward is about that call
        let invalid = code("invalid_response").unwrap_err();
        assert_eq!(invalid, JudgeError::Invalid);
        assert!(!invalid.pauses());
        assert_eq!(code("payload_too_large").unwrap_err(), JudgeError::TooLarge);
        let rejected = code("invalid_request").unwrap_err();
        assert_eq!(rejected, JudgeError::Rejected("invalid_request".into()));
        assert!(!rejected.pauses());
        assert!(!JudgeError::Deadline.pauses());
        assert!(!JudgeError::TooLarge.pauses());

        let bus = |e| classify(Err(e), "e", &[]).unwrap_err();
        assert_eq!(bus(iii_sdk::Error::Timeout), JudgeError::Deadline);
        assert_eq!(
            bus(iii_sdk::Error::Remote {
                code: "FUNCTION_NOT_FOUND".into(),
                message: "x".into(),
                stacktrace: None,
            }),
            JudgeError::Unavailable("not registered".into())
        );
        assert!(bus(iii_sdk::Error::NotConnected).pauses());
    }

    #[test]
    fn every_requested_key_must_be_a_unit_noul() {
        let (scores, tokens) = classify(
            ok(json!({"q000": {"type": "noul", "noul": 0.25},
                      "q001": {"type": "noul", "noul": 1.0}})),
            "e",
            &keys(&["q000", "q001"]),
        )
        .unwrap();
        assert_eq!(scores["q000"], 0.25);
        assert_eq!(scores["q001"], 1.0);
        assert_eq!(tokens, 42);
        // past the hub's checks, a malformed `ok` reply is an outage
        let invalid = JudgeError::Unavailable("invalid_response".into());
        assert!(invalid.pauses());
        // missing key
        assert_eq!(
            classify(
                ok(json!({"q000": {"type": "noul", "noul": 0.2}})),
                "e",
                &keys(&["q000", "q001"])
            )
            .unwrap_err(),
            invalid
        );
        // out of range is never clamped
        assert_eq!(
            classify(
                ok(json!({"q000": {"type": "noul", "noul": 1.2}})),
                "e",
                &keys(&["q000"])
            )
            .unwrap_err(),
            invalid
        );
        // wrong answer type
        let choice = json!({"q000": {"type": "choice", "choice": "a",
                                      "probabilities": {"a": 1.0}, "confidence": 1.0}});
        assert_eq!(
            classify(ok(choice), "e", &keys(&["q000"])).unwrap_err(),
            invalid
        );
        // no results for the evaluation
        assert_eq!(
            classify(
                Ok(json!({"status": "ok", "results": {}})),
                "e",
                &keys(&["q000"])
            )
            .unwrap_err(),
            invalid
        );
    }

    #[test]
    fn the_listing_reads_the_smallest_window_and_the_models_and_a_slow_one_is_unavailable() {
        let listing = json!({"status": "ok", "models": [
            {"name": "a", "context_window": 32768}, {"name": "b", "context_window": 8192},
            {"name": "c"}]});
        assert_eq!(smallest(&listing, "context_window"), Some(8192));
        assert_eq!(
            window_from(Ok(listing)),
            Ok(Listing {
                window: Some(8192),
                models: Some(keys(&["a", "b", "c"])),
            })
        );
        assert_eq!(
            window_from(Ok(json!({"status": "ok", "models": [{"name": "t"}]}))),
            Ok(Listing {
                window: None,
                models: Some(keys(&["t"])),
            })
        );
        // no hub, or a listing that failed: no window and no models
        assert_eq!(
            window_from(Err(iii_sdk::Error::Remote {
                code: "function_not_found".into(),
                message: String::new(),
                stacktrace: None,
            })),
            Ok(Listing::default())
        );
        assert_eq!(
            window_from(Ok(json!({"status": "error", "code": "missing_key"}))),
            Ok(Listing::default())
        );
        // a bus timeout or the provider's own deadline: loading or stalled
        let timed_out = Err(JudgeError::Unavailable("listing_timeout".into()));
        assert_eq!(window_from(Err(iii_sdk::Error::Timeout)), timed_out);
        for code in ["deadline", "attempt_timeout"] {
            assert_eq!(
                window_from(Ok(json!({"status": "error", "code": code}))),
                timed_out,
                "{code}"
            );
        }
    }

    #[test]
    fn a_listing_waits_up_to_a_minute_and_half_the_time_left() {
        assert_eq!(listing_budget_ms(240_000), 60_000);
        assert_eq!(listing_budget_ms(100_000), 50_000);
        assert_eq!(listing_budget_ms(0), 0);
    }

    #[test]
    fn a_call_waits_the_time_left_of_its_ask_up_to_a_minute() {
        assert_eq!(
            call_timeout_ms(Instant::now() + Duration::from_secs(200)),
            Some(CALL_TIMEOUT_MS)
        );
        let left = call_timeout_ms(Instant::now() + Duration::from_secs(30)).unwrap();
        assert!((29_000..=30_000).contains(&left), "{left}");
        assert_eq!(call_timeout_ms(Instant::now()), None);
    }

    #[test]
    fn each_hosted_attempt_is_bounded_apart_from_the_call() {
        let evaluation = Evaluation {
            id: "e".into(),
            state: json!({}),
            questions: BTreeMap::new(),
        };
        let request = evaluate_request(evaluation, CALL_TIMEOUT_MS);
        assert_eq!(request.options.attempt_timeout_ms, Some(ATTEMPT_TIMEOUT_MS));
        assert_eq!(request.timeout_ms, CALL_TIMEOUT_MS);
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["options"]["attempt_timeout_ms"], ATTEMPT_TIMEOUT_MS);
    }

    #[test]
    fn the_slot_pool_follows_the_configured_count() {
        let three = slots(3);
        assert!(Arc::ptr_eq(&three, &slots(3)));
        assert_eq!(three.available_permits(), 3);
        let eight = slots(8);
        assert!(!Arc::ptr_eq(&three, &eight));
        assert_eq!(eight.available_permits(), 8);
        assert_eq!(slots(0).available_permits(), 1);
        assert_eq!(slots(1000).available_permits(), MAX_SLOTS);
        slots(DEFAULT_SLOTS);
    }

    #[tokio::test]
    async fn an_evaluator_keeps_the_pool_its_ask_started_with() {
        let iii = IIIClient::new("ws://127.0.0.1:1");
        let pool = Arc::new(Semaphore::new(1));
        let _held = pool.clone().acquire_owned().await.unwrap();
        // the ask's pool, not the worker's current one (a resize replaces
        // that; other tests resize it concurrently)
        let evaluate = evaluator(iii, Some("pool-test".into()), pool);
        let evaluation = Evaluation {
            id: "e".into(),
            state: json!({}),
            questions: BTreeMap::new(),
        };
        let started = Instant::now();
        let outcome = evaluate(evaluation, started + Duration::from_millis(50)).await;
        assert_eq!(outcome, Err(JudgeError::Deadline));
        // refused at the slot wait, not after the bus's extra second
        assert!(started.elapsed() < Duration::from_millis(800));
        assert!(!paused("pool-test", now_ms()));
    }

    #[tokio::test]
    async fn a_call_waits_for_a_slot_of_its_asks_pool() {
        let iii = IIIClient::new("ws://127.0.0.1:1");
        let pool = Semaphore::new(1);
        let _held = pool.acquire().await.unwrap();
        let evaluation = Evaluation {
            id: "e".into(),
            state: json!({}),
            questions: BTreeMap::new(),
        };
        let deadline = Instant::now() + Duration::from_millis(50);
        let outcome = evaluate(&iii, evaluation, deadline, Some("slot-test"), &pool).await;
        assert_eq!(outcome, Err(JudgeError::Deadline));
        assert!(!paused("slot-test", now_ms()));
    }

    #[test]
    fn the_session_provider_comes_from_baggage_and_must_be_well_formed() {
        use iii_helpers::observability::opentelemetry::{
            baggage::BaggageExt as _, Context, KeyValue,
        };
        let with = |value: &'static str| {
            Context::current_with_baggage(vec![KeyValue::new(
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
}
