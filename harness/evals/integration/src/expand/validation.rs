use std::collections::BTreeMap;

use anyhow::Context;
use serde_json::Value;

use crate::types::scenario::{validate_scenario_id, IntegrationScenarioV1, RouterReplyV1};

use super::CompiledFunctionCall;

pub(super) fn validate_identity(authored: &IntegrationScenarioV1) -> anyhow::Result<()> {
    validate_scenario_id(&authored.id)?;
    if authored.description.trim().is_empty() {
        anyhow::bail!("scenario description must not be empty");
    }
    if authored.timeouts.readiness_ms == 0
        || authored.timeouts.scenario_ms == 0
        || authored.timeouts.teardown_ms == 0
    {
        anyhow::bail!("readiness, scenario, and teardown timeouts must be greater than zero");
    }
    for alias in authored.functions.keys() {
        validate_alias(alias)?;
    }
    validate_function_schemas(authored)?;
    Ok(())
}

fn validate_alias(alias: &str) -> anyhow::Result<()> {
    if alias.is_empty()
        || !alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        anyhow::bail!(
            "function alias {alias:?} must contain only ASCII letters, digits, '-' or '_'"
        );
    }
    Ok(())
}

fn validate_function_schemas(authored: &IntegrationScenarioV1) -> anyhow::Result<()> {
    for (alias, function) in &authored.functions {
        let schema = Value::Object(function.request_schema.clone());
        jsonschema::JSONSchema::compile(&schema).map_err(|error| {
            anyhow::anyhow!("function {alias:?} has an invalid request_schema: {error}")
        })?;
    }
    Ok(())
}

pub(super) fn validate_function_arguments(
    authored: &IntegrationScenarioV1,
    function: &str,
    arguments: &Value,
    context: &str,
) -> anyhow::Result<()> {
    let controlled = authored.functions.get(function).with_context(|| {
        format!("{context}: function call references unknown alias {function:?}")
    })?;
    let schema = Value::Object(controlled.request_schema.clone());
    let validator = jsonschema::JSONSchema::compile(&schema).map_err(|error| {
        anyhow::anyhow!("{context}: function {function:?} has an invalid request_schema: {error}")
    })?;
    if let Err(errors) = validator.validate(arguments) {
        let details = errors
            .take(5)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!(
            "{context}: arguments for function {function:?} do not match request_schema: {details}"
        );
    }
    Ok(())
}

pub(super) fn validate_hook_response(alias: &str, response: &Value) -> anyhow::Result<()> {
    let object = response
        .as_object()
        .with_context(|| format!("binding callback {alias:?} response must be an object"))?;
    if let Some(decision) = object.get("decision") {
        let decision = decision.as_str().with_context(|| {
            format!("binding callback {alias:?} response decision must be a string")
        })?;
        if !matches!(decision, "continue" | "deny" | "hold") {
            anyhow::bail!("binding callback {alias:?} has unsupported decision {decision:?}");
        }
    }
    if object
        .get("mutations")
        .is_some_and(|mutations| !mutations.is_object())
    {
        anyhow::bail!("binding callback {alias:?} response mutations must be an object");
    }
    Ok(())
}

pub(super) fn validate_release(
    authored: &IntegrationScenarioV1,
    calls: &[CompiledFunctionCall],
) -> anyhow::Result<()> {
    let Some(release) = &authored.release else {
        return Ok(());
    };
    let Some(call) = calls
        .iter()
        .find(|call| call.id == release.function_call_id)
    else {
        anyhow::bail!(
            "release references unknown function call {:?}",
            release.function_call_id
        );
    };
    let held = authored.bindings.iter().any(|binding| {
        binding.functions.contains(&call.function)
            && authored.functions[&binding.function]
                .response
                .get("decision")
                .and_then(Value::as_str)
                == Some("hold")
    });
    if !held {
        anyhow::bail!(
            "release for function call {:?} requires a selected hook with decision: hold",
            release.function_call_id
        );
    }
    Ok(())
}

pub(super) fn validate_reply(
    reply: &RouterReplyV1,
    ordinal: u64,
    authored: &IntegrationScenarioV1,
    function_ids: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    match reply {
        RouterReplyV1::Text { text, chunks, .. } => {
            if !chunks.is_empty() && chunks.concat() != *text {
                anyhow::bail!(
                    "generation {ordinal}: text chunks concatenate to {:?}, expected {:?}",
                    chunks.concat(),
                    text
                );
            }
        }
        RouterReplyV1::FunctionCall {
            function,
            arguments,
            ..
        } => {
            if !function_ids.contains_key(function) {
                anyhow::bail!(
                    "generation {ordinal}: function call references unknown alias {function:?}"
                );
            }
            let allowed = authored
                .send
                .allow
                .as_ref()
                .map(|aliases| aliases.contains(function))
                .unwrap_or(authored.functions[function].expose);
            if !allowed {
                anyhow::bail!(
                    "generation {ordinal}: function call alias {function:?} is not exposed by send"
                );
            }
            validate_function_arguments(
                authored,
                function,
                arguments,
                &format!("generation {ordinal}"),
            )?;
        }
        RouterReplyV1::Raw { .. } => {}
    }
    Ok(())
}
