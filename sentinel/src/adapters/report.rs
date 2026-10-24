//! The report adapter.
//!
//! An E2E run happens in a Docker stack that is torn down before anyone looks
//! at it, so there is no trace left to read: what survives is the report the
//! evaluation imported. Eval sends its error observations here, already in
//! one explicit shape — this adapter does not parse report formats — and they
//! go through the same redaction, normalization, fingerprint, store and
//! frozen evidence as a span.
//!
//! Two things are particular to a report. The identity of an occurrence is
//! the place it was observed (execution, run, attempt, scenario, phase, side,
//! revision, artifact, index), so importing the same package twice never
//! counts twice. And a scenario can provoke a failure on purpose: its
//! contract says which, and that evidence is kept without becoming work.

use std::collections::BTreeMap;

use serde_json::json;

use crate::adapters::ErrorEvent;
use crate::evidence::{
    cap_bytes, EvidenceBundleV1, EvidenceTruncatedV1, EvidenceWorkerV1, EVIDENCE_VERSION,
};
use crate::{
    fingerprint, ErrorSourceV1, EvidenceReportV1, ExpectedErrorV1, GroupTriageV1, Redactor,
    ReportClassificationV1, ReportExcerptV1, ReportObservationV1, ReportsIngestRequestV1,
    SentinelError, TriageKindV1, TriageSourceV1,
};

/// The namespace report groups are filed under when the caller names none.
pub const DEFAULT_NAMESPACE: &str = "e2e";
/// Observations per call.
pub const MAX_OBSERVATIONS: usize = 500;
/// Excerpts per observation.
pub const MAX_EXCERPTS: usize = 8;
pub const EXCERPT_MAX_BYTES: usize = 16 * 1024;
const STACKTRACE_MAX_BYTES: usize = 32 * 1024;
/// Identifiers and names: long enough for any real one, short enough that a
/// field cannot smuggle a document in.
const ID_MAX_BYTES: usize = 512;
/// The `question` on the triage of a group the scenario contract settled.
pub const CONTRACT_QUESTION: &str = "scenario_contract.expected_errors";
const EXPECTED_CRITERION: &str = "The scenario contract declares this error as provoked on \
    purpose by the test: kept as evidence, not correction work.";

/// Refuse a request that cannot name its occurrences, before anything is
/// written.
pub fn validate(request: &ReportsIngestRequestV1) -> Result<(), SentinelError> {
    if request.report_id.trim().is_empty() {
        return Err(SentinelError::invalid("report_id is required"));
    }
    if request.observations.len() > MAX_OBSERVATIONS {
        return Err(SentinelError::invalid(format!(
            "at most {MAX_OBSERVATIONS} observations per call, got {}",
            request.observations.len()
        )));
    }
    for (index, observation) in request.observations.iter().enumerate() {
        for (field, value) in [
            ("execution_id", &observation.execution_id),
            ("run_id", &observation.run_id),
            ("scenario_id", &observation.scenario_id),
            ("phase", &observation.phase),
            ("revision", &observation.revision),
            ("artifact", &observation.artifact),
            ("service_name", &observation.service_name),
        ] {
            if value.trim().is_empty() {
                return Err(SentinelError::invalid(format!(
                    "observations[{index}].{field} is required"
                )));
            }
        }
        if observation.excerpts.len() > MAX_EXCERPTS {
            return Err(SentinelError::invalid(format!(
                "observations[{index}] carries more than {MAX_EXCERPTS} excerpts"
            )));
        }
    }
    Ok(())
}

/// One physical occurrence, whatever the strings in it contain: the parts are
/// hashed apart from each other, so `a:b` + `c` never meets `a` + `b:c`.
pub fn dedupe_key(observation: &ReportObservationV1) -> String {
    let attempt = observation.attempt.to_string();
    let index = observation.occurrence_index.to_string();
    format!(
        "report:{}",
        fingerprint::fingerprint(
            ErrorSourceV1::Report,
            &[
                &observation.execution_id,
                &observation.run_id,
                &attempt,
                &observation.scenario_id,
                &observation.phase,
                observation.side.as_str(),
                &observation.revision,
                &observation.artifact,
                &index,
            ],
        )
    )
}

/// Whether the observation is an error at all. A score that dropped with no
/// exception and no message is not one, and no runtime error is made up for
/// it.
pub fn has_error(observation: &ReportObservationV1) -> bool {
    !observation.message.trim().is_empty()
        || observation
            .exception_type
            .as_deref()
            .is_some_and(|kind| !kind.trim().is_empty())
}

/// The observation measured against its scenario contract.
pub fn classify(observation: &ReportObservationV1) -> ReportClassificationV1 {
    if !has_error(observation) {
        return ReportClassificationV1::InsufficientEvidence;
    }
    match &observation.expected_errors {
        None => ReportClassificationV1::InsufficientEvidence,
        Some(expected) if expected.is_empty() => ReportClassificationV1::Unexpected,
        Some(_) if matching(observation).is_some() => ReportClassificationV1::Expected,
        Some(_) => ReportClassificationV1::ExpectedMismatch,
    }
}

