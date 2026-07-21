use anyhow::Context;

use super::script_validation::validate_script;
use crate::expand::{compile_scenario, CompiledFixtureV1};
use crate::scenarios::{RegisteredScenario, ScenarioDriver, VerifyFn};
use crate::types::scenario::CompiledScenarioV1;
use crate::types::script::RouterScriptV1;

const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/default.txt");

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub slug: String,
    pub driver: ScenarioDriver,
    pub quarantine: bool,
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    /// Compiled Harness default plus inferred session/policy aid.
    pub system_prompt_template: String,
    /// Scenario-specific checks over the collected run evidence.
    pub verify: VerifyFn,
}

impl ScenarioFixture {
    /// Compile one registered scenario against the Harness default prompt.
    pub fn from_registered(entry: &RegisteredScenario) -> anyhow::Result<Self> {
        let CompiledFixtureV1 {
            scenario: compiled,
            script,
            system_prompt_template,
        } = compile_scenario(&entry.authored, DEFAULT_SYSTEM_PROMPT)
            .with_context(|| format!("compiling scenario {}", entry.slug))?;

        let fixture = ScenarioFixture {
            slug: entry.slug.clone(),
            driver: entry.driver,
            quarantine: entry.authored.quarantine,
            scenario: compiled,
            script,
            system_prompt_template,
            verify: entry.verify,
        };
        fixture.validate()?;
        Ok(fixture)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.script.scenario_id != self.scenario.id {
            anyhow::bail!(
                "script scenario_id {:?} does not match scenario id {:?}",
                self.script.scenario_id,
                self.scenario.id
            );
        }
        for declared in std::iter::once(&self.scenario.recorder.target)
            .chain(self.scenario.recorder.extra_functions.iter())
        {
            if !declared.function_id.starts_with("{{run_id}}::") {
                anyhow::bail!(
                    "recorder function {:?} must be scoped by the {{{{run_id}}}}:: prefix",
                    declared.function_id
                );
            }
        }
        for binding in &self.scenario.bindings {
            if !binding.function_id.starts_with("{{run_id}}::") {
                anyhow::bail!(
                    "scenario binding {:?} must bind a run-scoped function",
                    binding.function_id
                );
            }
        }
        validate_script(&self.script)
            .with_context(|| format!("router script for {}", self.scenario.id))?;
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
