//! The function-trigger pipeline (harness.md § Functions / §
//! `harness::function::trigger`): the fail-closed allow/deny globs first,
//! then the target invocation, then result normalisation. Discovery results
//! (`engine::functions::list` / `info`) are post-filtered through the same
//! globs so the model only discovers what it can call.
//!
//! P1 covers the glob policy + target + normalisation. The `pre_trigger` /
//! `post_trigger` hook chain and `pending` deferral layer on in later phases.

use std::collections::{BTreeMap, HashMap};

use jsonschema::JSONSchema;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::clients::EngineClient;
use crate::policy::CompiledPolicy;
use crate::types::content::ContentBlock;
use crate::types::turn::FunctionContractLedgerEntry;

/// A normalised function result ready to become a `function_result` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultData {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub details: Value,
}

/// Discard ledger rows whose exact full source is not the only model-facing
/// result for its call id.
pub(crate) fn retain_visible_contract_sources(
    ledger: &mut BTreeMap<String, FunctionContractLedgerEntry>,
    messages: &[Value],
) {
    ledger.retain(|_, source| {
        let eligible = unique_visible_result(messages, &source.source_function_call_id)
            .is_some_and(|(function_id, content)| {
                function_id == Some("engine::functions::info")
                    && content.and_then(digest_value).as_deref()
                        == Some(source.source_content_digest.as_str())
            });
        source.eligible = eligible;
        eligible
    });
}

/// Record a successful function-result append. Reusing a source call id
/// invalidates every row that named it; such an ambiguous id cannot become a
/// replacement source in the same append.
pub(crate) fn apply_contract_updates_after_append(
    ledger: &mut BTreeMap<String, FunctionContractLedgerEntry>,
    call_id: &str,
    updates: Vec<(String, FunctionContractLedgerEntry)>,
) {
    let reused_source = ledger
        .values()
        .any(|source| source.source_function_call_id == call_id);
    ledger.retain(|_, source| source.source_function_call_id != call_id);
    if !reused_source {
        ledger.extend(updates);
    }
}

fn unique_visible_result<'a>(
    messages: &'a [Value],
    call_id: &str,
) -> Option<(Option<&'a str>, Option<&'a Value>)> {
    let mut found = None;
    let mut call_seen = false;
    for message in messages {
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match block.get("type").and_then(Value::as_str) {
                Some("function_call")
                    if block.get("id").and_then(Value::as_str) == Some(call_id) =>
                {
                    let function_id = block.get("function_id").and_then(Value::as_str);
                    let wrapped_target = block
                        .get("arguments")
                        .and_then(|arguments| arguments.get("function"))
                        .and_then(Value::as_str);
                    let is_info = function_id == Some("engine::functions::info")
                        || (function_id == Some("agent_trigger")
                            && wrapped_target == Some("engine::functions::info"));
                    if call_seen || !is_info {
                        return None;
                    }
                    call_seen = true;
                }
                Some("function_result")
                    if block.get("function_call_id").and_then(Value::as_str) == Some(call_id) =>
                {
                    if found.is_some() {
                        return None;
                    }
                    // Inline provider-style results do not carry the source
                    // function id, so they can never validate a top-level source.
                    found = Some((None, block.get("content")));
                }
                _ => {}
            }
        }
        if message.get("role").and_then(Value::as_str) == Some("function_result")
            && message.get("function_call_id").and_then(Value::as_str) == Some(call_id)
        {
            if found.is_some() {
                return None;
            }
            found = Some((
                message.get("function_id").and_then(Value::as_str),
                message.get("content"),
            ));
        }
    }
    found
}

