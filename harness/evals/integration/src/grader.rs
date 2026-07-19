//! Pure-code grading: every invariant reads collected evidence only, with no
//! engine calls or subject mutation.

mod dispatch;
mod helpers;
mod router_target;
mod send_transcript;
mod stabilize;
mod status_lifecycle;

#[cfg(test)]
mod tests;

use serde_json::Value;

use crate::types::recorder::RecorderEventV1;
use crate::types::scenario::{InvariantResultV1, InvariantSpecV1};

type GradeOutcome = (bool, Value, Value, Vec<&'static str>);

/// Everything the grader may look at, exactly as persisted to the artifact
/// directory (grading twice over the same evidence is byte-deterministic).
pub struct Evidence {
    pub run_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub send_response: Option<Value>,
    /// Final `harness::status` report (JSON null when the session is unknown).
    pub status: Value,
    /// All transcript `MessageItem`s across pages, in order.
    pub transcript: Vec<Value>,
    pub generations_consumed: u64,
    pub generations_total: u64,
    pub recorder_events: Vec<RecorderEventV1>,
}

pub fn grade(specs: &[InvariantSpecV1], evidence: &Evidence) -> Vec<InvariantResultV1> {
    specs
        .iter()
        .map(|spec| {
            let (passed, mut expected, mut actual, refs) = dispatch::grade_one(spec, evidence);
            stabilize::stabilize_value(&mut expected, evidence);
            stabilize::stabilize_value(&mut actual, evidence);
            InvariantResultV1 {
                id: spec.id.clone(),
                passed,
                expected,
                actual,
                evidence_refs: refs.into_iter().map(String::from).collect(),
            }
        })
        .collect()
}
