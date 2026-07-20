use serde_json::json;

use crate::readiness::{self, ReadinessSpec};
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::stack::{expected_config_entries, expected_harness_config_entry, Stack};

use super::super::runner::ScenarioRunner;
use super::super::state::PreparedRun;

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn probe(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        prepared: &PreparedRun,
    ) -> Result<(), RunError> {
        let spec = ReadinessSpec::pre_harness(expected_config_entries(&stack.paths));
        if let Err(report) =
            readiness::probe(services.client(), &spec, prepared.readiness_deadline).await
        {
            self.write_artifact(
                &prepared.scenario.id,
                "readiness-failure.json",
                &json!({ "phase": "pre_harness", "missing": report.missing }),
                RunPhase::Probe,
            )?;
            return Err(RunError::new(
                RunPhase::Probe,
                RunErrorKind::Setup,
                "pre-harness readiness failed",
            ));
        }
        Ok(())
    }

    pub(in crate::scenario) async fn arm(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        prepared: &PreparedRun,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Arm;
        let recorder = services.recorder();
        let scenario = &prepared.scenario;

        recorder
            .configure(&self.run_id, &scenario.recorder)
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Setup,
                    "configure controlled recorder",
                    error,
                )
            })?;

        recorder.reset(&self.run_id).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Setup,
                "reset controlled recorder",
                error,
            )
        })?;
        if !recorder
            .snapshot(None)
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Setup,
                    "snapshot controlled recorder after reset",
                    error,
                )
            })?
            .is_empty()
        {
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "recorder snapshot is not empty after reset",
            ));
        }

        let controlled_contracts = readiness::controlled_contracts(&scenario.recorder);
        let discovery_deadline = prepared.readiness_deadline;
        if let Err(report) = readiness::wait_for_contracts(
            services.client(),
            &controlled_contracts,
            discovery_deadline,
        )
        .await
        {
            self.write_artifact(
                &scenario.id,
                "readiness-failure.json",
                &json!({ "phase": "controlled_functions", "missing": report.missing }),
                phase,
            )?;
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "controlled function contracts did not match live discovery",
            ));
        }

        stack.spawn_harness(self.bins).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Setup,
                "spawn harness under test",
                error,
            )
        })?;

        let harness_deadline = prepared.readiness_deadline;
        let spec = ReadinessSpec::harness_surface(expected_harness_config_entry(&stack.paths));
        if let Err(report) = readiness::probe(services.client(), &spec, harness_deadline).await {
            self.write_artifact(
                &scenario.id,
                "readiness-failure.json",
                &json!({ "phase": "harness", "missing": report.missing }),
                phase,
            )?;
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "harness readiness failed",
            ));
        }

        recorder
            .bind_lifecycle(
                scenario.recorder.lifecycle.trigger_type.as_str(),
                &self.session_id,
            )
            .await
            .map_err(|error| {
                RunError::with_source(phase, RunErrorKind::Setup, "bind lifecycle recorder", error)
            })?;
        for binding in &scenario.bindings {
            recorder
                .bind(
                    &binding.trigger_type,
                    &binding.function_id,
                    binding.config.clone(),
                )
                .await
                .map_err(|error| {
                    RunError::with_source(
                        phase,
                        RunErrorKind::Setup,
                        format!(
                            "bind trigger {} to {}",
                            binding.trigger_type, binding.function_id
                        ),
                        error,
                    )
                })?;
        }

        let expected_bindings = super::expected_trigger_bindings(scenario, &self.session_id);
        let binding_deadline = prepared.readiness_deadline;
        if let Err(report) = readiness::wait_for_registered_triggers(
            services.client(),
            &expected_bindings,
            binding_deadline,
        )
        .await
        {
            self.write_artifact(
                &scenario.id,
                "readiness-failure.json",
                &json!({ "phase": "trigger_bindings", "missing": report.missing }),
                phase,
            )?;
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "trigger bindings did not match live discovery",
            ));
        }

        self.sink
            .as_mut()
            .expect("artifact sink initialized after allocation")
            .write_scenario_text(
                &scenario.id,
                "expected-system-prompt.txt",
                &prepared.expected_prompt,
            )
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Runner,
                    "write expected system prompt",
                    error,
                )
            })?;

        Ok(())
    }
}