/// Compact unchanged, successful `engine::functions::info` contracts while
/// preserving the raw result in `details`. Returned ledger updates are applied
/// only after the caller successfully appends the result.
pub(crate) fn prepare_info_result(
    call_id: &str,
    arguments: &Value,
    data: &ResultData,
    ledger: &BTreeMap<String, FunctionContractLedgerEntry>,
    hook_unchanged: bool,
) -> (ResultData, Vec<(String, FunctionContractLedgerEntry)>) {
    if data.is_error || !hook_unchanged || normalize(&data.details).0 != data.content {
        return (data.clone(), Vec::new());
    }

    let requested_single = arguments.get("function_id").and_then(Value::as_str);
    let requested_batch = arguments.get("function_ids").and_then(Value::as_array);
    let mut display = data.details.clone();
    let mut candidates = Vec::new();

    if let (Some(requested), Some(item)) = (requested_single, display.as_object()) {
        if valid_contract_id(&Value::Object(item.clone())) == Some(requested) {
            candidates.push((None, requested.to_string()));
        }
    } else if let (Some(requested), Some(items)) = (
        requested_batch,
        display.get("functions").and_then(Value::as_array),
    ) {
        let requested_counts = counts(requested.iter().filter_map(Value::as_str));
        let result_counts = counts(items.iter().filter_map(|item| {
            item.get("function_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
        }));
        for (index, item) in items.iter().enumerate() {
            let Some(id) = valid_contract_id(item) else {
                continue;
            };
            if requested_counts.get(id) == Some(&1) && result_counts.get(id) == Some(&1) {
                candidates.push((Some(index), id.to_string()));
            }
        }
    } else {
        return (data.clone(), Vec::new());
    }

    let mut full = Vec::new();
    let mut compacted = false;
    for (index, function_id) in candidates {
        let contract = match index {
            Some(index) => &data.details["functions"][index],
            None => &data.details,
        };
        let Some(contract_digest) = digest_value(contract) else {
            continue;
        };
        match ledger.get(&function_id) {
            Some(source)
                if source.eligible
                    && source.contract_digest == contract_digest
                    && source.source_function_call_id != call_id =>
            {
                let marker = json!({
                    "function_id": function_id,
                    "contract_status": "unchanged_in_context",
                    "source_function_call_id": source.source_function_call_id,
                });
                match index {
                    Some(index) => display["functions"][index] = marker,
                    None => display = marker,
                }
                compacted = true;
            }
            Some(source) if source.source_function_call_id == call_id => {}
            _ => full.push((function_id, contract_digest)),
        }
    }

    // Response-side schemas carry no calling contract — the model reads
    // results, it never constructs them — and they are ~40% of a typical
    // contract's bytes. Strip them from the model-visible copy only; the raw
    // contract stays in `details`, and the ledger digest is taken over the
    // raw contract, so unchanged-detection is unaffected.
    let mut stripped = false;
    match display.get_mut("functions").and_then(Value::as_array_mut) {
        Some(items) => {
            for item in items {
                stripped |= strip_response_schemas(item);
            }
        }
        None => stripped |= strip_response_schemas(&mut display),
    }

    let mut prepared = data.clone();
    if compacted || stripped {
        prepared.content = normalize(&display).0;
    }
    let Some(source_content_digest) = digest_content(&prepared.content) else {
        return (prepared, Vec::new());
    };
    let updates = full
        .into_iter()
        .map(|(function_id, contract_digest)| {
            (
                function_id,
                FunctionContractLedgerEntry {
                    contract_digest,
                    source_function_call_id: call_id.to_string(),
                    source_content_digest: source_content_digest.clone(),
                    eligible: false,
                },
            )
        })
        .collect();
    (prepared, updates)
}

/// Drop the response-side schema keys from one model-visible contract item.
/// A dedupe marker or error item has none of these — the call is a no-op.
fn strip_response_schemas(item: &mut Value) -> bool {
    let Some(object) = item.as_object_mut() else {
        return false;
    };
    let mut stripped = false;
    for key in ["response_schema", "response_format"] {
        stripped |= object.remove(key).is_some();
    }
    stripped
}

fn valid_contract_id(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    if object.contains_key("error") || !contract_schemas_compile(object) {
        return None;
    }
    object
        .get("function_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
}

fn contract_schemas_compile(object: &serde_json::Map<String, Value>) -> bool {
    ["parameters", "request_format", "request_schema"]
        .iter()
        .find_map(|key| object.get(*key))
        .is_some_and(|schema| JSONSchema::compile(schema).is_ok())
        && ["response_format", "response_schema"]
            .iter()
            .find_map(|key| object.get(*key))
            .is_some_and(|schema| JSONSchema::compile(schema).is_ok())
}

fn counts<'a>(ids: impl Iterator<Item = &'a str>) -> HashMap<&'a str, usize> {
    let mut counts = HashMap::new();
    for id in ids {
        *counts.entry(id).or_default() += 1;
    }
    counts
}

fn digest_content(content: &[ContentBlock]) -> Option<String> {
    serde_json::to_value(content)
        .ok()
        .as_ref()
        .and_then(digest_value)
}

fn digest_value(value: &Value) -> Option<String> {
    Some(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).ok()?)
    ))
}

