use crate::expand::CompiledFixtureV1;
use crate::scenarios::{ScenarioDriver, VerifyFn};
use crate::types::scenario::CompiledScenarioV1;
use crate::types::script::RouterScriptV1;

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub slug: String,
    pub driver: ScenarioDriver,
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    /// Compiled Harness default plus inferred session/policy aid.
    pub system_prompt_template: String,
    /// Scenario-specific checks over the collected run evidence.
    pub verify: VerifyFn,
}

impl ScenarioFixture {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.slug.is_empty()
                && self
                    .slug
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "scenario slug {:?} is not filesystem-safe",
            self.slug
        );
        anyhow::ensure!(
            !self.scenario.id.is_empty()
                && self.scenario.id.len() <= 128
                && self
                    .scenario
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "scenario id {:?} is not filesystem-safe",
            self.scenario.id
        );
        anyhow::ensure!(
            !self.scenario.description.trim().is_empty(),
            "scenario description must not be empty"
        );
        if self.script.scenario_id != self.scenario.id {
            anyhow::bail!(
                "script scenario_id {:?} does not match scenario id {:?}",
                self.script.scenario_id,
                self.scenario.id
            );
        }
        if !self
            .scenario
            .recorder
            .target
            .function_id
            .starts_with("{{run_id}}::")
        {
            anyhow::bail!(
                "recorder function {:?} must be run-scoped",
                self.scenario.recorder.target.function_id
            );
        }
        anyhow::ensure!(
            !self.script.generations.is_empty(),
            "router script has no generations"
        );
        let mut ordinals = std::collections::BTreeSet::new();
        for generation in &self.script.generations {
            anyhow::ensure!(
                ordinals.insert(generation.ordinal),
                "duplicate router generation ordinal {}",
                generation.ordinal
            );
            anyhow::ensure!(
                generation
                    .frames
                    .last()
                    .is_some_and(|frame| frame.is_terminal()),
                "generation {} does not end in a terminal frame",
                generation.ordinal
            );
            anyhow::ensure!(
                !generation.frames[..generation.frames.len() - 1]
                    .iter()
                    .any(|frame| frame.is_terminal()),
                "generation {} contains an early terminal frame",
                generation.ordinal
            );
        }
        crate::expand::expand_compiled_fixture(
            &self.compiled(),
            "validate-run",
            "validate-session",
        )?;
        Ok(())
    }

    pub fn compiled(&self) -> CompiledFixtureV1 {
        CompiledFixtureV1 {
            scenario: self.scenario.clone(),
            script: self.script.clone(),
            system_prompt_template: self.system_prompt_template.clone(),
        }
    }
}
