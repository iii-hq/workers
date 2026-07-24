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

        // Register observer bindings before the harness exists. Recoverable
        // triggers park the harness-owned bindings until their types appear;
        // the acknowledged RPCs also barrier the probe function registrations.
        probe
            .bind_observers(&self.session_id, deadline)
            .await
            .map_err(|error| RunError::setup(phase, "bind scenario observers", error))?;

        stack
            .spawn_harness(self.bins, &self.fixture.harness_env)
            .map_err(|error| RunError::setup(phase, "spawn harness under test", error))?;

        probe
            .wait_until_ready(deadline)
            .await
            .map_err(|error| RunError::setup(phase, "wait for harness readiness", error))?;
        discovery::wait_for_functions(services.client(), discovery::TURN_SURFACE, deadline)
            .await
            .map_err(|error| RunError::setup(phase, "wait for turn function surface", error))?;
        probe
            .confirm_completion_binding(&self.session_id, deadline)
            .await
            .map_err(|error| {
                RunError::setup(phase, "confirm completion observer binding", error)
            })?;
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
