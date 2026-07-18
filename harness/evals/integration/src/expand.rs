//! Compilation and placeholder expansion for authored scenarios.
//!
//! Authors work with aliases and typed replies. Compilation produces the
//! exact, strict runtime structures before any process is started.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::types::frames::{
    AssistantMessage, AssistantMessageEvent, AssistantRoleTag, ContentBlock, RouterChatResponse,
    StopReason,
};
use crate::types::recorder::{
    LifecycleFunctionId, LifecycleTriggerType, RecorderConfigV1, RecorderLifecycleV1,
    RecorderTargetV1,
};
use crate::types::scenario::{
    validate_scenario_id, CompiledFaultV1, CompiledScenarioV1, ExpectationsV1,
    FunctionResultExpectationV1, IntegrationScenarioV1, InvariantSpecV1,
    MessageCountsExpectationV1, RouterReplyV1, ScenarioFunctionV1, TargetCallsExpectationV1,
    TerminalStatusV1, TriggerBindingV1,
};
use crate::types::script::{
    GenerationMatchV1, JsonMatcherV1, JsonNormalizerV1, ModelFixtureV1, NormalizerOperation,
    RouterScriptV1, SchemaVersion1, ScriptedGenerationV1,
};

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

/// Compile the concise authored contract to the strict structures consumed by
/// the runner and scripted router.
pub fn compile_scenario(
    authored: &IntegrationScenarioV1,
    system_prompt_base: &str,
) -> anyhow::Result<CompiledFixtureV1> {
    validate_identity(authored)?;
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

    let allowed_aliases = allowed_aliases(authored)?;
    let function_ids = function_ids(authored);
    let allowed_ids: Vec<String> = allowed_aliases
        .iter()
        .map(|alias| function_ids[alias].clone())
        .collect();
    let tools = compile_tools(authored, &allowed_aliases, &function_ids);
    let recorder = compile_recorder(authored, &allowed_aliases, &function_ids);
    let bindings = compile_bindings(authored, &allowed_aliases, &function_ids)?;
    let calls = function_call_ids(authored, &function_ids, &allowed_aliases)?;
    validate_release(authored, &calls)?;
    let fault = compile_fault(authored, &function_ids, &calls)?;

    let send = compile_send(authored, &model, &allowed_ids)?;
    let script = compile_router(authored, &model, &tools, &function_ids, &calls)?;
    let invariants =
        compile_expectations(authored, &function_ids, &calls, script.generations.len())?;
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
    validate_placeholders(&fixture)?;
    Ok(fixture)
}

