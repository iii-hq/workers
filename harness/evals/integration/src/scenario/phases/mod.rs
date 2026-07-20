use std::time::Duration;

use serde_json::json;

use crate::readiness::ExpectedTriggerBinding;
use crate::types::scenario::CompiledScenarioV1;

mod completion;
mod evidence;
mod execution;
mod readiness;

const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_POLL_INTERVAL: Duration = Duration::from_millis(50);

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
