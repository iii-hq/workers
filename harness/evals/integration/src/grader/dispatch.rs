use serde_json::json;

use crate::types::scenario::{DecodedInvariantV1, InvariantDecodeError, InvariantSpecV1};

use super::{router_target, send_transcript, status_lifecycle, Evidence, GradeOutcome};

pub(super) fn grade_one(spec: &InvariantSpecV1, evidence: &Evidence) -> GradeOutcome {
    let decoded = match spec.decode() {
        Ok(decoded) => decoded,
        Err(InvariantDecodeError::UnknownId(id)) => {
            return (
                false,
                json!({ "error": "unknown invariant id" }),
                json!({ "id": id }),
                vec![],
            );
        }
    };
    match decoded {
        DecodedInvariantV1::SendFlags(params) => {
            send_transcript::grade_send_flags(params, evidence)
        }
        DecodedInvariantV1::TranscriptMessageCounts(params) => {
            send_transcript::grade_message_counts(params, evidence)
        }
        DecodedInvariantV1::TranscriptAssistantText(params) => {
            send_transcript::grade_assistant_text(params, evidence)
        }
        DecodedInvariantV1::TranscriptFunctionResult(params) => {
            send_transcript::grade_function_result(params, evidence)
        }
        DecodedInvariantV1::TranscriptCallsClosed(_) => {
            send_transcript::grade_calls_closed(evidence)
        }
        DecodedInvariantV1::TranscriptNoDuplicates(_) => {
            send_transcript::grade_no_duplicates(evidence)
        }
        DecodedInvariantV1::StatusTerminal(params) => {
            status_lifecycle::grade_terminal(params, evidence)
        }
        DecodedInvariantV1::LifecycleCompletedOnce(params) => {
            status_lifecycle::grade_completed_once(params, evidence)
        }
        DecodedInvariantV1::RouterGenerationsConsumed(params) => {
            router_target::grade_generations_consumed(params, evidence)
        }
        DecodedInvariantV1::TargetCalls(params) => {
            router_target::grade_target_calls(params, evidence)
        }
    }
}
