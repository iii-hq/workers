use crate::discovery;
use crate::runtime::{RunError, RunPhase};
use crate::services::RunServices;
use crate::stack::Stack;

use super::super::runner::ScenarioRunner;
use super::super::state::PreparedRun;

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn arm(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        prepared: &PreparedRun,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Arm;
        let probe = services.probe();
        let scenario = &prepared.scenario;
        let deadline = prepared.setup_deadline;

        probe
            .register_target(&self.run_id, scenario.target.as_ref())
            .map_err(|error| RunError::setup(phase, "register controlled function", error))?;

        stack
            .spawn_harness(self.bins)
            .map_err(|error| RunError::setup(phase, "spawn harness under test", error))?;

        // Harness and workers register asynchronously; wait before bind/send.
        discovery::wait_for_functions(services.client(), discovery::TURN_SURFACE, deadline)
            .await
            .map_err(|error| RunError::setup(phase, "wait for turn function surface", error))?;
        discovery::wait_for_trigger_types(
            services.client(),
            &["harness::turn-started", "harness::turn-completed"],
            deadline,
        )
        .await
        .map_err(|error| RunError::setup(phase, "wait for harness trigger types", error))?;

        probe
            .bind_completion(&self.session_id)
            .await
            .map_err(|error| RunError::setup(phase, "bind completion observer", error))?;
        self.sink_mut(phase)?
            .write_scenario_text(
                &scenario.id,
                "expected-system-prompt.txt",
                &prepared.expected_prompt,
            )
            .map_err(|error| RunError::runner(phase, "write expected system prompt", error))?;

        Ok(())
    }
}
