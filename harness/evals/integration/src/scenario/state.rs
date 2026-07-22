use serde_json::Value;

use crate::deadline::Deadline;
use crate::types::recorder::RecorderEventV1;
use crate::types::scenario::CompiledScenarioV1;

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
    pub(super) recorder_events: Vec<RecorderEventV1>,
    pub(super) timed_out: bool,
}

impl ActiveTurn {
    pub(super) fn new(deadline: Deadline, turn_id: Option<String>, send_response: Value) -> Self {
        Self {
            deadline,
            turn_id,
            send_response,
            final_status: Value::Null,
            transcript: Vec::new(),
            recorder_events: Vec::new(),
            timed_out: false,
        }
    }

    pub(super) fn external(deadline: Deadline) -> Self {
        Self::new(deadline, None, Value::Null)
    }
}