/// The outcome of triggering one call.
pub enum TriggerResult {
    /// A settled result (success, policy denial, or target error).
    Result(ResultData),
    /// Deferred — the result arrives later via `harness::function::resolve`.
    #[allow(dead_code)]
    Pending(PendingInfo),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct PendingInfo {
    pub pending_timeout_ms: Option<u64>,
    pub held_by: Option<String>,
    /// Hook holds only: the arguments as mutated by the chain up to the hold,
    /// checkpointed so a release executes the mutated call (issue #506).
    pub held_arguments: Option<Value>,
    pub child_session_id: Option<String>,
    pub child_turn_id: Option<String>,
}

/// Run the trigger pipeline for one call. `function_id` is the unwrapped
/// target; `arguments` the target payload.
pub async fn trigger_call(
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
) -> TriggerResult {
    // Fail-closed glob policy — structural and final.
    if !policy.allows(function_id) {
        return TriggerResult::Result(denied_result(function_id));
    }

    TriggerResult::Result(invoke_target(engine, policy, function_id, arguments).await)
}

/// Invoke the target and normalise its result — WITHOUT the policy gate (the
/// caller checks `policy.allows` first) or the hook chain. `policy` is still
/// used to post-filter runtime discovery results. Used by the loop and the
/// hook-held release path after `pre_trigger` has run.
pub async fn invoke_target(
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
) -> ResultData {
    match engine.dispatch(function_id, arguments.clone()).await {
        Ok(mut value) => {
            if function_id == "engine::functions::list" {
                post_filter_discovery(&mut value, policy);
            } else if function_id == "engine::functions::info" {
                post_filter_info(&mut value, policy);
            }
            normalized_result(value)
        }
        Err(e) => invocation_error_result(e.code, e.message),
    }
}

/// Apply the same function-result normalization to a locally intercepted call
/// as [`invoke_target`] applies to an engine-dispatched return value.
pub(crate) fn normalized_result(value: Value) -> ResultData {
    let (content, is_error) = normalize(&value);
    ResultData {
        content,
        is_error,
        details: value,
    }
}

pub(crate) fn invocation_error_result(code: Option<String>, message: String) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(message.clone())],
        is_error: true,
        details: json!({ "error": { "code": code, "message": message } }),
    }
}

/// Post-filter an `engine::functions::info` result — the flat single-id
/// detail (blanked to `null` when not callable, as ever) or the engine's
/// `function_ids` batch envelope. Batch entries the agent cannot dispatch are
/// masked to the same `{ function_id, error: "not available" }` stub the
/// engine uses for unknown ids — denied stays indistinguishable from
/// nonexistent, and (unlike the list filter) entries are never dropped, so
/// the model can pair every requested id with an outcome.
fn post_filter_info(value: &mut Value, policy: &CompiledPolicy) {
    let keep = |id: &str| policy.allows(id);
    if let Some(items) = value.get_mut("functions").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            let id = item
                .get("function_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !keep(&id) {
                *item = json!({ "function_id": id, "error": "not available" });
            } else {
                overlay_control_contract(item, &id);
            }
        }
    } else if let Some(id) = value
        .get("function_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
    {
        let id = id.to_string();
        if !keep(&id) {
            *value = Value::Null;
        } else {
            overlay_control_contract(value, &id);
        }
    }
}

/// `engine::register_trigger` / `engine::unregister_trigger` are intercepted
/// in-turn, so the engine's own registration (raw: `function_id` required, no
/// `once`/`lifecycle`/`conditions`) is NOT the contract an agent calls.
/// Discovery must describe the intercept, or an agent that reads
/// `functions::info` "learns" its tool schema is wrong.
fn overlay_control_contract(item: &mut Value, id: &str) {
    let Some((description, schema)) = crate::functions::subscribe::control_contract(id) else {
        return;
    };
    if let Some(map) = item.as_object_mut() {
        map.insert("description".into(), Value::String(description.into()));
        if map.contains_key("request_schema") {
            map.insert("request_schema".into(), schema);
        }
    }
}

/// The `is_error` result for a policy denial (no allow match or a deny match).
pub fn denied_result(function_id: &str) -> ResultData {
    let msg = format!(
        "function {function_id} is not permitted by this agent's dispatch policy (no allow-glob \
         match or a deny-glob match)"
    );
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": "policy_denied", "function_id": function_id, "message": msg }),
    }
}

/// The `is_error` result for an `agent_trigger` call with no resolvable
/// target — arguments were empty, null, or unparseable (local models emit
/// malformed JSON args). Dispatching the wrapper name to the engine would
/// only return a cryptic `function_not_found: agent_trigger`.
/// Char-safe ≤200-char preview of raw arguments for error messages —
/// model-emitted JSON serializes with literal UTF-8, and a byte-indexed
/// `String::truncate` panics mid-char on CJK/emoji payloads.
fn arguments_preview(arguments: &Value) -> String {
    let s = arguments.to_string();
    match s.char_indices().nth(200) {
        Some((i, _)) => s[..i].to_string(),
        None => s,
    }
}

pub fn wrapper_without_target_result(arguments: &Value) -> ResultData {
    let got = arguments_preview(arguments);
    let msg = format!(
        "agent_trigger was called without a usable target (arguments were {got}); expected \
         {{\"function\": \"<function id>\", \"payload\": {{...}}}}. Re-issue the call with the \
         target function id."
    );
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": "agent_trigger_no_target", "message": msg }),
    }
}

