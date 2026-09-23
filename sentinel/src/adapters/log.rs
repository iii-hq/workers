//! The log adapter.
//!
//! `tracing::error!` covers what a trace never sees: the background loop that
//! never had a caller, the reconcile that failed quietly, the handler that
//! logged and carried on. What it does not carry is a session: a log record
//! has a `trace_id` and a `span_id` but none of the trace's tags, so its
//! session — and with it the check that this is not one of the worker's own
//! investigations — has to come from resolving the trace.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::adapters::{AttributeSource, ErrorEvent};
use crate::evidence::{
    filter_attributes, EvidenceBundleV1, EvidenceLogV1, EvidenceTruncatedV1, EvidenceWorkerV1,
    EVIDENCE_VERSION,
};
use crate::{ErrorSourceV1, Redactor};

/// One ERROR log as the engine's `log` trigger delivers it.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    pub timestamp_unix_nano: u64,
    pub severity_text: String,
    pub body: String,
    pub service_name: String,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub instrumentation_scope_name: Option<String>,
}

impl AttributeSource for LogRecord {
    fn attribute(&self, key: &str) -> Option<String> {
        self.attributes.get(key).cloned()
    }
}

impl LogRecord {
    /// Parse the trigger payload. Log attributes are an object of arbitrary
    /// JSON, unlike a span's array of string pairs.
    pub fn from_payload(payload: &Value) -> Option<Self> {
        Some(Self {
            timestamp_unix_nano: payload
                .get("timestamp_unix_nano")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            severity_text: payload
                .get("severity_text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            body: payload
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            service_name: payload
                .get("service_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            trace_id: non_empty(payload.get("trace_id")),
            span_id: non_empty(payload.get("span_id")),
            attributes: payload
                .get("attributes")
                .and_then(Value::as_object)
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(key, value)| {
                            (
                                key.clone(),
                                value
                                    .as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| value.to_string()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            instrumentation_scope_name: non_empty(payload.get("instrumentation_scope_name")),
        })
    }

    pub fn at_ms(&self) -> i64 {
        (self.timestamp_unix_nano / 1_000_000) as i64
    }

    pub fn is_error(&self) -> bool {
        self.severity_text.eq_ignore_ascii_case("error")
    }

    /// Where the log was written from — a log's stand-in for a function id.
    ///
    /// `function_id` comes before the scope: the engine's own ERROR logs
    /// ("Function not found", …) name the function they are about there and
    /// share one scope, so without it every one of them folded into a single
    /// group that could not say which function it meant.
    pub fn call_site(&self) -> Option<String> {
        self.attribute("code.function")
            .or_else(|| self.attribute("target"))
            .or_else(|| self.attribute("function_id"))
            .or_else(|| self.instrumentation_scope_name.clone())
    }

    /// Unique per physical record: two identical bodies a second apart are
    /// two occurrences, the same record redelivered is one.
    pub fn dedupe_key(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.body.as_bytes());
        format!(
            "log:{}:{}:{}:{}",
            self.trace_id.as_deref().unwrap_or("-"),
            self.span_id.as_deref().unwrap_or("-"),
            self.timestamp_unix_nano,
            &format!("{digest:x}")[..16]
        )
    }
}

/// Build the event for an ERROR log. The session is passed in because the log
/// itself does not carry one; `None` means the trace could not be resolved.
pub fn event_for(
    log: &LogRecord,
    session_id: Option<String>,
    redactor: &Redactor,
    redactions: &mut u64,
) -> ErrorEvent {
    let (message, hits) = redactor.redact(&log.body);
    *redactions += hits;
    ErrorEvent {
        source: ErrorSourceV1::Log,
        dedupe_key: log.dedupe_key(),
        at_ms: log.at_ms(),
        service_name: log.service_name.clone(),
        namespace: String::new(),
        function_id: log.call_site(),
        span_name: None,
        exception_type: None,
        message,
        stacktrace: None,
        trace_id: log.trace_id.clone(),
        span_id: log.span_id.clone(),
        session_id,
        turn_id: None,
        worker_version: None,
        namespace_ambiguous: false,
        evidence: None,
    }
}

/// A minimal bundle for a log that never found its span. There is no trace to
/// freeze, so what the group carries is the record itself.
pub fn bundle_for(
    log: &LogRecord,
    attribute_allowlist: &[String],
    redactor: &Redactor,
    captured_at_ms: i64,
    redactions: &mut u64,
) -> EvidenceBundleV1 {
    let mut truncated = EvidenceTruncatedV1::default();
    let (body, hits) = redactor.redact(&log.body);
    *redactions += hits;
    EvidenceBundleV1 {
        version: EVIDENCE_VERSION,
        captured_at_ms,
        settled: true,
        trace_id: log.trace_id.clone().unwrap_or_default(),
        origin_span_id: log.span_id.clone().unwrap_or_default(),
        propagated_through: Vec::new(),
        trace_tags: BTreeMap::new(),
        spans: Vec::new(),
        logs: vec![EvidenceLogV1 {
            timestamp_unix_nano: log.timestamp_unix_nano,
            severity_text: log.severity_text.clone(),
            body,
            span_id: log.span_id.clone(),
            attributes: filter_attributes(
                log.attributes.clone(),
                attribute_allowlist,
                redactor,
                &mut truncated,
                redactions,
            ),
        }],
        worker: EvidenceWorkerV1 {
            service_name: log.service_name.clone(),
            version: None,
        },
        truncated,
    }
}

fn non_empty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Whether an ERROR log is the monitor's own: written by this worker, or
/// written by anyone about one of its functions (`function_id` or the call
/// site) — the engine's "Function not found" for a closed tab's live-update
/// handler above all. See `trace::is_own_function`.
pub fn is_own(log: &LogRecord, config: &crate::WorkerConfig) -> bool {
    let own = crate::WORKER_NAME;
    config.resolve_service_alias(&log.service_name) == own
        || [log.attribute("function_id"), log.call_site()]
            .into_iter()
            .flatten()
            .any(|function| super::trace::is_own_function(&function, own))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> Value {
        json!({
            "timestamp_unix_nano": 1_789_000_000_500_000_000u64,
            "observed_timestamp_unix_nano": 1_789_000_000_500_000_000u64,
            "severity_number": 17,
            "severity_text": "ERROR",
            "body": "redelivery exhausted after 3 attempts",
            "attributes": { "code.function": "deliver", "attempt": 3 },
            "trace_id": "t1",
            "span_id": "s1",
            "resource": { "service.name": "queue" },
            "service_name": "queue",
            "instrumentation_scope_name": "queue::delivery",
        })
    }

    #[test]
    fn a_trigger_payload_parses_including_non_string_attributes() {
        let log = LogRecord::from_payload(&payload()).expect("parses");
        assert_eq!(log.service_name, "queue");
        assert!(log.is_error());
        assert_eq!(log.at_ms(), 1_789_000_000_500);
        assert_eq!(log.attribute("attempt").as_deref(), Some("3"));
    }

    #[test]
    fn the_call_site_falls_back_from_the_attribute_to_the_scope() {
        let mut log = LogRecord::from_payload(&payload()).unwrap();
        assert_eq!(log.call_site().as_deref(), Some("deliver"));

        log.attributes.remove("code.function");
        log.attributes
            .insert("target".into(), "queue::delivery::retry".into());
        assert_eq!(log.call_site().as_deref(), Some("queue::delivery::retry"));

        log.attributes.remove("target");
        assert_eq!(log.call_site().as_deref(), Some("queue::delivery"));
    }

    #[test]
    fn a_log_about_one_of_the_monitors_own_functions_is_its_own() {
        let config = crate::WorkerConfig::default();
        let mut log = LogRecord::from_payload(&payload()).unwrap();
        log.service_name = "iii".into();
        log.attributes.remove("code.function");
        log.attributes.insert(
            "function_id".into(),
            "iii::sentinel-ui::events::tab-c28f:pane:0::console-3691".into(),
        );
        assert!(is_own(&log, &config), "a closed tab's live-update handler");

        log.attributes
            .insert("function_id".into(), "sentinel::groups::list".into());
        assert!(is_own(&log, &config), "one of the worker's functions");

        log.attributes.insert(
            "function_id".into(),
            "provider::claude-code::count_tokens".into(),
        );
        assert!(
            !is_own(&log, &config),
            "somebody else's function is watched"
        );

        log.attributes.remove("function_id");
        log.service_name = "sentinel".into();
        assert!(is_own(&log, &config), "the worker's own ERROR log");
    }

    #[test]
    fn an_engine_log_about_a_function_is_grouped_by_that_function() {
        let mut log = LogRecord::from_payload(&payload()).unwrap();
        log.attributes.remove("code.function");
        log.attributes.insert(
            "function_id".into(),
            "provider::claude-code::count_tokens".into(),
        );
        assert_eq!(
            log.call_site().as_deref(),
            Some("provider::claude-code::count_tokens"),
            "the function it names, not the scope every engine log shares"
        );
    }

    #[test]
    fn the_dedupe_key_separates_records_and_joins_redeliveries() {
        let first = LogRecord::from_payload(&payload()).unwrap();
        let same_again = LogRecord::from_payload(&payload()).unwrap();
        assert_eq!(first.dedupe_key(), same_again.dedupe_key());

        let mut later = first.clone();
        later.timestamp_unix_nano += 1_000_000;
        assert_ne!(first.dedupe_key(), later.dedupe_key());

        let mut other_body = first.clone();
        other_body.body = "something else".into();
        assert_ne!(first.dedupe_key(), other_body.dedupe_key());
    }

    #[test]
    fn a_log_without_a_trace_still_has_a_key() {
        let mut bare = payload();
        bare["trace_id"] = json!(null);
        bare["span_id"] = json!("");
        let log = LogRecord::from_payload(&bare).unwrap();
        assert!(log.dedupe_key().starts_with("log:-:-:"));
    }

    #[test]
    fn the_event_carries_the_session_it_was_given_and_a_redacted_body() {
        let mut log = LogRecord::from_payload(&payload()).unwrap();
        log.body = "auth failed for Bearer abc.def-ghi".into();
        let mut redactions = 0;
        let event = event_for(
            &log,
            Some("s_7a1".into()),
            &Redactor::default(),
            &mut redactions,
        );
        assert_eq!(event.source, ErrorSourceV1::Log);
        assert_eq!(event.session_id.as_deref(), Some("s_7a1"));
        assert_eq!(event.function_id.as_deref(), Some("deliver"));
        assert!(!event.message.contains("abc.def"), "{}", event.message);
        assert_eq!(redactions, 1);
    }

    #[test]
    fn a_promoted_log_carries_itself_as_its_evidence() {
        let log = LogRecord::from_payload(&payload()).unwrap();
        let mut redactions = 0;
        let bundle = bundle_for(
            &log,
            &[],
            &Redactor::default(),
            1_789_000_000_000,
            &mut redactions,
        );
        assert!(bundle.spans.is_empty(), "there is no trace to freeze");
        assert_eq!(bundle.logs.len(), 1);
        assert!(bundle.settled, "a log record is not waiting on anything");
        assert_eq!(
            bundle.logs[0].attributes.keys().collect::<Vec<_>>(),
            vec!["code.function"],
            "`attempt` is not on the allowlist"
        );
    }
}
