use std::time::Duration;

use serde_json::json;

use crate::readiness::ExpectedTriggerBinding;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::types::scenario::CompiledScenarioV1;

use super::runner::ScenarioRunner;

mod completion;
mod evidence;
mod execution;
mod readiness;

const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_POLL_INTERVAL: Duration = Duration::from_millis(50);

impl ScenarioRunner<'_> {
    /// Persist the readiness gap as `readiness-failure.json` and produce the
    /// phase error, preferring the artifact-write failure when both fail.
    fn readiness_failed<T: serde::Serialize>(
        &mut self,
        scenario_id: &str,
        phase: RunPhase,
        kind: RunErrorKind,
        stage: &str,
        message: &str,
        missing: &T,
    ) -> RunError {
        match self.write_artifact(
            scenario_id,
            "readiness-failure.json",
            &json!({ "phase": stage, "missing": missing }),
            phase,
        ) {
            Err(artifact_error) => artifact_error,
            Ok(()) => RunError::new(phase, kind, message),
        }
    }
}

fn expected_trigger_bindings(
    scenario: &CompiledScenarioV1,
    session_id: &str,
) -> Vec<ExpectedTriggerBinding> {
    std::iter::once(ExpectedTriggerBinding {
        trigger_type: scenario
            .recorder
            .lifecycle
            .trigger_type
            .as_str()
            .to_string(),
        function_id: "integration-recorder::lifecycle".to_string(),
        config: json!({ "session_id": session_id }),
    })
    .chain(
        scenario
            .bindings
            .iter()
            .map(|binding| ExpectedTriggerBinding {
                trigger_type: binding.trigger_type.clone(),
                function_id: binding.function_id.clone(),
                config: binding.config.clone(),
            }),
    )
    .collect()
}