/// Salvaged (`_partial`/`_raw`) or non-object arguments must never reach dispatch.
pub fn arguments_degraded(arguments: &Value) -> bool {
    match arguments {
        Value::Object(map) => map.contains_key("_partial") || map.contains_key("_raw"),
        _ => true,
    }
}

/// Provider-degraded arguments (a stream that died or hit max_tokens
/// mid-args, salvaged to a `"_partial": true` prefix or a raw `{"_raw": …}`
/// evidence object) must never execute: the salvage preserves evidence for
/// the transcript, not intent. Teachable local failure, mirroring
/// [`wrapper_without_target_result`].
pub fn truncated_arguments_result(function_id: &str, arguments: &Value) -> ResultData {
    let got = arguments_preview(arguments);
    let msg = format!(
        "the arguments for {function_id} arrived truncated (the model stream ended \
         mid-arguments; received {got}). The call was NOT executed — re-issue it with \
         complete arguments."
    );
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": "arguments_truncated", "message": msg }),
    }
}

/// Normalise an arbitrary function return into content blocks. `details`
/// always carries the raw value; content is a string render, an explicit
/// `content` block array, or a compact JSON fallback.
fn normalize(value: &Value) -> (Vec<ContentBlock>, bool) {
    let is_error = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if let Value::String(s) = value {
        return (vec![ContentBlock::text(s.clone())], is_error);
    }
    if let Some(blocks) = value.get("content") {
        if let Ok(parsed) = serde_json::from_value::<Vec<ContentBlock>>(blocks.clone()) {
            if !parsed.is_empty() {
                return (parsed, is_error);
            }
        }
    }
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    (vec![ContentBlock::text(rendered)], is_error)
}

