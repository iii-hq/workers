//! Harness-side OTel wrapper.
//!
//! Tags every harness span with `iii.session.id` / `iii.message.id` /
//! `iii.function.id` and propagates them via baggage. See
//! `docs/superpowers/specs/2026-05-12-harness-trace-correlation-design.md`.

use std::future::Future;

use iii_sdk::IIIError;
use serde_json::Value;

/// 128 chars leaves room for app-specific IDs while keeping response headers
/// under typical proxy caps and OTel attribute values within collector limits.
pub(crate) const MAX_ID_LEN: usize = 128;

pub const WILDCARD_SESSION_ID: &str = "*";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WildcardMode {
    /// `session_id: null` → `Some("*")`. For ui::subscribe / ui::unsubscribe.
    OnExplicitNull,
    Disabled,
}

#[derive(Clone, Copy, Debug)]
pub enum IdSource {
    /// `body.session_id` → top-level → `body.payload.session_id`.
    BridgeTrigger,
    QueryParams,
    /// `body.session_id` → top-level.
    BodyOrTopLevel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessIds {
    pub session_id: Option<String>,
    pub message_id: Option<String>,
    /// Always `false` after the Option-A switch; kept for source-compat.
    pub message_id_was_minted: bool,
}

/// Extract IDs from `input` according to `source`. Pure helper — no OTel
/// side-effects on its core path.
///
/// Resolution: payload → OTel baggage (set by an upstream harness wrapper).
/// Without the baggage fallback an inner wrapper would mint a fresh
/// `message_id` even when an outer wrapper already established one,
/// fragmenting the trace across "message" groups in TRACES.
///
/// Strings over `MAX_ID_LEN` or containing CR/LF/control bytes are treated
/// as missing so response headers and span attributes can't be poisoned.
#[must_use]
pub fn extract_ids(input: &Value, source: IdSource, wildcard: WildcardMode) -> HarnessIds {
    let (session_present, session_value, message_value) = match source {
        IdSource::BridgeTrigger => {
            let (s_present, s, m, _fid) = extract_bridge_trigger_ids(input);
            (s_present, s, m)
        }
        IdSource::QueryParams => extract_query_params_ids(input),
        IdSource::BodyOrTopLevel => extract_body_or_top_level_ids(input),
    };

    // Wildcard takes precedence over baggage for ui::subscribe / ui::unsubscribe.
    let session_id =
        if wildcard == WildcardMode::OnExplicitNull && session_present && session_value.is_none() {
            Some(WILDCARD_SESSION_ID.to_string())
        } else {
            session_value.or_else(|| iii_sdk::get_baggage_entry("iii.session.id"))
        };

    let message_id = message_value.or_else(|| iii_sdk::get_baggage_entry("iii.message.id"));

    HarnessIds {
        session_id,
        message_id,
        message_id_was_minted: false,
    }
}

/// Single-walk `bridge::trigger` extractor that also returns the inner
/// `function_id` (so callers don't re-walk the JSON for a span attribute).
#[must_use]
pub fn extract_ids_for_bridge_trigger(input: &Value) -> (HarnessIds, Option<String>) {
    let (_, s_value, m_value, function_id) = extract_bridge_trigger_ids(input);
    let ids = HarnessIds {
        session_id: s_value,
        message_id: m_value,
        message_id_was_minted: false,
    };
    (ids, function_id)
}

/// Merge `traceparent` and `x-iii-message-id` into the response envelope.
/// No-op when `out` lacks a `status_code` field (raw-shape callers).
#[must_use]
pub fn merge_response_headers(
    mut out: Value,
    trace_id: Option<&str>,
    span_id: Option<&str>,
    message_id: Option<&str>,
) -> Value {
    let Some(obj) = out.as_object_mut() else {
        return out;
    };

    if !obj.contains_key("status_code") {
        return out;
    }

    let headers = obj
        .entry("headers".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Some(headers_obj) = headers.as_object_mut() {
        if let (Some(tid), Some(sid)) = (trace_id, span_id) {
            let mut tp = String::with_capacity(3 + tid.len() + 1 + sid.len() + 3);
            tp.push_str("00-");
            tp.push_str(tid);
            tp.push('-');
            tp.push_str(sid);
            tp.push_str("-01");
            headers_obj.insert("traceparent".to_string(), Value::String(tp));
        }
        if let Some(mid) = message_id {
            headers_obj.insert(
                "x-iii-message-id".to_string(),
                Value::String(mid.to_string()),
            );
        }
    }

    out
}

/// Run `f` inside an OTel span named `fn_name`.
///
/// Returns its result with `traceparent` + `x-iii-message-id` merged into
/// the response envelope. Prefer [`with_envelope_span`] / [`with_raw_span`]
/// at call sites.
pub async fn with_harness_span<F, Fut>(
    fn_name: &'static str,
    input: Value,
    source: IdSource,
    wildcard: WildcardMode,
    inner_function_id: Option<&str>,
    f: F,
) -> Result<Value, IIIError>
where
    F: FnOnce(Value) -> Fut + Send,
    Fut: Future<Output = Result<Value, IIIError>> + Send,
{
    let ids = extract_ids(&input, source, wildcard);
    run_in_span(fn_name, ids, inner_function_id, move || f(input)).await
}

/// Envelope variant: closure returns `{status_code, headers, body}`.
/// Behaviorally identical to [`with_harness_span`]; the name documents intent.
pub async fn with_envelope_span<F, Fut>(
    fn_name: &'static str,
    input: Value,
    source: IdSource,
    wildcard: WildcardMode,
    inner_function_id: Option<&str>,
    f: F,
) -> Result<Value, IIIError>
where
    F: FnOnce(Value) -> Fut + Send,
    Fut: Future<Output = Result<Value, IIIError>> + Send,
{
    with_harness_span(fn_name, input, source, wildcard, inner_function_id, f).await
}

/// Raw variant: closure returns its payload directly. Header injection is
/// skipped (no `status_code`), but the OTel span still carries `iii.*` attrs.
pub async fn with_raw_span<F, Fut>(
    fn_name: &'static str,
    input: Value,
    source: IdSource,
    wildcard: WildcardMode,
    inner_function_id: Option<&str>,
    f: F,
) -> Result<Value, IIIError>
where
    F: FnOnce(Value) -> Fut + Send,
    Fut: Future<Output = Result<Value, IIIError>> + Send,
{
    with_harness_span(fn_name, input, source, wildcard, inner_function_id, f).await
}

/// Precedence: outer (`body.{session_id,message_id}`) wins as a unit; only
/// when BOTH are absent do we read `body.payload.*`. Mixing outer+inner is
/// ambiguous so we reject the composition intentionally (tested).
fn extract_bridge_trigger_ids(
    input: &Value,
) -> (bool, Option<String>, Option<String>, Option<String>) {
    // HTTP-trigger shape: { body: { session_id?, message_id?, function_id, payload: { session_id?, message_id? } } }
    // Direct-bus shape:   { session_id?, message_id?, function_id, payload }
    let body = input.get("body").unwrap_or(input);

    let function_id = match body.get("function_id") {
        Some(Value::String(s))
            if !s.is_empty() && s.len() <= MAX_ID_LEN && s.bytes().all(is_safe_id_char) =>
        {
            Some(s.clone())
        }
        Some(Value::String(s)) => {
            // Present-but-rejected. Distinguish reason for the operator.
            let reason = if s.is_empty() {
                "empty"
            } else if s.len() > MAX_ID_LEN {
                "over_max_id_len"
            } else {
                "control_bytes"
            };
            tracing::warn!(
                len = s.len(),
                reason,
                max_id_len = MAX_ID_LEN,
                "bridge::trigger function_id rejected; treating as absent (handler will surface 'missing function_id' error). \
                 Raw bytes not logged (may carry CRLF/control content).",
            );
            None
        }
        // Absent OR wrong-type (number, bool, array, object). Don't log;
        // "field missing entirely" is the common, expected case from a
        // client that hasn't filled the payload yet.
        _ => None,
    };

    let (s_present_outer, s_outer) = get_string_field(body, "session_id");
    let (_, m_outer) = get_string_field(body, "message_id");

    if s_present_outer || m_outer.is_some() {
        return (s_present_outer, s_outer, m_outer, function_id);
    }

    // Fall back to the nested payload object. Note: this branch is only
    // reached when BOTH outer fields are absent — so the returned
    // `session_present` here is exclusively determined by the inner read.
    let payload = body.get("payload").unwrap_or(&Value::Null);
    let (s_present_inner, s_inner) = get_string_field(payload, "session_id");
    let (_, m_inner) = get_string_field(payload, "message_id");
    debug_assert!(
        !s_present_outer,
        "fall-through path requires outer session absent"
    );
    (s_present_inner, s_inner, m_inner, function_id)
}

fn extract_query_params_ids(input: &Value) -> (bool, Option<String>, Option<String>) {
    let q = input.get("query_params").unwrap_or(&Value::Null);
    let (s_present, s) = get_string_field(q, "session_id");
    let (_, m) = get_string_field(q, "message_id");
    (s_present, s, m)
}

fn extract_body_or_top_level_ids(input: &Value) -> (bool, Option<String>, Option<String>) {
    let body = input.get("body").unwrap_or(input);
    let (s_present, s) = get_string_field(body, "session_id");
    let (_, m) = get_string_field(body, "message_id");
    (s_present, s, m)
}

/// Printable ASCII only. Rejects CR/LF/control bytes so caller-supplied IDs
/// can't inject HTTP response headers or poison log pipelines.
fn is_safe_id_char(b: u8) -> bool {
    (0x20..=0x7E).contains(&b)
}

/// `(present, value)`: present=true iff the key is JSON `null` OR a valid
/// 1..=`MAX_ID_LEN` printable-ASCII string. Empty / wrong-type / over-long /
/// control-byte strings collapse to `(false, None)` so the wildcard path is
/// consistent (only triggered by explicit `null`, never by `""`).
fn get_string_field(v: &Value, key: &str) -> (bool, Option<String>) {
    match v.get(key) {
        Some(Value::Null) => (true, None),
        Some(Value::String(s))
            if !s.is_empty() && s.len() <= MAX_ID_LEN && s.bytes().all(is_safe_id_char) =>
        {
            (true, Some(s.clone()))
        }
        _ => (false, None),
    }
}

/// Like [`with_harness_span`], but accepts pre-extracted [`HarnessIds`].
///
/// Use this when the caller has already produced IDs via
/// [`extract_ids_for_bridge_trigger`] (single-walk extraction that also
/// surfaces `function_id`). Avoids re-walking the JSON tree.
pub async fn run_in_span_with_ids<F, Fut>(
    fn_name: &'static str,
    ids: HarnessIds,
    inner_function_id: Option<&str>,
    input: Value,
    f: F,
) -> Result<Value, IIIError>
where
    F: FnOnce(Value) -> Fut + Send,
    Fut: Future<Output = Result<Value, IIIError>> + Send,
{
    run_in_span(fn_name, ids, inner_function_id, move || f(input)).await
}

/// Compute the `Status::error` message (if any) for a finished handler.
///
/// `Ok` envelope with `status_code >= 400` → `Some("http <code>")`.
/// `Err(e)` → `Some(e.to_string())`.
/// Otherwise → `None`.
///
/// Pure helper extracted so the contract (H4: explicit error tagging on Err
/// branch) can be unit-tested without an InMemorySpanExporter / live tracer.
#[must_use]
pub fn error_status_message(result: &Result<Value, IIIError>) -> Option<String> {
    match result {
        Ok(value) => value
            .get("status_code")
            .and_then(Value::as_u64)
            .filter(|&code| code >= 400)
            .map(|code| format!("http {code}")),
        Err(e) => Some(e.to_string()),
    }
}

/// Lock-step with `iii_sdk::DEFAULT_ALLOWLIST`; cross-crate test enforces parity.
pub const HARNESS_KEYS: &[&str] = &["iii.session.id", "iii.message.id", "iii.function.id"];

async fn run_in_span<F, Fut>(
    fn_name: &'static str,
    ids: HarnessIds,
    inner_function_id: Option<&str>,
    f: F,
) -> Result<Value, IIIError>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Value, IIIError>> + Send,
{
    let span_result =
        iii_sdk::with_span(fn_name, None, Some(iii_sdk::SpanKind::Server), || async {
            let recording = iii_sdk::current_span_is_recording();
            // Self-observability: rising `recording=false` rate signals the
            // observability worker has gone silent.
            tracing::trace!(
                fn_name,
                recording,
                session_id = ids.session_id.as_deref(),
                message_id_minted = ids.message_id_was_minted,
                "harness span entry",
            );
            if recording {
                if let Some(sid) = &ids.session_id {
                    iii_sdk::set_current_span_attribute("iii.session.id", sid.clone());
                }
                if let Some(mid) = &ids.message_id {
                    iii_sdk::set_current_span_attribute("iii.message.id", mid.clone());
                }
                if let Some(fid) = inner_function_id {
                    iii_sdk::set_current_span_attribute("iii.function.id", fid.to_string());
                }
            }

            let trace_id = iii_sdk::current_trace_id();
            let span_id = iii_sdk::current_span_id();

            // `fid_owned` lifetime extends `inner_function_id: &str` through
            // the `run_with_baggage` await point.
            let mut baggage_entries: Vec<(&str, &str)> = Vec::with_capacity(3);
            if let Some(sid) = ids.session_id.as_deref() {
                baggage_entries.push(("iii.session.id", sid));
            }
            if let Some(mid) = ids.message_id.as_deref() {
                baggage_entries.push(("iii.message.id", mid));
            }
            let fid_owned: String;
            if let Some(fid) = inner_function_id {
                fid_owned = fid.to_string();
                baggage_entries.push(("iii.function.id", fid_owned.as_str()));
            }

            let inner_result: Result<Value, IIIError> =
                match iii_sdk::run_with_baggage(&baggage_entries, f()).await {
                    Ok(value) => Ok(merge_response_headers(
                        value,
                        trace_id.as_deref(),
                        span_id.as_deref(),
                        ids.message_id.as_deref(),
                    )),
                    Err(e) => Err(e),
                };
            if let Some(msg) = error_status_message(&inner_result) {
                // Belt-and-suspenders: tag errors explicitly via the iii-sdk
                // helper. Don't rely on `iii_sdk::with_span` to auto-tag
                // failed closures; if a future iii-sdk version stops doing
                // that, errored spans would silently land as Status::Ok.
                iii_sdk::set_current_span_error(msg);
            }
            match inner_result {
                Ok(v) => Ok::<Value, Box<dyn std::error::Error + Send + Sync>>(v),
                Err(e) => Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
            }
        })
        .await;

    // Downcast always succeeds today (we only box IIIError above). The
    // fall-through guards against a future iii-sdk wrapping errors before
    // propagating, surfacing the regression via warn! instead of silent
    // type loss.
    span_result.map_err(|boxed| match boxed.downcast::<IIIError>() {
        Ok(e) => *e,
        Err(other) => {
            tracing::warn!(
                error = %other,
                "iii-sdk error downcast fallthrough; type fidelity lost — \
                 inspect iii-sdk error-propagation behavior",
            );
            IIIError::Handler(other.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bridge_trigger_reads_top_level_ids() {
        let input = json!({
            "body": {
                "function_id": "harness::status",
                "session_id": "S1",
                "message_id": "M1",
                "payload": {}
            }
        });
        let ids = extract_ids(&input, IdSource::BridgeTrigger, WildcardMode::Disabled);
        assert_eq!(ids.session_id.as_deref(), Some("S1"));
        assert_eq!(ids.message_id.as_deref(), Some("M1"));
        assert!(!ids.message_id_was_minted);
    }

    #[test]
    fn bridge_trigger_falls_back_to_nested_payload() {
        let input = json!({
            "body": {
                "function_id": "f",
                "payload": { "session_id": "S2", "message_id": "M2" }
            }
        });
        let ids = extract_ids(&input, IdSource::BridgeTrigger, WildcardMode::Disabled);
        assert_eq!(ids.session_id.as_deref(), Some("S2"));
        assert_eq!(ids.message_id.as_deref(), Some("M2"));
    }

    #[test]
    fn bridge_trigger_direct_bus_shape_no_body_wrap() {
        let input = json!({
            "function_id": "f",
            "session_id": "S3",
            "message_id": "M3",
            "payload": {}
        });
        let ids = extract_ids(&input, IdSource::BridgeTrigger, WildcardMode::Disabled);
        assert_eq!(ids.session_id.as_deref(), Some("S3"));
        assert_eq!(ids.message_id.as_deref(), Some("M3"));
    }

    #[test]
    fn missing_message_id_yields_none() {
        // Option A: when the caller doesn't supply a message_id and baggage
        // is empty, message_id stays None. Non-chat plumbing then never
        // shows up in `Group by message`.
        let input = json!({ "body": { "function_id": "f", "payload": {} } });
        let ids = extract_ids(&input, IdSource::BridgeTrigger, WildcardMode::Disabled);
        assert_eq!(ids.message_id, None);
        assert!(!ids.message_id_was_minted);
        assert_eq!(ids.session_id, None);
    }

    #[test]
    fn query_params_source_reads_from_query_params_only() {
        let input = json!({
            "body": { "session_id": "wrong", "message_id": "wrong" },
            "query_params": { "session_id": "S4", "message_id": "M4" }
        });
        let ids = extract_ids(&input, IdSource::QueryParams, WildcardMode::Disabled);
        assert_eq!(ids.session_id.as_deref(), Some("S4"));
        assert_eq!(ids.message_id.as_deref(), Some("M4"));
    }

    #[test]
    fn nullable_session_becomes_wildcard_when_flag_set() {
        let input = json!({ "body": { "session_id": null } });
        let with_flag = extract_ids(
            &input,
            IdSource::BodyOrTopLevel,
            WildcardMode::OnExplicitNull,
        );
        assert_eq!(with_flag.session_id.as_deref(), Some("*"));

        let without_flag = extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
        assert_eq!(without_flag.session_id, None);
    }

    #[test]
    fn wrong_type_session_does_not_trigger_wildcard() {
        // Regression: a caller that sends `session_id: 42` (number) used to be
        // silently promoted to "subscribe to all sessions" by the wildcard
        // path, because get_string_field collapsed "wrong type" into the same
        // (true, None) state as "explicit null". A buggy direct-WS client
        // would have fanned out cross-session events without an error.
        for wrong in [json!(42), json!(true), json!(["S1"]), json!({"id": "S1"})] {
            let input = json!({ "body": { "session_id": wrong.clone() } });
            let ids = extract_ids(
                &input,
                IdSource::BodyOrTopLevel,
                WildcardMode::OnExplicitNull,
            );
            assert_eq!(
                ids.session_id, None,
                "wrong-type session_id ({wrong}) must not become wildcard"
            );
        }
    }

    #[test]
    fn over_long_message_id_is_treated_as_missing() {
        // Regression: caller-supplied message_id is echoed verbatim into the
        // `x-iii-message-id` response header. A 10 MB string would either get
        // rejected by HTTP intermediaries (breaking the response) or balloon
        // log/trace storage. Cap at MAX_ID_LEN (128). Under Option A, oversized
        // input is dropped to None instead of being replaced with a minted UUID.
        let huge = "x".repeat(10_000);
        let input = json!({ "body": { "message_id": &huge } });
        let ids = extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
        assert_eq!(ids.message_id, None, "over-long message_id must be dropped",);
    }

    #[test]
    fn empty_string_message_id_is_treated_as_missing() {
        // P2-7 regression. Previously `"message_id": ""` passed
        // `bytes().all(is_safe_id_char)` vacuously (empty iterator → true)
        // and was accepted as a present-and-valid empty ID. Treat empty
        // as absent → None.
        let input = json!({ "body": { "message_id": "" } });
        let ids = extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
        assert_eq!(ids.message_id, None);
    }

    #[test]
    fn empty_string_session_id_does_not_trigger_wildcard() {
        // P2-7: `"session_id": ""` previously satisfied the byte-filter
        // vacuously and would have been treated as a present empty value.
        // Under `WildcardMode::OnExplicitNull` it had a subtle asymmetry
        // with `null` (only `null` triggered wildcard, "" didn't because
        // it fed Some(s)) — but the inconsistency leaked into
        // session_present and could trip future logic. Now empty is
        // unambiguously absent.
        let input = json!({ "body": { "session_id": "" } });
        let with_wildcard = extract_ids(
            &input,
            IdSource::BodyOrTopLevel,
            WildcardMode::OnExplicitNull,
        );
        assert_eq!(with_wildcard.session_id, None);

        let without_wildcard =
            extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
        assert_eq!(without_wildcard.session_id, None);
    }

    #[test]
    fn empty_string_function_id_is_treated_as_absent_for_bridge_trigger() {
        // P2-7 + P2-8: an empty function_id should be rejected (so the
        // handler downstream returns the standard "missing function_id"
        // error rather than dispatching to "") AND it should not be
        // logged as the same kind of warning as a content-poisoned
        // function_id — but it IS logged with reason="empty" so a noisy
        // client emits enough signal for an operator to act on.
        let input = json!({ "body": { "function_id": "" } });
        let (_, function_id) = extract_ids_for_bridge_trigger(&input);
        assert!(
            function_id.is_none(),
            "empty function_id must be treated as absent"
        );
    }

    #[test]
    fn over_long_session_id_does_not_trigger_wildcard() {
        let huge = "x".repeat(10_000);
        let input = json!({ "body": { "session_id": huge } });
        let ids = extract_ids(
            &input,
            IdSource::BodyOrTopLevel,
            WildcardMode::OnExplicitNull,
        );
        assert_eq!(
            ids.session_id, None,
            "over-long session_id must not become wildcard"
        );
    }

    #[test]
    fn message_id_at_boundary_is_accepted() {
        // 128-char message_id is exactly at the limit and must be accepted.
        let at_limit = "x".repeat(128);
        let input = json!({ "body": { "message_id": &at_limit } });
        let ids = extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
        assert_eq!(ids.message_id.as_deref(), Some(at_limit.as_str()));
    }

    #[test]
    fn crlf_in_message_id_is_rejected() {
        // Regression: a caller sending `"message_id": "abc\r\nSet-Cookie: x=y"`
        // would otherwise echo the raw bytes into the response's
        // `x-iii-message-id` header. Any upstream serializer using naive
        // `format!("{}: {}\r\n", k, v)` would then HTTP-response-split.
        // Under Option A, control-byte input is dropped to None.
        for poison in ["abc\r\nSet-Cookie: x=y", "a\nb", "a\tb", "a\0b", "\x1b[31m"] {
            let input = json!({ "body": { "message_id": poison } });
            let ids = extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
            assert_eq!(
                ids.message_id, None,
                "message_id containing control byte ({poison:?}) must be dropped",
            );
        }
    }

    #[test]
    fn crlf_in_session_id_does_not_trigger_wildcard_or_echo() {
        // Same regression for session_id: control-byte content rejected so
        // it can't land in OTel span attributes (log-pipeline poisoning) or,
        // for wildcard-mode handlers, be misinterpreted as "explicit null".
        for poison in ["S\rsmuggle", "S\nsmuggle", "S\x01", "\x1b]2;evil\x07"] {
            let input = json!({ "body": { "session_id": poison } });
            let with_wildcard = extract_ids(
                &input,
                IdSource::BodyOrTopLevel,
                WildcardMode::OnExplicitNull,
            );
            assert_eq!(
                with_wildcard.session_id, None,
                "control-byte session_id ({poison:?}) must not become wildcard or be echoed"
            );
            let without_wildcard =
                extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
            assert_eq!(without_wildcard.session_id, None);
        }
    }

    #[test]
    fn extract_bridge_trigger_outer_message_inner_session_documented_precedence() {
        // Documents the intentional precedence rule: when outer body has ANY
        // id field (here `message_id`), the inner `payload` is NOT consulted.
        // The inner `session_id` is therefore dropped. This is by design — a
        // mixed-shape payload is ambiguous, so outer wins as a unit.
        let input = json!({
            "body": {
                "function_id": "f",
                "message_id": "M-outer",
                "payload": { "session_id": "S-inner" }
            }
        });
        let ids = extract_ids(&input, IdSource::BridgeTrigger, WildcardMode::Disabled);
        assert_eq!(ids.message_id.as_deref(), Some("M-outer"));
        assert_eq!(
            ids.session_id, None,
            "inner session_id must be dropped when outer has any id field"
        );
    }

    #[test]
    fn bridge_trigger_extractor_returns_function_id_in_single_walk() {
        // Verifies the L6 single-walk optimization: function_id is surfaced
        // alongside the IDs without a second tree walk.
        let input = json!({
            "body": {
                "function_id": "harness::status",
                "session_id": "S1",
                "message_id": "M1",
                "payload": {}
            }
        });
        let (ids, function_id) = extract_ids_for_bridge_trigger(&input);
        assert_eq!(ids.session_id.as_deref(), Some("S1"));
        assert_eq!(ids.message_id.as_deref(), Some("M1"));
        assert_eq!(function_id.as_deref(), Some("harness::status"));
    }

    #[test]
    fn bridge_trigger_extractor_rejects_oversize_function_id() {
        // M1: function_id is caller-controlled. Apply the same MAX_ID_LEN cap
        // so a 10 KB function_id can't bloat OTel attribute storage or log
        // indexers downstream.
        let huge = "f".repeat(10_000);
        let input = json!({ "body": { "function_id": &huge, "payload": {} } });
        let (_ids, function_id) = extract_ids_for_bridge_trigger(&input);
        assert!(
            function_id.is_none(),
            "over-long function_id must be treated as absent"
        );
    }

    #[test]
    fn bridge_trigger_extractor_rejects_control_chars_in_function_id() {
        let input = json!({ "body": { "function_id": "harness\r\nX-Injected: 1", "payload": {} } });
        let (_ids, function_id) = extract_ids_for_bridge_trigger(&input);
        assert!(function_id.is_none());
    }

    #[test]
    fn wildcard_session_id_constant_matches_extractor_output() {
        // M8: a single source of truth for the wildcard token. If the
        // constant ever changes, this test catches the drift.
        let input = json!({ "body": { "session_id": null } });
        let ids = extract_ids(
            &input,
            IdSource::BodyOrTopLevel,
            WildcardMode::OnExplicitNull,
        );
        assert_eq!(ids.session_id.as_deref(), Some(WILDCARD_SESSION_ID));
        assert_eq!(WILDCARD_SESSION_ID, "*");
    }

    #[test]
    fn extract_ids_is_safe_under_concurrent_calls() {
        // M7: `extract_ids` is a pure function over its `&Value` input; this
        // test asserts no hidden global state by hammering it from multiple
        // threads with distinct session IDs and verifying each call's output
        // matches its own input. (Span-attribute isolation in `run_in_span`
        // depends on the iii-sdk task-local OTel context, which can't be
        // unit-tested here — see the integration tests for that path.)
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        let errors = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let errors = Arc::clone(&errors);
                thread::spawn(move || {
                    let session = format!("S-thread-{i}");
                    let message = format!("M-thread-{i}");
                    let input = json!({
                        "body": { "session_id": &session, "message_id": &message }
                    });
                    let ids = extract_ids(&input, IdSource::BodyOrTopLevel, WildcardMode::Disabled);
                    if ids.session_id.as_deref() != Some(session.as_str())
                        || ids.message_id.as_deref() != Some(message.as_str())
                    {
                        errors.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread");
        }
        assert_eq!(
            errors.load(Ordering::SeqCst),
            0,
            "concurrent extract_ids calls must not cross-contaminate"
        );
    }

    #[test]
    fn error_status_message_tags_handler_err() {
        // H4 contract: the Err branch must produce a Status::error message.
        let err: Result<Value, IIIError> = Err(IIIError::Handler("missing function_id".into()));
        let msg = error_status_message(&err).expect("Err yields a status message");
        assert!(msg.contains("missing function_id"), "got: {msg}");
    }

    #[test]
    fn error_status_message_tags_http_error_envelope() {
        // status_code >= 400 in an Ok envelope is also tagged as Status::error.
        let env = Ok(json!({ "status_code": 503, "headers": {}, "body": "down" }));
        assert_eq!(error_status_message(&env).as_deref(), Some("http 503"));
        let env_404 = Ok(json!({ "status_code": 404 }));
        assert_eq!(error_status_message(&env_404).as_deref(), Some("http 404"));
    }

    #[test]
    fn error_status_message_none_for_ok_envelope_and_raw_ok() {
        // 200 OK envelope: no error tag.
        let env_ok = Ok(json!({ "status_code": 200, "body": {} }));
        assert_eq!(error_status_message(&env_ok), None);
        // Raw (non-envelope) Ok: no status_code, no error tag.
        let raw_ok = Ok(json!({ "ok": true, "total_browsers": 1 }));
        assert_eq!(error_status_message(&raw_ok), None);
    }

    #[test]
    fn error_status_message_ignores_redirects_and_informational() {
        // 3xx / 1xx / 2xx are not Status::error.
        for code in [100u64, 200, 204, 301, 302, 399] {
            let env = Ok(json!({ "status_code": code }));
            assert_eq!(error_status_message(&env), None, "code {code}");
        }
    }

    #[test]
    fn error_status_message_handles_400_boundary() {
        let env = Ok(json!({ "status_code": 400 }));
        assert_eq!(error_status_message(&env).as_deref(), Some("http 400"));
    }

    #[test]
    fn missing_session_key_does_not_become_wildcard() {
        let input = json!({ "body": { "message_id": "M" } });
        let ids = extract_ids(
            &input,
            IdSource::BodyOrTopLevel,
            WildcardMode::OnExplicitNull,
        );
        assert_eq!(ids.session_id, None);
    }

    // The four `augment_baggage_*` tests that previously lived here pinned
    // the behavior of the in-process baggage-augment helper. That helper
    // moved into iii-sdk as `iii_sdk::run_with_baggage`; the equivalent
    // tests live at
    // `motia/sdk/packages/rust/iii/tests/span_ops_api.rs::run_with_baggage_*`.
    // Harness-side coverage is now the lockstep allowlist test in
    // `harness/tests/trace_correlation.rs` plus the e2e attribute-filter test.

    #[test]
    fn merge_adds_traceparent_and_message_id_to_existing_headers() {
        let envelope = json!({
            "status_code": 200,
            "headers": { "content-type": "application/json" },
            "body": { "ok": true }
        });
        let merged = merge_response_headers(
            envelope,
            Some("0af7651916cd43dd8448eb211c80319c"),
            Some("b7ad6b7169203331"),
            Some("M-test"),
        );
        let headers = merged.get("headers").expect("headers field exists");
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(
            headers["traceparent"],
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        );
        assert_eq!(headers["x-iii-message-id"], "M-test");
        // body is untouched
        assert_eq!(merged["body"], json!({"ok": true}));
    }

    #[test]
    fn merge_creates_headers_when_absent() {
        let envelope = json!({ "status_code": 200, "body": {} });
        let trace = "a".repeat(32);
        let span = "b".repeat(16);
        let merged = merge_response_headers(
            envelope,
            Some(trace.as_str()),
            Some(span.as_str()),
            Some("M"),
        );
        assert!(merged["headers"].is_object());
        assert!(merged["headers"]["traceparent"].is_string());
        assert_eq!(merged["headers"]["x-iii-message-id"], "M");
    }

    #[test]
    fn merge_skips_traceparent_when_trace_id_is_none() {
        let envelope = json!({ "status_code": 200, "headers": {} });
        let merged = merge_response_headers(envelope, None, None, Some("M"));
        assert!(merged["headers"].get("traceparent").is_none());
        assert_eq!(merged["headers"]["x-iii-message-id"], "M");
    }

    #[test]
    fn merge_skips_message_id_header_when_none() {
        // Option A: no caller-supplied message_id and no upstream baggage
        // → no `x-iii-message-id` header. Header is for chat correlation,
        // not for plumbing calls.
        let envelope = json!({ "status_code": 200, "headers": {} });
        let merged = merge_response_headers(envelope, None, None, None);
        assert!(merged["headers"].get("x-iii-message-id").is_none());
    }

    #[test]
    fn merge_leaves_non_object_returns_untouched() {
        let raw = json!("just a string");
        let merged = merge_response_headers(raw.clone(), Some("x"), Some("y"), Some("M"));
        assert_eq!(merged, raw);
    }

    #[test]
    fn merge_leaves_envelope_lacking_status_code_untouched() {
        // fs::read_inline returns this shape directly (no status_code).
        // The wrapper should NOT graft `headers` onto it.
        let raw = json!({
            "content": [{ "type": "text", "text": "hello" }],
            "details": { "size": 5, "truncated": false, "bytes_read": 5 },
            "terminate": false
        });
        let merged = merge_response_headers(
            raw.clone(),
            Some("a".repeat(32).as_str()),
            Some("b".repeat(16).as_str()),
            Some("M"),
        );
        assert_eq!(merged, raw, "non-envelope objects must be left untouched");
    }
}
