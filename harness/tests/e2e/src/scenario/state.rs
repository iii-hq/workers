use serde_json::Value;

use crate::deadline::Deadline;
use crate::types::scenario::CompiledScenarioV1;
use crate::types::trace::TraceEvidenceV1;

/// Fixture data expanded for one run before the stack starts.
pub(super) struct PreparedRun {
    pub(super) scenario: CompiledScenarioV1,
    pub(super) expected_prompt: String,
    pub(super) setup_deadline: Deadline,
}

impl PreparedRun {
    pub(super) fn new(scenario: CompiledScenarioV1, expected_prompt: String) -> Self {
        let setup_deadline = Deadline::after(std::time::Duration::from_millis(
            scenario.deadlines.readiness_ms,
        ));
        Self {
            scenario,
            expected_prompt,
            setup_deadline,
        }
    }
}

/// State that only exists after `harness::send` succeeds.
///
/// Keeping the deadline and response here makes Fault, Release, Await,
/// Collect, and Grade impossible to invoke without a completed Send phase.
pub(super) struct ActiveTurn {
    pub(super) deadline: Deadline,
    pub(super) turn_id: Option<String>,
    pub(super) send_response: Value,
    pub(super) final_status: Value,
    pub(super) transcript: Vec<Value>,
    pub(super) traces: TraceEvidenceV1,
    pub(super) trace_generation: u64,
    pub(super) timed_out: bool,
}

impl ActiveTurn {
    pub(super) fn new(
        deadline: Deadline,
        turn_id: Option<String>,
        send_response: Value,
        trace_generation: u64,
    ) -> Self {
        Self {
            deadline,
            turn_id,
            send_response,
            final_status: Value::Null,
            transcript: Vec::new(),
            traces: TraceEvidenceV1::new(Vec::new()),
            trace_generation,
            timed_out: false,
        }
    }

    pub(super) fn external(deadline: Deadline, trace_generation: u64) -> Self {
        Self::new(deadline, None, Value::Null, trace_generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_turn_keeps_the_pre_stimulus_trace_baseline() {
        let active = ActiveTurn::new(
            Deadline::after(std::time::Duration::from_secs(1)),
            None,
            Value::Null,
            7,
        );
        assert_eq!(active.trace_generation, 7);
    }
}
