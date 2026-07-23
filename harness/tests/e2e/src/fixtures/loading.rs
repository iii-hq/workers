use crate::expand::CompiledFixtureV1;
use crate::scenarios::{ScenarioDriver, VerifyFn};
use crate::types::scenario::CompiledScenarioV1;
use crate::types::script::RouterScriptV1;

#[derive(Debug, Clone)]
pub struct ScenarioFixture {
    pub slug: String,
    pub driver: ScenarioDriver,
    /// Number of distinct terminal turns the run must observe. Usually 1;
    /// Playground turns and harness-initiated follow-on turns (e.g. a reseed
    /// after a message parks mid-final-step) push it higher. Always equals
    /// `expected_turn_statuses.len()`.
    pub expected_terminal_turns: usize,
    /// Each terminal turn's lifecycle status, in completion order. The last
    /// must be `completed` — the floor's durable-status check binds to it.
    pub expected_turn_statuses: Vec<String>,
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
        anyhow::ensure!(
            self.expected_terminal_turns > 0,
            "scenario must expect at least one terminal turn"
        );
        anyhow::ensure!(
            self.expected_turn_statuses.len() == self.expected_terminal_turns,
            "scenario declares {} turn status(es) for {} terminal turn(s)",
            self.expected_turn_statuses.len(),
            self.expected_terminal_turns
        );
        for status in &self.expected_turn_statuses {
            anyhow::ensure!(
                matches!(status.as_str(), "completed" | "failed" | "cancelled"),
                "unknown terminal turn status {status:?}"
            );
        }
        anyhow::ensure!(
            self.expected_turn_statuses.last().map(String::as_str) == Some("completed"),
            "the last terminal turn must be completed"
        );
        // A direct scenario is one external send, but the harness may seed
        // further turns from it (a reseed after a parked message); those extra
        // terminal turns are declared with `terminal_turns(n)` and awaited the
        // same way Playground awaits its externally driven turns.
        if self.script.scenario_id != self.scenario.id {
            anyhow::bail!(
                "script scenario_id {:?} does not match scenario id {:?}",
                self.script.scenario_id,
                self.scenario.id
            );
        }
        if let Some(target) = &self.scenario.target {
            anyhow::ensure!(
                target.function_id.starts_with("{{run_id}}::"),
                "controlled function {:?} must be run-scoped",
                target.function_id
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
            if generation.failure.is_some() {
                anyhow::ensure!(
                    generation.frames.is_empty(),
                    "failing generation {} must stream no frames",
                    generation.ordinal
                );
                continue;
            }
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

    /// Distinct trace trees the run must produce. Turns chain into their
    /// initiating trace through the durable queue, so a direct scenario's one
    /// send yields exactly one trace no matter how many turns it seeds (a
    /// finalize reseed enqueues from inside the finalizing step); Playground
    /// turns are each externally initiated and trace separately.
    pub fn expected_traces(&self) -> usize {
        match self.driver {
            ScenarioDriver::Direct => 1,
            ScenarioDriver::Playground => self.expected_terminal_turns,
        }
    }

    pub fn compiled(&self) -> CompiledFixtureV1 {
        CompiledFixtureV1 {
            scenario: self.scenario.clone(),
            script: self.script.clone(),
            system_prompt_template: self.system_prompt_template.clone(),
        }
    }
}