/// The contract entry this error answers to.
fn matching(observation: &ReportObservationV1) -> Option<&ExpectedErrorV1> {
    observation
        .expected_errors
        .as_deref()?
        .iter()
        .find(|expected| {
            expected
                .exception_type
                .as_ref()
                .is_none_or(|kind| observation.exception_type.as_ref() == Some(kind))
                && expected
                    .function_id
                    .as_ref()
                    .is_none_or(|function| observation.function_id.as_ref() == Some(function))
                && expected
                    .message_contains
                    .as_ref()
                    .is_none_or(|part| observation.message.contains(part.as_str()))
        })
}

/// What the fingerprint keys on in place of a function, when the report
/// names none: the case and the step that failed.
pub fn identity(observation: &ReportObservationV1) -> String {
    observation
        .function_id
        .clone()
        .filter(|function| !function.trim().is_empty())
        .unwrap_or_else(|| format!("e2e:{}:{}", observation.scenario_id, observation.phase))
}

/// A copy of the observation with every string the caller sent passed
/// through the redactor and capped: message, provenance, contract,
/// stacktrace and excerpts alike. Classification and the dedupe key are
/// taken from the original before this; everything stored comes from here.
pub fn redact(
    observation: &ReportObservationV1,
    message_max_bytes: usize,
    redactor: &Redactor,
    redactions: &mut u64,
) -> ReportObservationV1 {
    let mut text = |value: &str, max: usize| {
        let (value, hits) = redactor.redact(&cap_bytes(value.to_string(), max));
        *redactions += hits;
        value
    };
    ReportObservationV1 {
        execution_id: text(&observation.execution_id, ID_MAX_BYTES),
        run_id: text(&observation.run_id, ID_MAX_BYTES),
        attempt: observation.attempt,
        scenario_id: text(&observation.scenario_id, ID_MAX_BYTES),
        phase: text(&observation.phase, ID_MAX_BYTES),
        side: observation.side,
        revision: text(&observation.revision, ID_MAX_BYTES),
        artifact: text(&observation.artifact, ID_MAX_BYTES),
        occurrence_index: observation.occurrence_index,
        at_ms: observation.at_ms,
        service_name: text(&observation.service_name, ID_MAX_BYTES),
        function_id: observation
            .function_id
            .as_deref()
            .map(|value| text(value, ID_MAX_BYTES)),
        exception_type: observation
            .exception_type
            .as_deref()
            .map(|value| text(value, ID_MAX_BYTES)),
        message: text(&observation.message, message_max_bytes),
        stacktrace: observation
            .stacktrace
            .as_deref()
            .map(|value| text(value, STACKTRACE_MAX_BYTES)),
        trace_id: observation
            .trace_id
            .as_deref()
            .map(|value| text(value, ID_MAX_BYTES)),
        span_id: observation
            .span_id
            .as_deref()
            .map(|value| text(value, ID_MAX_BYTES)),
        session_id: observation
            .session_id
            .as_deref()
            .map(|value| text(value, ID_MAX_BYTES)),
        expected_errors: observation.expected_errors.as_ref().map(|entries| {
            entries
                .iter()
                .map(|entry| ExpectedErrorV1 {
                    exception_type: entry
                        .exception_type
                        .as_deref()
                        .map(|value| text(value, ID_MAX_BYTES)),
                    function_id: entry
                        .function_id
                        .as_deref()
                        .map(|value| text(value, ID_MAX_BYTES)),
                    message_contains: entry
                        .message_contains
                        .as_deref()
                        .map(|value| text(value, EXCERPT_MAX_BYTES)),
                })
                .collect()
        }),
        excerpts: observation
            .excerpts
            .iter()
            .map(|excerpt| ReportExcerptV1 {
                name: text(&excerpt.name, ID_MAX_BYTES),
                content: text(&excerpt.content, EXCERPT_MAX_BYTES),
            })
            .collect(),
    }
}

/// Build the event from a [`redact`]ed observation. `service_name` is
/// already alias-resolved; `dedupe_key` comes from the original.
pub fn event_for(
    redacted: &ReportObservationV1,
    dedupe_key: String,
    namespace: &str,
    service_name: &str,
) -> ErrorEvent {
    ErrorEvent {
        source: ErrorSourceV1::Report,
        dedupe_key,
        at_ms: redacted.at_ms,
        service_name: service_name.to_string(),
        namespace: namespace.to_string(),
        function_id: redacted.function_id.clone(),
        span_name: Some(identity(redacted)),
        exception_type: redacted.exception_type.clone(),
        message: redacted.message.clone(),
        stacktrace: redacted.stacktrace.clone(),
        trace_id: redacted.trace_id.clone(),
        span_id: redacted.span_id.clone(),
        session_id: redacted.session_id.clone(),
        turn_id: None,
        worker_version: Some(redacted.revision.clone()),
        namespace_ambiguous: false,
        evidence: None,
    }
}

