use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde_json::{json, Value};

use crate::types::frames::{AssistantMessageEvent, ContentBlock};
use crate::types::recorder::{
    LifecycleFunctionId, LifecycleTriggerType, RecorderConfigV1, RecorderLifecycleV1,
    RecorderTargetV1,
};
use crate::types::scenario::{
    AuthoredScenarioV1, CompiledFaultV1, RouterReplyV1, ScenarioFunctionV1, TriggerBindingV1,
};
use crate::types::script::ModelFixtureV1;

use super::{
    validation::{validate_function_arguments, validate_hook_response},
    CompiledFunctionCall, SYNTHETIC_FUNCTION_ALIAS,
};

pub(super) fn function_ids(authored: &AuthoredScenarioV1) -> BTreeMap<String, String> {
    authored
        .functions
        .keys()
        .map(|alias| (alias.clone(), format!("{{{{run_id}}}}::{alias}")))
        .collect()
}

pub(super) fn allowed_aliases(authored: &AuthoredScenarioV1) -> anyhow::Result<Vec<String>> {
    let mut aliases = match &authored.send.allow {
        Some(aliases) => aliases.clone(),
        None => authored
            .functions
            .iter()
            .filter(|(_, function)| function.expose)
            .map(|(alias, _)| alias.clone())
            .collect(),
    };
    let mut seen = BTreeSet::new();
    for alias in &aliases {
        if !authored.functions.contains_key(alias) {
            anyhow::bail!("send.allow references unknown function alias {alias:?}");
        }
        if !seen.insert(alias) {
            anyhow::bail!("send.allow contains duplicate function alias {alias:?}");
        }
    }
    aliases.sort();
    Ok(aliases)
}

pub(super) fn compile_tools(
    authored: &AuthoredScenarioV1,
    allowed_aliases: &[String],
    function_ids: &BTreeMap<String, String>,
) -> Value {
    Value::Array(
        allowed_aliases
            .iter()
            .map(|alias| {
                let function = &authored.functions[alias];
                json!({
                    "name": function_ids[alias],
                    "description": function.description,
                    "parameters": function.request_schema,
                    "execution_mode": "sequential"
                })
            })
            .collect(),
    )
}

pub(super) fn compile_recorder(
    authored: &AuthoredScenarioV1,
    allowed_aliases: &[String],
    function_ids: &BTreeMap<String, String>,
) -> RecorderConfigV1 {
    if authored.functions.is_empty() {
        return RecorderConfigV1 {
            target: synthetic_target(),
            lifecycle: compiled_lifecycle(),
            extra_functions: Vec::new(),
        };
    }

    let target_alias = allowed_aliases
        .first()
        .cloned()
        .or_else(|| authored.functions.keys().next().cloned())
        .expect("non-empty functions has a target");
    let target = compile_function(
        &function_ids[&target_alias],
        &authored.functions[&target_alias],
    );
    let extra_functions = authored
        .functions
        .iter()
        .filter(|(alias, _)| *alias != &target_alias)
        .map(|(alias, function)| compile_function(&function_ids[alias], function))
        .collect();
    RecorderConfigV1 {
        target,
        lifecycle: compiled_lifecycle(),
        extra_functions,
    }
}

fn compile_function(function_id: &str, function: &ScenarioFunctionV1) -> RecorderTargetV1 {
    RecorderTargetV1 {
        function_id: function_id.to_string(),
        description: function.description.clone(),
        request_schema: function.request_schema.clone(),
        response: function.response.clone(),
        hold_response_at: None,
    }
}

fn synthetic_target() -> RecorderTargetV1 {
    RecorderTargetV1 {
        function_id: format!("{{{{run_id}}}}::{SYNTHETIC_FUNCTION_ALIAS}"),
        description: "Synthetic integration target; must never be called.".to_string(),
        request_schema: json!({
            "type": "object",
            "additionalProperties": false
        })
        .as_object()
        .cloned()
        .expect("object"),
        response: json!({
            "content": [{ "type": "text", "text": "unused" }],
            "is_error": false
        }),
        hold_response_at: None,
    }
}

