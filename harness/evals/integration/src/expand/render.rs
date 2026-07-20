use anyhow::Context;
use serde_json::json;

use super::{expand_compiled_fixture, CompiledFixtureV1};
use crate::types::scenario::IntegrationScenarioV1;

pub(super) fn validate_placeholders(fixture: &CompiledFixtureV1) -> anyhow::Result<()> {
    render_compiled(fixture)
        .map(|_| ())
        .context("validating compiled placeholders")
}

/// Deterministic, fully expanded representation used by the `render` CLI.
pub fn render_compiled(fixture: &CompiledFixtureV1) -> anyhow::Result<String> {
    let run_id = format!("render-{}", fixture.scenario.id.to_ascii_lowercase());
    let session_id = format!(
        "render-session-{}",
        fixture.scenario.id.to_ascii_lowercase()
    );
    let expanded = expand_compiled_fixture(fixture, &run_id, &session_id)?;
    Ok(crate::canonical::canonical_json_pretty(&json!({
        "scenario": expanded.scenario,
        "router_script": expanded.script,
        "system_prompt": expanded.system_prompt
    })))
}

/// Serialize an authored scenario in stable YAML for `init`.
pub fn render_authored_yaml(scenario: &IntegrationScenarioV1) -> anyhow::Result<String> {
    let mut rendered = serde_yaml::to_string(scenario)?;
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}