/// The frozen evidence, from a [`redact`]ed observation: no span tree
/// survives the stack, so the bundle is the report's own account of where
/// and how it failed.
pub fn bundle_for(
    redacted: &ReportObservationV1,
    report_id: &str,
    classification: ReportClassificationV1,
    service_name: &str,
    captured_at_ms: i64,
) -> EvidenceBundleV1 {
    EvidenceBundleV1 {
        version: EVIDENCE_VERSION,
        captured_at_ms,
        // A report is final: there is nothing left to re-read.
        settled: true,
        trace_id: redacted.trace_id.clone().unwrap_or_default(),
        origin_span_id: redacted.span_id.clone().unwrap_or_default(),
        propagated_through: Vec::new(),
        trace_tags: BTreeMap::new(),
        spans: Vec::new(),
        logs: Vec::new(),
        worker: EvidenceWorkerV1 {
            service_name: service_name.to_string(),
            version: Some(redacted.revision.clone()),
        },
        truncated: EvidenceTruncatedV1::default(),
        report: Some(EvidenceReportV1 {
            report_id: report_id.to_string(),
            execution_id: redacted.execution_id.clone(),
            run_id: redacted.run_id.clone(),
            attempt: redacted.attempt,
            scenario_id: redacted.scenario_id.clone(),
            phase: redacted.phase.clone(),
            side: redacted.side,
            revision: redacted.revision.clone(),
            artifact: redacted.artifact.clone(),
            occurrence_index: redacted.occurrence_index,
            classification,
            expected_errors: redacted.expected_errors.clone(),
            stacktrace: redacted.stacktrace.clone(),
            excerpts: redacted.excerpts.clone(),
        }),
    }
}

/// The triage of a group whose failure the scenario provoked on purpose. A
/// rule, not a judgement: the contract already answered the question.
pub fn expected_triage(
    observation: &ReportObservationV1,
    redactor: &Redactor,
    at_ms: i64,
) -> GroupTriageV1 {
    let mut evidence = json!({
        "scenario_id": observation.scenario_id,
        "phase": observation.phase,
        "expected": matching(observation),
    });
    redactor.redact_json(&mut evidence);
    GroupTriageV1 {
        kind: TriageKindV1::TestTraffic,
        confidence: 1.0,
        source: TriageSourceV1::Rule,
        model: None,
        at_ms,
        question: Some(CONTRACT_QUESTION.into()),
        criterion: Some(EXPECTED_CRITERION.into()),
        evidence: Some(evidence),
        usage: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReportSideV1;

    fn observation() -> ReportObservationV1 {
        ReportObservationV1 {
            execution_id: "exe_1".into(),
            run_id: "run_1".into(),
            attempt: 1,
            scenario_id: "checkout".into(),
            phase: "act".into(),
            side: ReportSideV1::Candidate,
            revision: "git:abc1234".into(),
            artifact: "logs/worker.log".into(),
            occurrence_index: 0,
            at_ms: 1_789_000_000_000,
            service_name: "orders".into(),
            function_id: Some("orders::place".into()),
            exception_type: Some("Panic".into()),
            message: "index out of bounds".into(),
            stacktrace: None,
            trace_id: None,
            span_id: None,
            session_id: None,
            expected_errors: Some(Vec::new()),
            excerpts: Vec::new(),
        }
    }

    #[test]
    fn the_contract_decides_the_classification() {
        let mut seen = observation();
        assert_eq!(classify(&seen), ReportClassificationV1::Unexpected);

        seen.expected_errors = None;
        assert_eq!(
            classify(&seen),
            ReportClassificationV1::InsufficientEvidence
        );

        seen.expected_errors = Some(vec![ExpectedErrorV1 {
            exception_type: Some("Panic".into()),
            message_contains: Some("out of bounds".into()),
            ..ExpectedErrorV1::default()
        }]);
        assert_eq!(classify(&seen), ReportClassificationV1::Expected);

        seen.expected_errors = Some(vec![ExpectedErrorV1 {
            exception_type: Some("Timeout".into()),
            ..ExpectedErrorV1::default()
        }]);
        assert_eq!(classify(&seen), ReportClassificationV1::ExpectedMismatch);
    }

    #[test]
    fn a_score_drop_without_an_error_is_never_an_error() {
        let mut seen = observation();
        seen.exception_type = None;
        seen.message = "  ".into();
        assert!(!has_error(&seen));
        assert_eq!(
            classify(&seen),
            ReportClassificationV1::InsufficientEvidence
        );
    }

    #[test]
    fn every_identity_part_separates_occurrences() {
        let first = observation();
        assert_eq!(dedupe_key(&first), dedupe_key(&observation()));
        let mut baseline = observation();
        baseline.side = ReportSideV1::Baseline;
        assert_ne!(dedupe_key(&first), dedupe_key(&baseline));
        let mut retry = observation();
        retry.attempt = 2;
        assert_ne!(dedupe_key(&first), dedupe_key(&retry));
        let mut joined = observation();
        joined.execution_id = "exe_1:run_1".into();
        joined.run_id = String::new();
        assert_ne!(dedupe_key(&first), dedupe_key(&joined));
    }
}
