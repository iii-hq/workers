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
        probe.register_stand_ins(&self.fixture.stand_in_functions);

        // Observe before the harness exists. Observers of the harness's
        // functions stay pending until it registers them; the engine-hosted
        // span observer also barriers the probe function registrations.
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
        if self.fixture.intervention.is_some() {
            probe
                .bind_child_completion_observer(&self.session_id, deadline)
                .await
                .map_err(|error| RunError::setup(phase, "bind child completion observer", error))?;
        }
        self.setup_calls(services, deadline).await?;
        self.sink_mut(phase)?
            .write_scenario_text(
                &scenario.id,
                "expected-system-prompt.txt",
                &prepared.expected_prompt,
            )
            .map_err(|error| RunError::runner(phase, "write expected system prompt", error))?;

        Ok(())
    }

    /// The fixture's calls before the send, each once its function is
    /// registered; a failed call or an unexpected answer fails setup.
    async fn setup_calls(
        &self,
        services: &RunServices,
        deadline: crate::deadline::Deadline,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Arm;
        for call in &self.fixture.setup_calls {
            discovery::wait_for_functions(
                services.client(),
                &[call.function_id.as_str()],
                deadline,
            )
            .await
            .map_err(|error| RunError::setup(phase, "wait for a setup function", error))?;
            let mut payload = call.payload.clone();
            crate::expand::Placeholders::new(&self.run_id, &self.session_id)
                .expand_value(&mut payload)
                .map_err(|error| RunError::runner(phase, "expand setup-call payload", error))?;
            let output = services
                .client()
                .call_until_deadline(&call.function_id, payload, deadline)
                .await
                .map_err(|error| {
                    RunError::setup(
                        phase,
                        format!("setup call {}", call.function_id),
                        anyhow::anyhow!(error),
                    )
                })?;
            if let Some(expected) = &call.expected_output {
                let matched = crate::matcher::evaluate("setup output", expected, Some(&output));
                if !matched.passed {
                    return Err(RunError::new(
                        phase,
                        crate::runtime::RunErrorKind::Contract,
                        matched.detail,
                    ));
                }
            }
        }
        Ok(())
    }
}