/// Drop functions the agent cannot call from an `engine::functions::list`
/// result so the model only sees what its policy permits (a list is a
/// filtered view — dropping is correct there; info results go through
/// [`post_filter_info`] instead, which masks rather than drops).
fn post_filter_discovery(value: &mut Value, policy: &CompiledPolicy) {
    let keep = |id: &str| policy.allows(id);
    if let Some(arr) = value.get_mut("functions").and_then(Value::as_array_mut) {
        arr.retain(|f| {
            f.get("function_id")
                .or_else(|| f.get("id"))
                .or_else(|| f.get("name"))
                .and_then(Value::as_str)
                .map(keep)
                .unwrap_or(false)
        });
        for item in arr.iter_mut() {
            let id = item
                .get("function_id")
                .or_else(|| item.get("id"))
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            overlay_control_contract(item, &id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::turn::FunctionPolicy;
    use std::collections::BTreeMap;

    fn pol(allow: &[&str]) -> CompiledPolicy {
        CompiledPolicy::from(Some(&FunctionPolicy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: vec![],
            expose: Default::default(),
        }))
    }

    #[test]
    fn discovery_reports_the_intercepted_registration_contract() {
        // The raw engine row for register_trigger would claim `function_id`
        // is required and know nothing of once/lifecycle/conditions — the
        // overlay swaps in the intercept's contract.
        let policy = pol(&["*"]);
        let mut info = serde_json::json!({
            "function_id": "engine::register_trigger",
            "description": "Register a trigger that fires `function_id` directly",
            "request_schema": { "required": ["function_id", "trigger_type"] }
        });
        post_filter_info(&mut info, &policy);
        assert!(info["description"]
            .as_str()
            .unwrap()
            .contains("omit function_id"));
        let schema = serde_json::to_string(&info["request_schema"]).unwrap();
        assert!(
            schema.contains("lifecycle"),
            "intercept schema expected: {schema}"
        );

        let mut list = serde_json::json!({ "functions": [
            { "function_id": "engine::register_trigger", "description": "raw" },
            { "function_id": "state::get", "description": "Get a value from state" }
        ]});
        post_filter_discovery(&mut list, &policy);
        assert!(list["functions"][0]["description"]
            .as_str()
            .unwrap()
            .contains("omit function_id"));
        assert_eq!(
            list["functions"][1]["description"],
            "Get a value from state"
        );
    }

    #[test]
    fn arguments_preview_is_char_safe_on_multibyte_payloads() {
        // Byte 200 lands mid-emoji: the old byte-indexed String::truncate
        // panicked here on model-emitted CJK/emoji args.
        let args = json!({ "text": "🎉".repeat(100) });
        let preview = arguments_preview(&args);
        assert!(preview.chars().count() <= 200);
        assert!(args.to_string().starts_with(&preview));
        // Both teachable results render without panicking.
        assert!(wrapper_without_target_result(&args).is_error);
        assert!(truncated_arguments_result("state::set", &args).is_error);
        // Short args pass through whole.
        assert_eq!(arguments_preview(&json!({"a": 1})), r#"{"a":1}"#);
    }

    #[test]
    fn degraded_arguments_cover_markers_and_non_objects() {
        assert!(arguments_degraded(
            &json!({ "_partial": true, "path": "/tmp" })
        ));
        assert!(arguments_degraded(&json!({ "_raw": "{\"path\":\"/tm" })));
        assert!(arguments_degraded(&Value::Null));
        assert!(arguments_degraded(&json!("{}")));
        assert!(arguments_degraded(&json!([1, 2])));
        assert!(!arguments_degraded(&json!({})));
        assert!(!arguments_degraded(&json!({ "path": "/tmp" })));
    }

    /// The model-visible rendering of a full contract result: `details`
    /// minus the response-side schema keys prepare_info_result strips.
    fn stripped_content(details: &Value) -> Vec<ContentBlock> {
        let mut display = details.clone();
        match display.get_mut("functions").and_then(Value::as_array_mut) {
            Some(items) => {
                for item in items {
                    strip_response_schemas(item);
                }
            }
            None => {
                strip_response_schemas(&mut display);
            }
        }
        normalize(&display).0
    }

    #[test]
    fn response_schemas_are_stripped_from_the_model_visible_copy_only() {
        let details = json!({ "functions": [{
            "function_id": "worker::function",
            "description": "Does work",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" },
            "response_format": { "type": "object" }
        }]});
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details: details.clone(),
        };
        let (prepared, updates) = prepare_info_result(
            "call_1",
            &json!({ "function_ids": ["worker::function"] }),
            &data,
            &BTreeMap::new(),
            true,
        );
        let rendered: Value =
            serde_json::from_str(&ContentBlock::join_text(&prepared.content)).unwrap();
        assert!(rendered["functions"][0].get("response_schema").is_none());
        assert!(rendered["functions"][0].get("response_format").is_none());
        assert_eq!(
            rendered["functions"][0]["request_schema"],
            json!({ "type": "object" })
        );
        assert_eq!(prepared.details, details, "raw contract stays in details");
        // The ledger digest is over the RAW contract, so a repeat fetch of the
        // same contract still dedupes to a marker — the stripped transcript
        // copy is what the visibility check digests.
        let mut ledger: BTreeMap<_, _> = updates.into_iter().collect();
        let source = json!({
            "role": "function_result",
            "function_call_id": "call_1",
            "function_id": "engine::functions::info",
            "content": serde_json::to_value(&prepared.content).unwrap()
        });
        retain_visible_contract_sources(&mut ledger, &[source]);
        let (second, _) = prepare_info_result(
            "call_2",
            &json!({ "function_ids": ["worker::function"] }),
            &data,
            &ledger,
            true,
        );
        let rendered: Value =
            serde_json::from_str(&ContentBlock::join_text(&second.content)).unwrap();
        assert_eq!(
            rendered["functions"][0]["contract_status"],
            "unchanged_in_context"
        );
    }

    #[test]
    fn unchanged_info_contract_reuses_an_exact_model_visible_source() {
        let details = json!({
            "function_id": "worker::function",
            "description": "Does work",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
        });
        let data = ResultData {
            content: vec![ContentBlock::text(details.to_string())],
            is_error: false,
            details,
        };
        let mut ledger = BTreeMap::new();

        let (first, updates) = prepare_info_result(
            "call_123",
            &json!({ "function_id": "worker::function" }),
            &data,
            &ledger,
            true,
        );
        assert_eq!(
            first.content,
            stripped_content(&data.details),
            "the first result keeps everything but response schemas"
        );
        ledger.extend(updates);

        let source = json!({
            "role": "function_result",
            "function_call_id": "call_123",
            "function_id": "engine::functions::info",
            "content": serde_json::to_value(&first.content).unwrap(),
            "details": first.details,
            "is_error": false,
            "timestamp": 1
        });
        retain_visible_contract_sources(&mut ledger, &[source]);

        let (second, updates) = prepare_info_result(
            "call_456",
            &json!({ "function_id": "worker::function" }),
            &data,
            &ledger,
            true,
        );
        assert_eq!(
            second.content,
            vec![ContentBlock::text(
                json!({
                    "function_id": "worker::function",
                    "contract_status": "unchanged_in_context",
                    "source_function_call_id": "call_123"
                })
                .to_string()
            )]
        );
        assert_eq!(second.details, data.details, "details stay exact");
        assert!(updates.is_empty(), "markers never replace the full source");
    }

    #[test]
    fn info_batch_preserves_order_cardinality_unknowns_and_details() {
        let details = json!({ "functions": [
            {
                "function_id": "a::one",
                "request_schema": { "type": "object" },
                "response_schema": { "type": "string" }
            },
            { "function_id": "missing::fn", "error": "not available" },
            {
                "function_id": "b::two",
                "request_schema": { "type": "object" },
                "response_schema": { "type": "number" }
            }
        ]});
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details: details.clone(),
        };
        let args = json!({ "function_ids": ["a::one", "missing::fn", "b::two"] });
        let (first, updates) =
            prepare_info_result("batch-full", &args, &data, &BTreeMap::new(), true);
        let mut ledger: BTreeMap<_, _> = updates.into_iter().collect();
        let source = json!({
            "role": "function_result",
            "function_call_id": "batch-full",
            "function_id": "engine::functions::info",
            "content": serde_json::to_value(&first.content).unwrap()
        });
        retain_visible_contract_sources(&mut ledger, &[source]);

        let (prepared, updates) = prepare_info_result("batch-repeat", &args, &data, &ledger, true);
        let rendered: Value = serde_json::from_str(&ContentBlock::join_text(&prepared.content))
            .expect("prepared batch content is JSON");
        let functions = rendered["functions"].as_array().unwrap();
        assert_eq!(functions.len(), 3);
        assert_eq!(functions[0]["contract_status"], "unchanged_in_context");
        assert_eq!(functions[1], details["functions"][1]);
        assert_eq!(functions[2]["contract_status"], "unchanged_in_context");
        assert_eq!(prepared.details, details);
        assert!(updates.is_empty());
    }

    #[test]
    fn ambiguous_or_unverifiable_info_sources_stay_full() {
        let details = json!({
            "function_id": "worker::function",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
        });
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details,
        };
        let args = json!({ "function_id": "worker::function" });
        let (first, updates) = prepare_info_result("source", &args, &data, &BTreeMap::new(), true);
        let mut ledger: BTreeMap<_, _> = updates.into_iter().collect();
        let exact = json!({
            "role": "function_result",
            "function_call_id": "source",
            "function_id": "engine::functions::info",
            "content": serde_json::to_value(&first.content).unwrap()
        });
        let mutated = json!({
            "role": "user",
            "content": [{
                "type": "function_result",
                "function_call_id": "source",
                "content": [{ "type": "text", "text": "changed by a hook" }]
            }]
        });
        retain_visible_contract_sources(&mut ledger, &[exact, mutated]);
        assert!(ledger.is_empty(), "the exact last result must match");

        let (without_source, updates) = prepare_info_result("repeat", &args, &data, &ledger, true);
        assert_eq!(without_source.content, stripped_content(&data.details));
        assert_eq!(updates.len(), 1, "the new full result becomes the source");

        let same_call_ledger: BTreeMap<_, _> = updates.into_iter().collect();
        let (same_call, updates) =
            prepare_info_result("repeat", &args, &data, &same_call_ledger, true);
        assert_eq!(same_call.content, stripped_content(&data.details));
        assert!(updates.is_empty(), "a call id cannot source itself");

        let (hook_mutated, updates) =
            prepare_info_result("later", &args, &data, &same_call_ledger, false);
        assert_eq!(hook_mutated, data);
        assert!(updates.is_empty());

        let changed_details = json!({
            "function_id": "worker::function",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "string" }
        });
        let changed = ResultData {
            content: normalize(&changed_details).0,
            is_error: false,
            details: changed_details,
        };
        let (prepared, updates) =
            prepare_info_result("changed", &args, &changed, &same_call_ledger, true);
        assert_eq!(prepared.content, stripped_content(&changed.details));
        assert_eq!(prepared.details, changed.details);
        assert_eq!(updates.len(), 1, "a changed contract stays full");

        let error = ResultData {
            is_error: true,
            ..data.clone()
        };
        let (prepared, updates) =
            prepare_info_result("error", &args, &error, &same_call_ledger, true);
        assert_eq!(prepared, error);
        assert!(updates.is_empty());
    }

    #[test]
    fn malformed_single_info_contract_stays_full_and_is_not_recorded() {
        let details = json!({
            "function_id": "worker::function",
            "request_schema": null,
            "response_schema": "not a schema"
        });
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details: details.clone(),
        };
        let ledger = BTreeMap::from([(
            "worker::function".into(),
            FunctionContractLedgerEntry {
                contract_digest: digest_value(&details).unwrap(),
                source_function_call_id: "source".into(),
                source_content_digest: "source-content".into(),
                eligible: true,
            },
        )]);

        let (prepared, updates) = prepare_info_result(
            "repeat",
            &json!({ "function_id": "worker::function" }),
            &data,
            &ledger,
            true,
        );

        assert_eq!(prepared.content, stripped_content(&data.details));
        assert_eq!(prepared.details, data.details);
        assert!(updates.is_empty());
    }

    #[test]
    fn malformed_batch_entry_stays_full_while_a_valid_sibling_reuses_its_source() {
        let details = json!({ "functions": [
            {
                "function_id": "worker::bad",
                "request_schema": { "type": 42 },
                "response_schema": { "type": "object" }
            },
            {
                "function_id": "worker::good",
                "request_schema": { "type": "object" },
                "response_schema": { "type": "string" }
            }
        ]});
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details: details.clone(),
        };
        let ledger = BTreeMap::from([
            (
                "worker::bad".into(),
                FunctionContractLedgerEntry {
                    contract_digest: digest_value(&details["functions"][0]).unwrap(),
                    source_function_call_id: "bad-source".into(),
                    source_content_digest: "bad-content".into(),
                    eligible: true,
                },
            ),
            (
                "worker::good".into(),
                FunctionContractLedgerEntry {
                    contract_digest: digest_value(&details["functions"][1]).unwrap(),
                    source_function_call_id: "good-source".into(),
                    source_content_digest: "good-content".into(),
                    eligible: true,
                },
            ),
        ]);

        let (prepared, updates) = prepare_info_result(
            "batch-repeat",
            &json!({ "function_ids": ["worker::bad", "worker::good"] }),
            &data,
            &ledger,
            true,
        );
        let rendered: Value = serde_json::from_str(&ContentBlock::join_text(&prepared.content))
            .expect("prepared batch content is JSON");

        assert_eq!(
            rendered["functions"][0],
            json!({
                "function_id": "worker::bad",
                "request_schema": { "type": 42 }
            }),
            "malformed entry stays full, minus the stripped response schema"
        );
        assert_eq!(
            rendered["functions"][1],
            json!({
                "function_id": "worker::good",
                "contract_status": "unchanged_in_context",
                "source_function_call_id": "good-source"
            })
        );
        assert_eq!(prepared.details, details);
        assert!(updates.is_empty());
    }

    #[test]
    fn malformed_same_id_batch_is_never_compacted_or_recorded() {
        let details = json!({ "functions": [
            {
                "function_id": "worker::same",
                "request_schema": { "type": 42 },
                "response_schema": { "type": "object" }
            },
            {
                "function_id": "worker::same",
                "request_schema": { "type": "object" },
                "response_schema": { "type": "string" }
            }
        ]});
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details: details.clone(),
        };
        let ledger = BTreeMap::from([(
            "worker::same".into(),
            FunctionContractLedgerEntry {
                contract_digest: digest_value(&details["functions"][1]).unwrap(),
                source_function_call_id: "source".into(),
                source_content_digest: "source-content".into(),
                eligible: true,
            },
        )]);

        let (prepared, updates) = prepare_info_result(
            "repeat",
            &json!({ "function_ids": ["worker::same"] }),
            &data,
            &ledger,
            true,
        );

        assert_eq!(prepared.content, stripped_content(&data.details));
        assert_eq!(prepared.details, data.details);
        assert!(updates.is_empty());
    }

    #[test]
    fn duplicate_non_source_result_id_cannot_become_contract_source() {
        let details = json!({
            "function_id": "worker::function",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
        });
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details,
        };
        let mut ledger = BTreeMap::new();
        let (prepared, updates) = prepare_info_result(
            "duplicate",
            &json!({ "function_id": "worker::function" }),
            &data,
            &ledger,
            true,
        );
        apply_contract_updates_after_append(&mut ledger, "duplicate", updates);
        retain_visible_contract_sources(
            &mut ledger,
            &[
                json!({
                    "role": "function_result",
                    "function_call_id": "duplicate",
                    "function_id": "worker::other",
                    "content": [{ "type": "text", "text": "earlier" }]
                }),
                json!({
                    "role": "function_result",
                    "function_call_id": "duplicate",
                    "function_id": "engine::functions::info",
                    "content": serde_json::to_value(&prepared.content).unwrap()
                }),
            ],
        );

        assert!(
            ledger.is_empty(),
            "a call id shared by visible results is ambiguous"
        );
    }

    #[test]
    fn duplicate_visible_calls_cannot_source_a_contract() {
        let details = json!({
            "function_id": "worker::function",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
        });
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details,
        };
        let mut ledger = BTreeMap::new();
        let (prepared, updates) = prepare_info_result(
            "duplicate",
            &json!({ "function_id": "worker::function" }),
            &data,
            &ledger,
            true,
        );
        apply_contract_updates_after_append(&mut ledger, "duplicate", updates);
        retain_visible_contract_sources(
            &mut ledger,
            &[
                json!({
                    "role": "assistant",
                    "content": [
                        {
                            "type": "function_call",
                            "id": "duplicate",
                            "function_id": "engine::functions::info"
                        },
                        {
                            "type": "function_call",
                            "id": "duplicate",
                            "function_id": "worker::other"
                        }
                    ]
                }),
                json!({
                    "role": "function_result",
                    "function_call_id": "duplicate",
                    "function_id": "engine::functions::info",
                    "content": serde_json::to_value(&prepared.content).unwrap()
                }),
            ],
        );

        assert!(
            ledger.is_empty(),
            "a call id shared by visible calls is ambiguous"
        );
    }

    #[test]
    fn visible_source_call_must_resolve_to_info() {
        let details = json!({
            "function_id": "worker::function",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
        });
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details,
        };
        let mut ledger = BTreeMap::new();
        let (prepared, updates) = prepare_info_result(
            "source",
            &json!({ "function_id": "worker::function" }),
            &data,
            &ledger,
            true,
        );
        apply_contract_updates_after_append(&mut ledger, "source", updates);
        let result = json!({
            "role": "function_result",
            "function_call_id": "source",
            "function_id": "engine::functions::info",
            "content": serde_json::to_value(&prepared.content).unwrap()
        });

        let mut wrapper_ledger = ledger.clone();
        retain_visible_contract_sources(
            &mut wrapper_ledger,
            &[
                json!({
                    "role": "assistant",
                    "content": [{
                        "type": "function_call",
                        "id": "source",
                        "function_id": "agent_trigger",
                        "arguments": { "function": "engine::functions::info" }
                    }]
                }),
                result.clone(),
            ],
        );
        assert!(wrapper_ledger["worker::function"].eligible);

        retain_visible_contract_sources(
            &mut ledger,
            &[
                json!({
                    "role": "assistant",
                    "content": [{
                        "type": "function_call",
                        "id": "source",
                        "function_id": "worker::other",
                        "arguments": {}
                    }]
                }),
                result,
            ],
        );
        assert!(
            ledger.is_empty(),
            "a mismatched visible call cannot identify the info result"
        );
    }

    #[test]
    fn duplicate_batch_entries_are_never_compacted_or_recorded() {
        let contract = json!({
            "function_id": "worker::function",
            "request_schema": { "type": "object" },
            "response_schema": { "type": "object" }
        });
        let details = json!({ "functions": [contract.clone(), contract] });
        let data = ResultData {
            content: normalize(&details).0,
            is_error: false,
            details,
        };

        let (prepared, updates) = prepare_info_result(
            "duplicates",
            &json!({ "function_ids": ["worker::function", "worker::function"] }),
            &data,
            &BTreeMap::new(),
            true,
        );
        assert_eq!(prepared.content, stripped_content(&data.details));
        assert_eq!(prepared.details, data.details);
        assert!(updates.is_empty());
    }

    #[test]
    fn persisted_contract_digests_are_stable_sha256() {
        assert_eq!(
            digest_value(&json!({ "a": 1 })).as_deref(),
            Some("015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862")
        );
    }

    #[test]
    fn normalize_string_value() {
        let (content, is_error) = normalize(&json!("hello"));
        assert!(!is_error);
        assert_eq!(content.len(), 1);
        assert!(matches!(&content[0], ContentBlock::Text { text } if text == "hello"));
    }

    #[test]
    fn normalize_uses_content_blocks_when_present() {
        let v = json!({ "content": [{ "type": "text", "text": "ok" }], "is_error": true });
        let (content, is_error) = normalize(&v);
        assert!(is_error);
        assert_eq!(content.len(), 1);
    }

    #[test]
    fn discovery_filter_drops_uncallable_functions() {
        let policy = pol(&["shell::*"]);
        let mut v = json!({ "functions": [
            { "function_id": "shell::run" },
            { "function_id": "fs::read" }
        ]});
        post_filter_discovery(&mut v, &policy);
        let ids: Vec<&str> = v["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["function_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["shell::run"]);
    }

    #[test]
    fn info_filter_masks_denied_batch_entries_without_dropping() {
        let policy = pol(&["shell::*"]);
        let mut v = json!({ "functions": [
            { "function_id": "shell::run", "request_schema": { "type": "object" } },
            { "function_id": "fs::read", "request_schema": { "type": "object" } },
            { "function_id": "nope::missing", "error": "not_found" }
        ]});
        post_filter_info(&mut v, &policy);
        let items = v["functions"].as_array().unwrap();
        assert_eq!(items.len(), 3, "batch entries are masked, never dropped");
        assert!(items[0].get("request_schema").is_some());
        assert_eq!(
            items[1],
            json!({ "function_id": "fs::read", "error": "not available" })
        );
        // A not_found marker for an id outside the policy is masked uniformly.
        assert_eq!(
            items[2],
            json!({ "function_id": "nope::missing", "error": "not available" })
        );
    }

    #[test]
    fn info_filter_blanks_denied_single_detail() {
        let policy = pol(&["shell::*"]);
        let mut allowed = json!({ "function_id": "shell::run", "request_schema": {} });
        post_filter_info(&mut allowed, &policy);
        assert!(allowed.is_object());

        let mut denied = json!({ "function_id": "fs::read", "request_schema": {} });
        post_filter_info(&mut denied, &policy);
        assert!(denied.is_null());
    }
}
