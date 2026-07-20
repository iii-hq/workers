use std::path::{Path, PathBuf};

use anyhow::Context;

use super::script_validation::validate_script;
use crate::expand::{compile_scenario, CompiledFixtureV1};
use crate::types::scenario::{AuthoredScenarioV1, CompiledScenarioV1};
use crate::types::script::RouterScriptV1;

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub dir: PathBuf,
    pub authored: AuthoredScenarioV1,
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    /// Compiled shared golden plus inferred session/policy aid.
    pub system_prompt_template: String,
}

impl ScenarioFixture {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let scenario_path = dir.join("scenario.yaml");
        let scenario: AuthoredScenarioV1 = serde_yaml::from_str(
            &std::fs::read_to_string(&scenario_path)
                .with_context(|| format!("reading {}", scenario_path.display()))?,
        )
        .with_context(|| format!("parsing {}", scenario_path.display()))?;

        let scenarios_root = dir.parent().with_context(|| {
            format!("scenario directory {} has no scenarios root", dir.display())
        })?;
        let prompt_path = scenarios_root.join("system-prompt.txt");
        let system_prompt_base = std::fs::read_to_string(&prompt_path)
            .with_context(|| format!("reading {}", prompt_path.display()))?;
        let CompiledFixtureV1 {
            scenario: compiled,
            script,
            system_prompt_template,
        } = compile_scenario(&scenario, &system_prompt_base)
            .with_context(|| format!("compiling {}", scenario_path.display()))?;

        let fixture = ScenarioFixture {
            dir: dir.to_path_buf(),
            authored: scenario,
            scenario: compiled,
            script,
            system_prompt_template,
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