fn compiled_lifecycle() -> RecorderLifecycleV1 {
    RecorderLifecycleV1 {
        trigger_type: LifecycleTriggerType::TurnCompleted,
        function_id: LifecycleFunctionId::Lifecycle,
    }
}

pub(super) fn compile_bindings(
    authored: &AuthoredScenarioV1,
    allowed_aliases: &[String],
    function_ids: &BTreeMap<String, String>,
) -> anyhow::Result<Vec<TriggerBindingV1>> {
    authored
        .bindings
        .iter()
        .map(|binding| {
            let function_id = function_ids.get(&binding.function).with_context(|| {
                format!(
                    "binding references unknown controlled function alias {:?}",
                    binding.function
                )
            })?;
            let callback = &authored.functions[&binding.function];
            if callback.expose {
                anyhow::bail!(
                    "binding callback {:?} must set expose: false",
                    binding.function
                );
            }
            validate_hook_response(&binding.function, &callback.response)?;
            if binding.functions.is_empty() {
                anyhow::bail!(
                    "binding callback {:?} must select at least one exposed function",
                    binding.function
                );
            }
            let mut selected = Vec::with_capacity(binding.functions.len());
            let mut seen = BTreeSet::new();
            for alias in &binding.functions {
                let selected_id = function_ids.get(alias).cloned().with_context(|| {
                    format!("binding references unknown exposed function alias {alias:?}")
                })?;
                if !seen.insert(alias) {
                    anyhow::bail!(
                        "binding callback {:?} selects duplicate function alias {alias:?}",
                        binding.function
                    );
                }
                if !allowed_aliases.contains(alias) {
                    anyhow::bail!(
                        "binding callback {:?} selects function {alias:?} outside send.allow",
                        binding.function
                    );
                }
                selected.push(selected_id);
            }
            Ok(TriggerBindingV1 {
                trigger_type: binding.trigger.as_str().to_string(),
                function_id: function_id.clone(),
                config: json!({
                    "functions": selected,
                    "priority": binding.priority
                }),
            })
        })
        .collect()
}

pub(super) fn function_call_ids(
    authored: &AuthoredScenarioV1,
    function_ids: &BTreeMap<String, String>,
    allowed_aliases: &[String],
) -> anyhow::Result<Vec<CompiledFunctionCall>> {
    let mut calls = Vec::new();
    let mut seen = BTreeSet::new();
    for (generation_index, generation) in authored.router.generations.iter().enumerate() {
        match &generation.reply {
            RouterReplyV1::FunctionCall {
                id,
                function,
                arguments,
                ..
            } => {
                let call_ordinal = calls.len() + 1;
                let id = id.clone().unwrap_or_else(|| format!("call-{call_ordinal}"));
                validate_function_arguments(
                    authored,
                    function,
                    arguments,
                    &format!("generation {}", generation_index + 1),
                )?;
                register_call(
                    &mut calls,
                    &mut seen,
                    CompiledFunctionCall {
                        id,
                        function: function.clone(),
                        generation_index,
                        typed: true,
                    },
                )?;
            }
            RouterReplyV1::Raw { frames, .. } => {
                for message in frames.iter().filter_map(|frame| match frame {
                    AssistantMessageEvent::Done { message } => Some(message),
                    _ => None,
                }) {
                    for block in &message.content {
                        let ContentBlock::FunctionCall {
                            id,
                            function_id,
                            arguments,
                        } = block
                        else {
                            continue;
                        };
                        let function = function_ids
                            .iter()
                            .find_map(|(alias, resolved)| {
                                (resolved == function_id).then_some(alias)
                            })
                            .with_context(|| {
                                format!(
                                    "generation {} raw function call references unknown function id {function_id:?}",
                                    generation_index + 1
                                )
                            })?;
                        if !allowed_aliases.contains(function) {
                            anyhow::bail!(
                                "generation {} raw function call alias {function:?} is outside send.allow",
                                generation_index + 1
                            );
                        }
                        validate_function_arguments(
                            authored,
                            function,
                            arguments,
                            &format!("generation {} raw reply", generation_index + 1),
                        )?;
                        register_call(
                            &mut calls,
                            &mut seen,
                            CompiledFunctionCall {
                                id: id.clone(),
                                function: function.clone(),
                                generation_index,
                                typed: false,
                            },
                        )?;
                    }
                }
            }
            RouterReplyV1::Text { .. } => {}
        }
    }
    Ok(calls)
}

