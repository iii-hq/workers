use serde_json::json;

use crate::types::scenario::{InvariantKind, InvariantSpecV1};

use super::{router_target, send_transcript, status_lifecycle, Evidence, GradeOutcome};

pub(super) fn grade_one(spec: &InvariantSpecV1, evidence: &Evidence) -> GradeOutcome {
    let Some(kind) = InvariantKind::from_id(&spec.id) else {
        return (
            false,
            json!({ "error": "unknown invariant id" }),
            json!({ "id": spec.id }),
            vec![],
        );
    };
    let params = &spec.parameters;
    match kind {
        InvariantKind::SendFlags => send_transcript::grade_send_flags(params, evidence),
        InvariantKind::TranscriptMessageCounts => {
            send_transcript::grade_message_counts(params, evidence)
        }
        InvariantKind::TranscriptAssistantText => {
            send_transcript::grade_assistant_text(params, evidence)
        }
        InvariantKind::TranscriptFunctionResult => {
            send_transcript::grade_function_result(params, evidence)
        }
        InvariantKind::TranscriptCallsClosed => send_transcript::grade_calls_closed(evidence),
        InvariantKind::TranscriptNoDuplicates => send_transcript::grade_no_duplicates(evidence),
        InvariantKind::StatusTerminal => status_lifecycle::grade_terminal(params, evidence),
        InvariantKind::LifecycleCompletedOnce => {
            status_lifecycle::grade_completed_once(params, evidence)
        }
        InvariantKind::RouterGenerationsConsumed => {
            router_target::grade_generations_consumed(params, evidence)
        }
        InvariantKind::TargetCalls => router_target::grade_target_calls(params, evidence),
    }
}