fn validate_identity(authored: &IntegrationScenarioV1) -> anyhow::Result<()> {
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
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
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

fn validate_function_arguments(
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

fn function_ids(authored: &IntegrationScenarioV1) -> BTreeMap<String, String> {
    authored
        .functions
        .keys()
        .map(|alias| (alias.clone(), format!("{{{{run_id}}}}::{alias}")))
        .collect()
}

fn allowed_aliases(authored: &IntegrationScenarioV1) -> anyhow::Result<Vec<String>> {
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

fn compile_tools(
    authored: &IntegrationScenarioV1,
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

fn compile_recorder(
    authored: &IntegrationScenarioV1,
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
        response_delay_ms: function.response_delay_ms,
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
        response_delay_ms: None,
    }
}

fn compiled_lifecycle() -> RecorderLifecycleV1 {
    RecorderLifecycleV1 {
        trigger_type: LifecycleTriggerType::TurnCompleted,
        function_id: LifecycleFunctionId::Lifecycle,
    }
}

fn compile_bindings(
    authored: &IntegrationScenarioV1,
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

fn validate_hook_response(alias: &str, response: &Value) -> anyhow::Result<()> {
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

#[derive(Debug)]
struct CompiledFunctionCall {
    id: String,
    function: String,
    generation_index: usize,
    typed: bool,
}

fn function_call_ids(
    authored: &IntegrationScenarioV1,
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

fn validate_release(
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

fn compile_fault(
    authored: &IntegrationScenarioV1,
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
    let delay = authored.functions[function].response_delay_ms.unwrap_or(0);
    if delay == 0 {
        anyhow::bail!(
            "fault target {function:?} requires response_delay_ms > 0 for a deterministic interruption window"
        );
    }
    Ok(Some(CompiledFaultV1 {
        kind: fault.kind,
        function_id: function_id.clone(),
        after_target_calls: fault.after_target_calls,
        restart_delay_ms: fault.restart_delay_ms,
    }))
}

fn compile_send(
    authored: &IntegrationScenarioV1,
    model: &ModelFixtureV1,
    allowed_ids: &[String],
) -> anyhow::Result<Value> {
    Ok(json!({
        "session_id": "{{session_id}}",
        "message": authored.send.message,
        "model": model.id,
        "provider": model.provider,
        "idempotency_key": authored.send.idempotency_key.clone()
            .unwrap_or_else(|| format!("{{{{run_id}}}}:{}", authored.id.to_ascii_lowercase())),
        "options": {
            "functions": {
            "allow": allowed_ids,
            "deny": [],
            "expose": "native"
            }
        }
    }))
}

fn compile_router(
    authored: &IntegrationScenarioV1,
    model: &ModelFixtureV1,
    tools: &Value,
    function_ids: &BTreeMap<String, String>,
    calls: &[CompiledFunctionCall],
) -> anyhow::Result<RouterScriptV1> {
    let mut messages = vec![json!({
        "role": "user",
        "content": [{ "type": "text", "text": authored.send.message }]
    })];
    let mut generations = Vec::with_capacity(authored.router.generations.len());

    for (index, authored_generation) in authored.router.generations.iter().enumerate() {
        let ordinal = (index + 1) as u64;
        let call_id = if matches!(
            &authored_generation.reply,
            RouterReplyV1::FunctionCall { .. }
        ) {
            let id = calls
                .iter()
                .find(|call| call.generation_index == index && call.typed)
                .context("compiler lost a typed function-call id")?;
            Some(id.id.as_str())
        } else {
            None
        };
        validate_reply(&authored_generation.reply, ordinal, authored, function_ids)?;
        let (frames, response) = compile_reply(
            &authored_generation.reply,
            ordinal,
            model,
            function_ids,
            call_id,
        )?;
        let mut match_ = default_match(ordinal, model, &messages, tools);
        apply_match_overrides(&mut match_, &authored_generation.match_overrides);
        generations.push(ScriptedGenerationV1 {
            ordinal,
            match_,
            frames,
            response,
        });
        extend_history(
            &mut messages,
            &authored_generation.reply,
            model,
            authored,
            function_ids,
            call_id,
        )?;
    }

    Ok(RouterScriptV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: authored.id.clone(),
        model: model.clone(),
        generations,
    })
}

fn validate_reply(
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

fn default_match(
    ordinal: u64,
    model: &ModelFixtureV1,
    messages: &[Value],
    tools: &Value,
) -> GenerationMatchV1 {
    let normalize = (0..messages.len())
        .map(|index| JsonNormalizerV1 {
            pointer: format!("/{index}/timestamp"),
            operation: NormalizerOperation::Delete,
            replacement: None,
        })
        .collect();
    GenerationMatchV1 {
        writer_ref: JsonMatcherV1::Subset {
            expected: json!({ "direction": "write" }),
            normalize: None,
        },
        request_id: JsonMatcherV1::Regex {
            pattern: if ordinal == 1 {
                "^t_[0-9a-f]{32}:[0-9]+$".to_string()
            } else {
                format!("^t_[0-9a-f]{{32}}:{}$", ordinal - 1)
            },
        },
        model: JsonMatcherV1::Exact {
            expected: json!(model.id),
            normalize: None,
        },
        provider: JsonMatcherV1::Exact {
            expected: json!(model.provider),
            normalize: None,
        },
        system_prompt: JsonMatcherV1::Sha256 {
            expected: "{{system_prompt_sha256}}".to_string(),
        },
        messages: JsonMatcherV1::Exact {
            expected: Value::Array(messages.to_vec()),
            normalize: Some(normalize),
        },
        tools: JsonMatcherV1::Exact {
            expected: tools.clone(),
            normalize: None,
        },
        response_format: JsonMatcherV1::Absent,
        thinking_level: JsonMatcherV1::Absent,
        max_output_tokens: JsonMatcherV1::Absent,
        provider_options: JsonMatcherV1::Absent,
        metadata: JsonMatcherV1::Absent,
    }
}

fn apply_match_overrides(
    target: &mut GenerationMatchV1,
    overrides: &crate::types::scenario::GenerationMatchOverridesV1,
) {
    macro_rules! replace {
        ($field:ident) => {
            if let Some(value) = &overrides.$field {
                target.$field = value.clone();
            }
        };
    }
    replace!(writer_ref);
    replace!(request_id);
    replace!(model);
    replace!(provider);
    replace!(system_prompt);
    replace!(messages);
    replace!(tools);
    replace!(response_format);
    replace!(thinking_level);
    replace!(max_output_tokens);
    replace!(provider_options);
    replace!(metadata);
}

fn compile_reply(
    reply: &RouterReplyV1,
    ordinal: u64,
    model: &ModelFixtureV1,
    function_ids: &BTreeMap<String, String>,
    call_id: Option<&str>,
) -> anyhow::Result<(Vec<AssistantMessageEvent>, RouterChatResponse)> {
    match reply {
        RouterReplyV1::Text {
            text,
            chunks,
            usage,
        } => {
            let message = assistant_message(
                vec![ContentBlock::Text { text: text.clone() }],
                StopReason::End,
                usage.clone(),
                model,
                ordinal as i64,
            );
            let mut frames = Vec::new();
            if chunks.is_empty() {
                frames.push(AssistantMessageEvent::Done {
                    message: message.clone(),
                });
            } else {
                frames.push(AssistantMessageEvent::Start {
                    partial: assistant_message(
                        Vec::new(),
                        StopReason::End,
                        None,
                        model,
                        ordinal as i64,
                    ),
                });
                frames.push(AssistantMessageEvent::TextStart {
                    partial: assistant_message(
                        vec![ContentBlock::Text {
                            text: String::new(),
                        }],
                        StopReason::End,
                        None,
                        model,
                        ordinal as i64,
                    ),
                });
                frames.extend(chunks.iter().cloned().map(|delta| {
                    AssistantMessageEvent::TextDelta {
                        partial: None,
                        delta,
                    }
                }));
                frames.push(AssistantMessageEvent::TextEnd {
                    partial: assistant_message(
                        vec![ContentBlock::Text { text: text.clone() }],
                        StopReason::End,
                        None,
                        model,
                        ordinal as i64,
                    ),
                });
                if let Some(usage) = usage {
                    frames.push(AssistantMessageEvent::Usage {
                        usage: usage.clone(),
                    });
                }
                frames.push(AssistantMessageEvent::Stop {
                    stop_reason: StopReason::End,
                    error_message: None,
                    error_kind: None,
                });
                frames.push(AssistantMessageEvent::Done {
                    message: message.clone(),
                });
            }
            Ok((
                frames,
                RouterChatResponse {
                    ok: true,
                    provider: model.provider.clone(),
                    model: model.id.clone(),
                    stop_reason: Some(StopReason::End),
                    usage: usage.clone(),
                    error: None,
                },
            ))
        }
        RouterReplyV1::FunctionCall {
            function,
            arguments,
            usage,
            ..
        } => {
            let call_id = call_id.context("missing compiled function-call id")?;
            let message = assistant_message(
                vec![ContentBlock::FunctionCall {
                    id: call_id.to_string(),
                    function_id: function_ids[function].clone(),
                    arguments: arguments.clone(),
                }],
                StopReason::FunctionCall,
                usage.clone(),
                model,
                ordinal as i64,
            );
            Ok((
                vec![AssistantMessageEvent::Done { message }],
                RouterChatResponse {
                    ok: true,
                    provider: model.provider.clone(),
                    model: model.id.clone(),
                    stop_reason: Some(StopReason::FunctionCall),
                    usage: usage.clone(),
                    error: None,
                },
            ))
        }
        RouterReplyV1::Raw { frames, response } => Ok((frames.clone(), response.clone())),
    }
}

fn assistant_message(
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    usage: Option<crate::types::frames::Usage>,
    model: &ModelFixtureV1,
    timestamp: i64,
) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content,
        stop_reason,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: None,
        usage,
        model: model.id.clone(),
        provider: model.provider.clone(),
        timestamp,
    }
}

fn extend_history(
    messages: &mut Vec<Value>,
    reply: &RouterReplyV1,
    model: &ModelFixtureV1,
    authored: &IntegrationScenarioV1,
    function_ids: &BTreeMap<String, String>,
    call_id: Option<&str>,
) -> anyhow::Result<()> {
    match reply {
        RouterReplyV1::Text { text, .. } => messages.push(json!({
            "role": "assistant",
            "content": [{ "type": "text", "text": text }],
            "stop_reason": "end",
            "model": model.id,
            "provider": model.provider
        })),
        RouterReplyV1::FunctionCall {
            function,
            arguments,
            ..
        } => {
            let call_id = call_id.context("missing compiled function-call id")?;
            let function_id = &function_ids[function];
            messages.push(json!({
                "role": "assistant",
                "content": [{
                    "type": "function_call",
                    "id": call_id,
                    "function_id": function_id,
                    "arguments": arguments
                }],
                // The harness persists an open call with its default stop
                // reason; the router wire response remains `function_call`.
                "stop_reason": "end",
                "model": model.id,
                "provider": model.provider
            }));
            let response = &authored.functions[function].response;
            let (content, is_error) = normalize_function_response(response);
            messages.push(json!({
                "role": "function_result",
                "function_call_id": call_id,
                "function_id": function_id,
                "content": content,
                "details": response,
                "is_error": is_error
            }));
        }
        RouterReplyV1::Raw { frames, .. } => {
            let Some(AssistantMessageEvent::Done { message }) = frames.last() else {
                return Ok(());
            };
            messages.push(serde_json::to_value(message)?);
        }
    }
    Ok(())
}

/// Mirrors `harness::trigger::normalize`: the next router generation must
/// match the exact function-result shape the real harness persists.
fn normalize_function_response(response: &Value) -> (Value, bool) {
    let is_error = response
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Value::String(text) = response {
        return (json!([{ "type": "text", "text": text }]), is_error);
    }
    if let Some(content) = response.get("content") {
        if let Ok(blocks) = serde_json::from_value::<Vec<ContentBlock>>(content.clone()) {
            if !blocks.is_empty() {
                return (
                    serde_json::to_value(blocks).expect("content blocks serialize"),
                    is_error,
                );
            }
        }
    }
    let rendered =
        serde_json::to_string(response).unwrap_or_else(|_| "<unserializable>".to_string());
    (json!([{ "type": "text", "text": rendered }]), is_error)
}

fn compile_expectations(
    authored: &IntegrationScenarioV1,
    function_ids: &BTreeMap<String, String>,
    calls: &[CompiledFunctionCall],
    generation_count: usize,
) -> anyhow::Result<Vec<InvariantSpecV1>> {
    let expect = &authored.expect;
    let mut invariants = vec![invariant(
        "send.flags",
        json!({
            "merged": expect.send_flags.merged,
            "queued": expect.send_flags.queued,
            "deduplicated": expect.send_flags.deduplicated
        }),
    )];

    if let Some(counts) = expect.message_counts {
        invariants.push(invariant(
            "transcript.message_counts",
            serde_json::to_value(counts)?,
        ));
    }
    if let Some(text) = &expect.assistant_text {
        invariants.push(invariant(
            "transcript.assistant_text",
            json!({ "text": text }),
        ));
    }
    for result in &expect.function_results {
        let call = calls.iter().find(|call| call.id == result.function_call_id);
        if call.is_none() {
            anyhow::bail!(
                "function result expectation references unknown function call {:?}",
                result.function_call_id
            );
        }
        if let (Some(call), Some(expected_function)) = (call, &result.function) {
            if call.function != *expected_function {
                anyhow::bail!(
                    "function result expectation for {:?} names function {:?}, but the call targets {:?}",
                    result.function_call_id,
                    expected_function,
                    call.function
                );
            }
        }
        let mut parameters = Map::new();
        parameters.insert(
            "function_call_id".to_string(),
            json!(result.function_call_id),
        );
        if let Some(alias) = &result.function {
            parameters.insert(
                "function_id".to_string(),
                json!(resolve_alias(function_ids, alias, "function result")?),
            );
        }
        if let Some(content) = &result.content {
            parameters.insert("content".to_string(), json!(content));
        }
        if let Some(is_error) = result.is_error {
            parameters.insert("is_error".to_string(), json!(is_error));
        }
        invariants.push(InvariantSpecV1 {
            id: "transcript.function_result".to_string(),
            parameters,
        });
    }
    if expect.calls_closed {
        invariants.push(invariant("transcript.calls_closed", json!({})));
    }
    if expect.no_duplicates {
        invariants.push(invariant("transcript.no_duplicates", json!({})));
    }
    invariants.push(invariant(
        "status.terminal",
        json!({
            "status": terminal_status(expect.terminal.status),
            "pending_calls": expect.terminal.pending_calls,
            "queued_messages": expect.terminal.queued_messages
        }),
    ));
    invariants.push(invariant(
        "lifecycle.completed_once",
        json!({
            "allow_identical_duplicates": expect.lifecycle.allow_identical_duplicates
        }),
    ));
    invariants.push(invariant(
        "router.generations_consumed",
        json!({
            "count": generation_count as u64
        }),
    ));

    for call in &expect.calls {
        let mut parameters = Map::new();
        parameters.insert(
            "function_id".to_string(),
            json!(resolve_alias(
                function_ids,
                &call.function,
                "call expectation"
            )?),
        );
        parameters.insert("count".to_string(), json!(call.count));
        if let Some(payload) = &call.payload {
            parameters.insert("payload".to_string(), payload.clone());
        }
        if let Some(payload_subset) = &call.payload_subset {
            parameters.insert("payload_subset".to_string(), payload_subset.clone());
        }
        invariants.push(InvariantSpecV1 {
            id: "target.calls".to_string(),
            parameters,
        });
    }
    if authored.functions.is_empty() {
        invariants.push(invariant(
            "target.calls",
            json!({
                "function_id": format!("{{{{run_id}}}}::{SYNTHETIC_FUNCTION_ALIAS}"),
                "count": 0
            }),
        ));
    }
    Ok(invariants)
}

fn invariant(id: &str, parameters: Value) -> InvariantSpecV1 {
    InvariantSpecV1 {
        id: id.to_string(),
        parameters: parameters
            .as_object()
            .cloned()
            .expect("invariant parameters are objects"),
    }
}

fn resolve_alias<'a>(
    function_ids: &'a BTreeMap<String, String>,
    alias: &str,
    context: &str,
) -> anyhow::Result<&'a str> {
    function_ids
        .get(alias)
        .map(String::as_str)
        .with_context(|| format!("{context} references unknown function alias {alias:?}"))
}

fn terminal_status(status: TerminalStatusV1) -> &'static str {
    match status {
        TerminalStatusV1::Completed => "completed",
        TerminalStatusV1::Failed => "failed",
        TerminalStatusV1::Cancelled => "cancelled",
    }
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

fn validate_placeholders(fixture: &CompiledFixtureV1) -> anyhow::Result<()> {
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
    let base = Placeholders::new(&run_id, &session_id);
    let system_prompt = base.expand_str(&fixture.system_prompt_template)?;
    let digest = crate::canonical::sha256_of_bytes(system_prompt.as_bytes());
    let placeholders = base.with_system_prompt_sha256(&digest);

    let mut scenario = serde_json::to_value(&fixture.scenario)?;
    placeholders.expand_value(&mut scenario)?;
    let mut script = serde_json::to_value(&fixture.script)?;
    placeholders.expand_value(&mut script)?;
    Ok(crate::canonical::canonical_json_pretty(&json!({
        "scenario": scenario,
        "router_script": script,
        "system_prompt": system_prompt
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioTemplateKind {
    Text,
    Function,
    Hook,
    Crash,
}

/// Minimal valid authored scenario used by the non-interactive `init` CLI.
pub fn scenario_template(
    id: &str,
    description: &str,
    kind: ScenarioTemplateKind,
) -> IntegrationScenarioV1 {
    let mut functions = BTreeMap::new();
    let record = ScenarioFunctionV1 {
        description: "Record one integration fixture value.".to_string(),
        request_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        })
        .as_object()
        .cloned()
        .expect("object"),
        response: json!({
            "content": [{ "type": "text", "text": "recorded" }],
            "is_error": false
        }),
        response_delay_ms: None,
        expose: true,
    };
    if kind != ScenarioTemplateKind::Text {
        functions.insert("record".to_string(), record);
    }

    let mut bindings = Vec::new();
    let mut release = None;
    let mut fault = None;
    let mut timeouts = crate::types::scenario::DeadlinesV1::default();
    if kind == ScenarioTemplateKind::Hook {
        functions.insert(
            "hook".to_string(),
            ScenarioFunctionV1 {
                description: "Hold the controlled call for explicit release.".to_string(),
                request_schema: json!({ "type": "object" })
                    .as_object()
                    .cloned()
                    .expect("object"),
                response: json!({ "decision": "hold" }),
                response_delay_ms: None,
                expose: false,
            },
        );
        bindings.push(crate::types::scenario::TriggerBindingSpecV1 {
            trigger: crate::types::scenario::TriggerKindV1::HookPreTrigger,
            function: "hook".to_string(),
            functions: vec!["record".to_string()],
            priority: 10,
        });
        release = Some(crate::types::scenario::ReleaseV1 {
            function_call_id: "call-1".to_string(),
            action: crate::types::scenario::ReleaseActionV1::Execute,
        });
    }
    if kind == ScenarioTemplateKind::Crash {
        functions
            .get_mut("record")
            .expect("crash template has record")
            .response_delay_ms = Some(8_000);
        fault = Some(crate::types::scenario::FaultV1 {
            kind: crate::types::scenario::FaultKind::EngineSigkill,
            function: Some("record".to_string()),
            after_target_calls: 1,
            restart_delay_ms: 1_500,
        });
        timeouts.scenario_ms = 120_000;
    }

    let first_reply = match kind {
        ScenarioTemplateKind::Text => RouterReplyV1::Text {
            text: "fixture complete".to_string(),
            chunks: vec!["fixture ".to_string(), "complete".to_string()],
            usage: None,
        },
        _ => RouterReplyV1::FunctionCall {
            id: None,
            function: "record".to_string(),
            arguments: json!({ "value": "expected" }),
            usage: None,
        },
    };
    let mut generations = vec![crate::types::scenario::ScenarioGenerationV1 {
        reply: first_reply,
        match_overrides: Default::default(),
    }];
    if kind != ScenarioTemplateKind::Text {
        let match_overrides = if matches!(
            kind,
            ScenarioTemplateKind::Hook | ScenarioTemplateKind::Crash
        ) {
            crate::types::scenario::GenerationMatchOverridesV1 {
                request_id: Some(JsonMatcherV1::Regex {
                    pattern: "^t_[0-9a-f]{32}:[0-9]+$".to_string(),
                }),
                system_prompt: Some(JsonMatcherV1::Present),
                messages: Some(JsonMatcherV1::Present),
                tools: Some(JsonMatcherV1::Present),
                ..Default::default()
            }
        } else {
            Default::default()
        };
        generations.push(crate::types::scenario::ScenarioGenerationV1 {
            reply: RouterReplyV1::Text {
                text: "recorded once".to_string(),
                chunks: Vec::new(),
                usage: None,
            },
            match_overrides,
        });
    }

    let expect = if kind == ScenarioTemplateKind::Text {
        ExpectationsV1 {
            message_counts: Some(MessageCountsExpectationV1 {
                user: 1,
                assistant: 1,
                function_result: 0,
            }),
            assistant_text: Some("fixture complete".to_string()),
            ..Default::default()
        }
    } else {
        let function_result = FunctionResultExpectationV1 {
            function_call_id: "call-1".to_string(),
            function: (kind != ScenarioTemplateKind::Crash).then(|| "record".to_string()),
            content: (kind != ScenarioTemplateKind::Crash)
                .then(|| vec![json!({ "type": "text", "text": "recorded" })]),
            is_error: (kind != ScenarioTemplateKind::Crash).then_some(false),
        };
        let mut calls = vec![TargetCallsExpectationV1 {
            function: "record".to_string(),
            count: 1,
            payload: Some(json!({ "value": "expected" })),
            payload_subset: None,
        }];
        if kind == ScenarioTemplateKind::Hook {
            calls.push(TargetCallsExpectationV1 {
                function: "hook".to_string(),
                count: 1,
                payload: None,
                payload_subset: None,
            });
        }
        ExpectationsV1 {
            message_counts: Some(MessageCountsExpectationV1 {
                user: 1,
                assistant: 2,
                function_result: 1,
            }),
            assistant_text: Some("recorded once".to_string()),
            function_results: vec![function_result],
            calls_closed: true,
            calls,
            ..Default::default()
        }
    };

    IntegrationScenarioV1 {
        schema_version: SchemaVersion1::V1,
        id: id.to_string(),
        description: description.to_string(),
        quarantine: false,
        send: crate::types::scenario::ScenarioSendV1 {
            message: if kind == ScenarioTemplateKind::Text {
                "Return the fixture phrase.".to_string()
            } else {
                "Call the recorder once.".to_string()
            },
            allow: None,
            idempotency_key: None,
        },
        functions,
        router: crate::types::scenario::ScenarioRouterV1 {
            model: None,
            generations,
        },
        bindings,
        release,
        fault,
        timeouts,
        expect,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Placeholders {
    values: BTreeMap<&'static str, String>,
}

impl Placeholders {
    pub fn new(run_id: &str, session_id: &str) -> Self {
        let mut values = BTreeMap::new();
        values.insert("run_id", run_id.to_string());
        values.insert("session_id", session_id.to_string());
        Self { values }
    }

    pub fn with_system_prompt_sha256(mut self, digest: &str) -> Self {
        self.values
            .insert("system_prompt_sha256", digest.to_string());
        self
    }

    pub fn expand_str(&self, text: &str) -> anyhow::Result<String> {
        let mut out = text.to_string();
        for (key, value) in &self.values {
            out = out.replace(&format!("{{{{{key}}}}}"), value);
        }
        if let Some(start) = out.find("{{") {
            let tail: String = out[start..].chars().take(40).collect();
            anyhow::bail!("unexpanded placeholder near {tail:?}");
        }
        Ok(out)
    }

    pub fn expand_value(&self, value: &mut Value) -> anyhow::Result<()> {
        match value {
            Value::String(s) => {
                *s = self.expand_str(s)?;
            }
            Value::Array(items) => {
                for item in items {
                    self.expand_value(item)?;
                }
            }
            Value::Object(map) => {
                let needs_key_rewrite = map.keys().any(|k| k.contains("{{"));
                if needs_key_rewrite {
                    let old = std::mem::take(map);
                    for (k, mut v) in old {
                        self.expand_value(&mut v)?;
                        map.insert(self.expand_str(&k)?, v);
                    }
                } else {
                    for (_, v) in map.iter_mut() {
                        self.expand_value(v)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::scenario::{ScenarioGenerationV1, ScenarioRouterV1, ScenarioSendV1};

    fn minimal(reply: RouterReplyV1) -> IntegrationScenarioV1 {
        IntegrationScenarioV1 {
            schema_version: SchemaVersion1::V1,
            id: "C-E2E-T".to_string(),
            description: "A focused compiler fixture.".to_string(),
            quarantine: false,
            send: ScenarioSendV1 {
                message: "hello".to_string(),
                allow: None,
                idempotency_key: None,
            },
            functions: BTreeMap::new(),
            router: ScenarioRouterV1 {
                model: None,
                generations: vec![ScenarioGenerationV1 {
                    reply,
                    match_overrides: Default::default(),
                }],
            },
            bindings: Vec::new(),
            release: None,
            fault: None,
            timeouts: Default::default(),
            expect: Default::default(),
        }
    }

    #[test]
    fn text_reply_compiles_stream_and_common_defaults() {
        let authored = minimal(RouterReplyV1::Text {
            text: "hello".to_string(),
            chunks: vec!["hel".to_string(), "lo".to_string()],
            usage: None,
        });
        let compiled = compile_scenario(&authored, "base\n").unwrap();
        assert_eq!(compiled.script.generations[0].frames.len(), 7);
        assert_eq!(
            compiled.scenario.send["options"]["functions"]["allow"],
            json!([])
        );
        assert_eq!(compiled.scenario.deadlines.teardown_ms, 15_000);
        assert!(compiled
            .scenario
            .invariants
            .iter()
            .any(|i| i.id == "target.calls" && i.parameters["count"] == 0));
        assert!(compiled.system_prompt_template.ends_with(
            "Function dispatch is entirely disabled this turn — do not call any function."
        ));
    }

    #[test]
    fn mismatched_chunks_and_unknown_aliases_fail_before_runtime() {
        let authored = minimal(RouterReplyV1::Text {
            text: "hello".to_string(),
            chunks: vec!["wrong".to_string()],
            usage: None,
        });
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("chunks concatenate")
        );

        let mut authored = minimal(RouterReplyV1::FunctionCall {
            id: None,
            function: "missing".to_string(),
            arguments: json!({}),
            usage: None,
        });
        authored.send.allow = Some(vec![]);
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("unknown alias")
        );
    }

    #[test]
    fn compile_rejects_unknown_placeholders_and_unsafe_ids() {
        let mut authored = minimal(RouterReplyV1::Text {
            text: "hello".to_string(),
            chunks: Vec::new(),
            usage: None,
        });
        authored.send.message = "{{unknown}}".to_string();
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("unexpanded placeholder")
        );

        authored.send.message = "hello".to_string();
        authored.id = "../../escape".to_string();
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("scenario id")
        );
    }

    #[test]
    fn function_call_defaults_are_ordered_by_call_and_validated() {
        let mut authored = scenario_template(
            "C-E2E-CALLS",
            "Validate generated function call ids.",
            ScenarioTemplateKind::Function,
        );
        authored.router.generations.insert(
            0,
            ScenarioGenerationV1 {
                reply: RouterReplyV1::Text {
                    text: "before call".to_string(),
                    chunks: Vec::new(),
                    usage: None,
                },
                match_overrides: Default::default(),
            },
        );
        let compiled = compile_scenario(&authored, "base").unwrap();
        let messages = match &compiled.script.generations[2].match_.messages {
            JsonMatcherV1::Exact { expected, .. } => expected.as_array().unwrap(),
            other => panic!("expected exact history, got {other:?}"),
        };
        assert_eq!(messages[2]["content"][0]["id"], "call-1");
        assert_eq!(messages[3]["function_call_id"], "call-1");

        let RouterReplyV1::FunctionCall { id, .. } = &mut authored.router.generations[1].reply
        else {
            unreachable!()
        };
        *id = Some(String::new());
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("must not be empty")
        );
    }

    #[test]
    fn tools_are_canonicalized_and_function_contracts_validate() {
        let mut authored = scenario_template(
            "C-E2E-TOOLS",
            "Validate tool ordering and arguments.",
            ScenarioTemplateKind::Function,
        );
        let record = authored.functions["record"].clone();
        authored
            .functions
            .insert("zeta".to_string(), record.clone());
        authored.functions.insert("alpha".to_string(), record);
        authored.send.allow = Some(vec![
            "zeta".to_string(),
            "record".to_string(),
            "alpha".to_string(),
        ]);
        let compiled = compile_scenario(&authored, "base").unwrap();
        let allowed = compiled.scenario.send["options"]["functions"]["allow"]
            .as_array()
            .unwrap();
        assert_eq!(
            allowed,
            &[
                json!("{{run_id}}::alpha"),
                json!("{{run_id}}::record"),
                json!("{{run_id}}::zeta")
            ]
        );
        let JsonMatcherV1::Exact { expected, .. } = &compiled.script.generations[0].match_.tools
        else {
            panic!("tools must use an exact matcher");
        };
        assert_eq!(
            expected
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "{{run_id}}::alpha",
                "{{run_id}}::record",
                "{{run_id}}::zeta"
            ]
        );

        let RouterReplyV1::FunctionCall { arguments, .. } =
            &mut authored.router.generations[0].reply
        else {
            unreachable!()
        };
        *arguments = json!({});
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("do not match request_schema")
        );

        authored.functions.get_mut("record").unwrap().request_schema =
            json!({ "type": "not-a-json-schema-type" })
                .as_object()
                .unwrap()
                .clone();
        assert!(
            format!("{:#}", compile_scenario(&authored, "base").unwrap_err())
                .contains("invalid request_schema")
        );
    }

    #[test]
    fn function_response_history_matches_harness_normalization() {
        let mut authored = scenario_template(
            "C-E2E-RESPONSE",
            "Validate normalized function results.",
            ScenarioTemplateKind::Function,
        );
        authored.functions.get_mut("record").unwrap().response = json!("ok");
        let compiled = compile_scenario(&authored, "base").unwrap();
        let JsonMatcherV1::Exact { expected, .. } = &compiled.script.generations[1].match_.messages
        else {
            panic!("function history must be exact");
        };
        assert_eq!(
            expected[2]["content"],
            json!([{ "type": "text", "text": "ok" }])
        );

        authored.functions.get_mut("record").unwrap().response = json!({ "value": 1 });
        let compiled = compile_scenario(&authored, "base").unwrap();
        let JsonMatcherV1::Exact { expected, .. } = &compiled.script.generations[1].match_.messages
        else {
            unreachable!()
        };
        assert_eq!(
            expected[2]["content"],
            json!([{ "type": "text", "text": "{\"value\":1}" }])
        );
    }

    #[test]
    fn bindings_release_and_fault_fail_fast_when_incoherent() {
        let mut hook = scenario_template(
            "C-E2E-HOOK",
            "Validate hook relationships.",
            ScenarioTemplateKind::Hook,
        );
        hook.bindings[0].functions.clear();
        assert!(
            format!("{:#}", compile_scenario(&hook, "base").unwrap_err())
                .contains("select at least one")
        );

        hook.bindings[0].functions = vec!["record".to_string()];
        hook.functions.get_mut("hook").unwrap().response = json!({ "decision": "continue" });
        assert!(
            format!("{:#}", compile_scenario(&hook, "base").unwrap_err())
                .contains("requires a selected hook with decision: hold")
        );

        let mut crash = scenario_template(
            "C-E2E-CRASH",
            "Validate fault relationships.",
            ScenarioTemplateKind::Crash,
        );
        crash.functions.get_mut("record").unwrap().response_delay_ms = None;
        assert!(
            format!("{:#}", compile_scenario(&crash, "base").unwrap_err())
                .contains("response_delay_ms > 0")
        );

        crash.functions.get_mut("record").unwrap().response_delay_ms = Some(1);
        crash.fault.as_mut().unwrap().after_target_calls = 0;
        assert!(
            format!("{:#}", compile_scenario(&crash, "base").unwrap_err())
                .contains("after_target_calls must be greater than zero")
        );
    }

    #[test]
    fn templates_compile_and_render_deterministically() {
        for kind in [
            ScenarioTemplateKind::Text,
            ScenarioTemplateKind::Function,
            ScenarioTemplateKind::Hook,
            ScenarioTemplateKind::Crash,
        ] {
            let authored = scenario_template("C-E2E-NEW", "A generated scenario.", kind);
            let yaml = render_authored_yaml(&authored).unwrap();
            let reparsed: IntegrationScenarioV1 = serde_yaml::from_str(&yaml).unwrap();
            let fixture = compile_scenario(&reparsed, "base\n").unwrap();
            assert_eq!(
                render_compiled(&fixture).unwrap(),
                render_compiled(&fixture).unwrap()
            );
            assert!(fixture
                .scenario
                .invariants
                .iter()
                .any(|invariant| invariant.id == "transcript.message_counts"));
            if kind != ScenarioTemplateKind::Text {
                assert!(fixture
                    .scenario
                    .invariants
                    .iter()
                    .any(|invariant| invariant.id == "target.calls"));
            }
            if matches!(
                kind,
                ScenarioTemplateKind::Hook | ScenarioTemplateKind::Crash
            ) {
                assert!(matches!(
                    fixture.script.generations[1].match_.messages,
                    JsonMatcherV1::Present
                ));
            }
        }
    }

    #[test]
    fn expands_strings_keys_and_rejects_unknown_tokens() {
        let p = Placeholders::new("r1", "s_abc");
        let mut v = json!({
            "idempotency_key": "{{run_id}}:streamed-text",
            "session_id": "{{session_id}}",
            "{{run_id}}::record": { "count": 1 }
        });
        p.expand_value(&mut v).unwrap();
        assert_eq!(v["idempotency_key"], "r1:streamed-text");
        assert_eq!(v["session_id"], "s_abc");
        assert!(v.get("r1::record").is_some());

        let mut bad = json!("{{unknown_token}}");
        assert!(p.expand_value(&mut bad).is_err());
    }
}