fn register_call(
    calls: &mut Vec<CompiledFunctionCall>,
    seen: &mut BTreeSet<String>,
    call: CompiledFunctionCall,
) -> anyhow::Result<()> {
    if call.id.trim().is_empty() {
        anyhow::bail!("function call id must not be empty");
    }
    if !seen.insert(call.id.clone()) {
        anyhow::bail!("duplicate function call id {:?}", call.id);
    }
    calls.push(call);
    Ok(())
}

pub(super) fn compile_fault(
    authored: &AuthoredScenarioV1,
    function_ids: &BTreeMap<String, String>,
    calls: &[CompiledFunctionCall],
) -> anyhow::Result<Option<CompiledFaultV1>> {
    let Some(fault) = &authored.fault else {
        return Ok(None);
    };
    if fault.after_target_calls == 0 {
        anyhow::bail!("fault.after_target_calls must be greater than zero");
    }
    let function = fault
        .function
        .as_deref()
        .or_else(|| calls.first().map(|call| call.function.as_str()))
        .context("fault injection requires a typed or raw function call")?;
    let function_id = function_ids
        .get(function)
        .with_context(|| format!("fault references unknown function alias {function:?}"))?;
    let matching_calls = calls
        .iter()
        .filter(|call| call.function == function)
        .count() as u64;
    if matching_calls < fault.after_target_calls {
        anyhow::bail!(
            "fault waits for {} call(s) to {function:?}, but only {matching_calls} are authored",
            fault.after_target_calls
        );
    }
    Ok(Some(CompiledFaultV1 {
        kind: fault.kind,
        function_id: function_id.clone(),
        after_target_calls: fault.after_target_calls,
        restart_delay_ms: fault.restart_delay_ms,
    }))
}

pub(super) fn hold_fault_target(
    recorder: &mut RecorderConfigV1,
    function_id: &str,
    call_ordinal: u64,
) -> anyhow::Result<()> {
    let target = std::iter::once(&mut recorder.target)
        .chain(recorder.extra_functions.iter_mut())
        .find(|target| target.function_id == function_id)
        .with_context(|| format!("fault target {function_id:?} is not a controlled function"))?;
    target.hold_response_at = Some(call_ordinal);
    Ok(())
}

pub(super) fn compile_send(
    authored: &AuthoredScenarioV1,
    model: &ModelFixtureV1,
    allowed_ids: &[String],
) -> anyhow::Result<crate::types::scenario::CompiledSendV1> {
    use crate::types::scenario::{
        CompiledFunctionExposureV1, CompiledFunctionPolicyV1, CompiledSendOptionsV1, CompiledSendV1,
    };

    Ok(CompiledSendV1 {
        session_id: "{{session_id}}".to_string(),
        message: authored.send.message.clone(),
        model: model.id.clone(),
        provider: model.provider.clone(),
        idempotency_key: authored
            .send
            .idempotency_key
            .clone()
            .unwrap_or_else(|| format!("{{{{run_id}}}}:{}", authored.id.to_ascii_lowercase())),
        options: CompiledSendOptionsV1 {
            functions: CompiledFunctionPolicyV1 {
                allow: allowed_ids.to_vec(),
                deny: Vec::new(),
                expose: CompiledFunctionExposureV1::Native,
            },
        },
    })
}
