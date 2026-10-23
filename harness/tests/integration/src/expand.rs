//! Run-scoped placeholder expansion for the three checked-in fixtures.

mod tokens;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::scenario::CompiledScenarioV1;
use crate::types::script::RouterScriptV1;

pub(crate) use tokens::Placeholders;

pub(crate) const ALLOWED_FUNCTIONS_MARKER: &str = "__ALLOWED_FUNCTIONS__";
/// Where the runtime context names the session's pinned response language.
/// The harness pins it from the user's first message with enough prose
/// (`harness::language`), so the line exists only for some messages — and a
/// message carrying a run id can tip the verdict either way. It is therefore
/// resolved here, against the EXPANDED send message, with the harness's own
/// detector: a fixture states only where the line goes.
pub(crate) const RESPONSE_LANGUAGE_MARKER: &str = "__RESPONSE_LANGUAGE__";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledFixtureV1 {
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    pub system_prompt_template: String,
}

#[derive(Debug)]
pub(crate) struct ExpandedFixtureV1 {
    pub(crate) scenario: CompiledScenarioV1,
    pub(crate) script: RouterScriptV1,
    pub(crate) system_prompt: String,
}

/// Resolve all run-scoped placeholders in a compiled fixture.
///
/// The system prompt is expanded before the router script because its digest
/// is itself a placeholder consumed by that script.
pub(crate) fn expand_compiled_fixture(
    fixture: &CompiledFixtureV1,
    run_id: &str,
    session_id: &str,
) -> anyhow::Result<ExpandedFixtureV1> {
    let base = Placeholders::new(run_id, session_id);
    let mut scenario = serde_json::to_value(&fixture.scenario)?;
    base.expand_value(&mut scenario)?;
    let scenario: CompiledScenarioV1 = serde_json::from_value(scenario)?;

    let mut allowed_functions = scenario
        .send
        .options
        .functions
        .as_ref()
        .map(|policy| policy.allow.clone())
        .unwrap_or_default();
    allowed_functions.sort_unstable();
    allowed_functions.dedup();
    let system_prompt = base
        .expand_str(&fixture.system_prompt_template)?
        .replace(ALLOWED_FUNCTIONS_MARKER, &allowed_functions.join(", "))
        .replace(
            RESPONSE_LANGUAGE_MARKER,
            &response_language_line(&scenario.send.message),
        );
    let digest = crate::canonical::sha256_of_bytes(system_prompt.as_bytes());
    let placeholders = base.with_system_prompt_sha256(&digest);

    let mut script = serde_json::to_value(&fixture.script)?;
    placeholders.expand_value(&mut script)?;
    let script = serde_json::from_value(script)?;

    Ok(ExpandedFixtureV1 {
        scenario,
        script,
        system_prompt,
    })
}

/// The runtime-context line (newline included) the harness adds for the
/// language it pins from `message`, or nothing when it pins none.
fn response_language_line(message: &str) -> String {
    harness::language::detect_text(message)
        .map(|language| format!("{}\n", harness::language::runtime_line(&language)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_line_follows_the_harness_detector() {
        let english = "Call the recorder twice with collision-control ids.";
        let line = response_language_line(english);
        assert!(line.starts_with("Response language: English"), "{line}");
        assert!(line.ends_with('\n'));
        assert_eq!(response_language_line("ok"), "", "too short to pin");
    }
}
