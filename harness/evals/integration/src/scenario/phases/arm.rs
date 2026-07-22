use crate::discovery;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
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
        let recorder = services.recorder();
        let scenario = &prepared.scenario;
        let deadline = prepared.setup_deadline;

        recorder
            .configure(&self.run_id, &scenario.recorder)
            .map_err(|error| RunError::setup(phase, "configure controlled recorder", error))?;

        recorder
            .reset(&self.run_id)
            .map_err(|error| RunError::setup(phase, "reset controlled recorder", error))?;
        if !recorder
            .snapshot()
            .map_err(|error| {
                RunError::setup(phase, "snapshot controlled recorder after reset", error)
            })?
            .is_empty()
        {
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "recorder snapshot is not empty after reset",
            ));
        }

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

        recorder
            .bind_lifecycle(
                scenario.recorder.lifecycle.trigger_type.as_str(),
                &self.session_id,
            )
            .await
            .map_err(|error| RunError::setup(phase, "bind lifecycle recorder", error))?;
        for binding in &scenario.bindings {
            recorder
                .bind(
                    &binding.trigger_type,
                    &binding.function_id,
                    binding.config.clone(),
                )
                .await
                .map_err(|error| {
                    RunError::setup(
                        phase,
                        format!(
                            "bind trigger {} to {}",
                            binding.trigger_type, binding.function_id
                        ),
                        error,
                    )
                })?;
        }

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
