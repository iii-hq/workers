//! Compilation and placeholder expansion for authored scenarios.
//!
//! Authors work with aliases and typed replies. Compilation produces the
//! exact, strict runtime structures before any process is started.

mod expectations;
mod functions;
mod render;
mod router;
mod templates;
mod tokens;
mod validation;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::types::scenario::{CompiledScenarioV1, IntegrationScenarioV1};
use crate::types::script::{ModelFixtureV1, RouterScriptV1};

pub use render::{render_authored_yaml, render_compiled};
pub use templates::{scenario_template, ScenarioTemplateKind};
pub use tokens::Placeholders;

const DEFAULT_MODEL: &str = "fixture-model";
const DEFAULT_PROVIDER: &str = "scripted";
const SYNTHETIC_FUNCTION_ALIAS: &str = "unused";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledFixtureV1 {
    pub scenario: CompiledScenarioV1,
    pub script: RouterScriptV1,
    pub system_prompt_template: String,
}

#[derive(Debug)]
struct CompiledFunctionCall {
    id: String,
    function: String,
    generation_index: usize,
    typed: bool,
}

/// Compile the concise authored contract to the strict structures consumed by
/// the runner and scripted router.
pub fn compile_scenario(
    authored: &IntegrationScenarioV1,
    system_prompt_base: &str,
) -> anyhow::Result<CompiledFixtureV1> {
    validation::validate_identity(authored)?;
    let model = authored
        .router
        .model
        .clone()
        .unwrap_or_else(default_model_fixture);
    if model.id.is_empty() || model.provider.is_empty() {
        anyhow::bail!("router model id and provider must be non-empty");
    }
    if authored.router.generations.is_empty() {
        anyhow::bail!("router has no generations");
    }

    let allowed_aliases = functions::allowed_aliases(authored)?;
    let function_ids = functions::function_ids(authored);
    let allowed_ids: Vec<String> = allowed_aliases
        .iter()
        .map(|alias| function_ids[alias].clone())
        .collect();
    let tools = functions::compile_tools(authored, &allowed_aliases, &function_ids);
    let recorder = functions::compile_recorder(authored, &allowed_aliases, &function_ids);
    let bindings = functions::compile_bindings(authored, &allowed_aliases, &function_ids)?;
    let calls = functions::function_call_ids(authored, &function_ids, &allowed_aliases)?;
    validation::validate_release(authored, &calls)?;
    let fault = functions::compile_fault(authored, &function_ids, &calls)?;

    let send = functions::compile_send(authored, &model, &allowed_ids)?;
    let script = router::compile_router(authored, &model, &tools, &function_ids, &calls)?;
    let invariants = expectations::compile_expectations(
        authored,
        &function_ids,
        &calls,
        script.generations.len(),
    )?;
    let system_prompt_template = compile_system_prompt(system_prompt_base, &allowed_ids);

    let fixture = CompiledFixtureV1 {
        scenario: CompiledScenarioV1 {
            schema_version: authored.schema_version,
            id: authored.id.clone(),
            description: authored.description.clone(),
            send,
            recorder,
            deadlines: authored.timeouts,
            invariants,
            fault,
            bindings,
            release: authored.release.clone(),
            quarantine: authored.quarantine,
        },
        script,
        system_prompt_template,
    };
    render::validate_placeholders(&fixture)?;
    Ok(fixture)
}

fn compile_system_prompt(base: &str, allowed_ids: &[String]) -> String {
    let base = base.strip_suffix('\n').unwrap_or(base);
    let policy = if allowed_ids.is_empty() {
        "Function dispatch is entirely disabled this turn — do not call any function.".to_string()
    } else {
        let mut allowed = allowed_ids.to_vec();
        allowed.sort();
        allowed.dedup();
        format!(
            "Your dispatch policy allows ONLY these functions: {}. This narrowed-policy \
             instruction OVERRIDES the general discovery requirement for this turn: call the \
             listed target ids directly when the task already supplies their arguments. Anything \
             else — including discovery (engine::functions::list / ::info) unless listed above — \
             is denied. Do not probe: if the task genuinely needs an unlisted function or an \
             unknown contract, report that blocker and finish.",
            allowed.join(", ")
        )
    };
    format!("{base}\n\nYour session id is {{{{session_id}}}}.\n{policy}")
}

fn default_model_fixture() -> ModelFixtureV1 {
    ModelFixtureV1 {
        id: DEFAULT_MODEL.to_string(),
        provider: DEFAULT_PROVIDER.to_string(),
        display_name: None,
        context_window: 32_768,
        max_output_tokens: 4_096,
        input_limit: None,
        supports_thinking: Some(false),
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: Some(true),
        supports_vision: Some(false),
        supports_cache: Some(false),
        supports_structured_output: Some(true),
        thinking_budgets: None,
        pricing: None,
    }
}
