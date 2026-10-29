//! Deterministic detectors over raw `session::messages` entries, and the
//! masked previews that may leave for a model. Detectors run on complete
//! transcripts before anything is reduced.

use std::collections::BTreeSet;

use harness::policy::{plan_calls, CallKind};
use harness::types::message::AgentMessage;
use harness::types::turn::ExposeMode;
use serde_json::{Map, Value};

use crate::contract::{CorrelationV1, DiagnosticV1, EntryRefV1, ExcludedProbeV1, RULES_VERSION};
use crate::ids;

pub const REPEATED_CONTRACT_DISCOVERY: &str = "repeated_contract_discovery";
pub const REPEATED_TOOL_ERROR: &str = "repeated_tool_error";
const CONTRACT_INFO: &str = "engine::functions::info";
const TRIGGERS_INFO: &str = "engine::triggers::info";
use harness::window::MODEL_NOTICE;
const REGISTRY_CHANGED: &str = "registry-changed";
const UNCHANGED_IN_CONTEXT: &str = "unchanged_in_context";

/// What the detectors found, plus how many assistant entries they could not
/// interpret (a coverage limitation, never a silent pass).
#[derive(Debug, Default)]
pub struct Detection {
    pub diagnostics: Vec<DiagnosticV1>,
    /// Expected protocol probes the error rule skipped on purpose.
    pub excluded_probes: Vec<ExcludedProbeV1>,
    pub unreadable_messages: usize,
}

struct Call {
    at: usize,
    id: String,
    target: String,
    payload: Value,
}

struct Outcome<'a> {
    at: usize,
    call_id: &'a str,
    function_id: &'a str,
    is_error: bool,
    details: &'a Value,
    content: &'a Value,
}

/// The turn a harness entry belongs to: entry ids are `e_<turn_id>_…` and
/// turn ids are `t_<hex>`. Session-level entries (user messages, spawns)
/// carry no turn.
pub fn entry_turn(entry_id: &str) -> Option<&str> {
    let rest = entry_id.strip_prefix("e_")?;
    let body = rest.strip_prefix("t_")?;
    let end = body.find('_').map_or(rest.len(), |index| index + 2);
    Some(&rest[..end])
}

pub fn entry_id(entry: &Value) -> &str {
    entry["entry_id"].as_str().unwrap_or_default()
}

pub fn role(entry: &Value) -> &str {
    entry["message"]["role"].as_str().unwrap_or_default()
}

pub fn is_registry_notice(entry: &Value) -> bool {
    entry["custom"]["custom_type"] == MODEL_NOTICE
        && entry["custom"]["data"]["kind"] == REGISTRY_CHANGED
}

pub fn detect(session_id: &str, entries: &[Value]) -> Detection {
    let mut detection = Detection::default();
    let mut calls = Vec::new();
    let mut outcomes = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        match role(entry) {
            "assistant" => match serde_json::from_value::<AgentMessage>(entry["message"].clone()) {
                // `Native` unwraps only `agent_trigger` wrappers; any other
                // block already names its target.
                Ok(AgentMessage::Assistant(message)) => calls.extend(
                    plan_calls(&message, ExposeMode::Native)
                        .into_iter()
                        .filter(|call| call.kind == CallKind::Trigger)
                        .map(|call| Call {
                            at,
                            id: call.id,
                            target: call.function_id,
                            payload: call.arguments,
                        }),
                ),
                _ => detection.unreadable_messages += 1,
            },
            "function_result" => {
                let message = &entry["message"];
                outcomes.push(Outcome {
                    at,
                    call_id: message["function_call_id"].as_str().unwrap_or_default(),
                    function_id: message["function_id"].as_str().unwrap_or_default(),
                    is_error: message["is_error"] == true,
                    details: &message["details"],
                    content: &message["content"],
                });
            }
            _ => {}
        }
    }
    detection.diagnostics.extend(repeated_contract_discovery(
        session_id, entries, &calls, &outcomes,
    ));
    let (errors, probes) = repeated_tool_error(session_id, entries, &calls, &outcomes);
    detection.diagnostics.extend(errors);
    detection.excluded_probes = probes;
    detection
}

/// A successful `engine::functions::info` whose every contract was already
/// in context: an `unchanged_in_context` marker, or a contract the Harness
/// re-sent in full with its "already in your context" note, each traced to an
/// earlier successful result that supplied the same schema.
fn repeated_contract_discovery(
    session_id: &str,
    entries: &[Value],
    calls: &[Call],
    outcomes: &[Outcome<'_>],
) -> Vec<DiagnosticV1> {
    let mut found = Vec::new();
    for result in outcomes
        .iter()
        .filter(|outcome| outcome.function_id == CONTRACT_INFO && !outcome.is_error)
    {
        let Some(call) = calls
            .iter()
            .rev()
            .find(|call| call.at < result.at && call.id == result.call_id)
        else {
            continue;
        };
        let Some(shown) = content_json(result.content) else {
            continue;
        };
        let items = contract_items(&shown);
        if items.is_empty() {
            continue;
        }
        let mut positions = BTreeSet::from([call.at, result.at]);
        let mut functions = Vec::new();
        let mut sources = Vec::new();
        let mut resent = 0;
        let mut last_source = 0;
        let verified = items.iter().all(|item| {
            let Some(function_id) = item["function_id"].as_str() else {
                return false;
            };
            let earlier = |outcome: &&Outcome<'_>| {
                outcome.at < call.at && outcome.function_id == CONTRACT_INFO && !outcome.is_error
            };
            let source = if item["contract_status"] == UNCHANGED_IN_CONTEXT {
                let Some(source_id) = item["source_function_call_id"].as_str() else {
                    return false;
                };
                outcomes.iter().filter(earlier).find(|outcome| {
                    outcome.call_id == source_id
                        && raw_contract(outcome.details, function_id).is_some()
                })
            } else if item["note"].is_string() {
                // Re-sent in full because it was requested again: redundant
                // only if the raw contract is identical to an earlier one.
                let Some(current) = raw_contract(result.details, function_id) else {
                    return false;
                };
                resent += 1;
                outcomes
                    .iter()
                    .filter(earlier)
                    .rev()
                    .find(|outcome| raw_contract(outcome.details, function_id) == Some(current))
            } else {
                None
            };
            let Some(source) = source else {
                return false;
            };
            if let Some(source_call) = calls
                .iter()
                .rev()
                .find(|candidate| candidate.at < source.at && candidate.id == source.call_id)
            {
                positions.insert(source_call.at);
            }
            positions.insert(source.at);
            last_source = last_source.max(source.at);
            functions.push(function_id.to_string());
            sources.push(source.call_id.to_string());
            true
        });
        if !verified {
            continue;
        }
        let notices: Vec<usize> = (last_source + 1..call.at)
            .filter(|&at| is_registry_notice(&entries[at]))
            .collect();
        positions.extend(notices.iter().copied());
        sources.sort();
        sources.dedup();
        found.push(diagnostic(
            REPEATED_CONTRACT_DISCOVERY,
            session_id,
            entries,
            call,
            &[&call.id],
            CONTRACT_INFO,
            format!(
                "Call {} re-requested contract(s) {} that were already in context{}; the same \
                 schemas were supplied by call(s) {}.{}",
                call.id,
                functions.join(", "),
                if resent == 0 {
                    format!(" and received only `{UNCHANGED_IN_CONTEXT}`")
                } else {
                    format!(
                        " ({resent} re-sent in full with the Harness's already-in-context note)"
                    )
                },
                sources.join(", "),
                if notices.is_empty() {
                    String::new()
                } else {
                    format!(
                        " {} registry-changed notice(s) reached the model in between.",
                        notices.len()
                    )
                }
            ),
            if notices.is_empty() {
                CorrelationV1::Unknown
            } else {
                CorrelationV1::HarnessNoticeCorrelated
            },
            positions,
        ));
    }
    found
}

/// Consecutive calls in one turn to the same target with structurally equal
/// payloads, where the first failure was visible before the retry and both
/// failed with the same explicit error code.
fn repeated_tool_error(
    session_id: &str,
    entries: &[Value],
    calls: &[Call],
    outcomes: &[Outcome<'_>],
) -> (Vec<DiagnosticV1>, Vec<ExcludedProbeV1>) {
    let mut found = Vec::new();
    let mut probes = Vec::new();
    for pair in calls.windows(2) {
        let [first, second] = pair else {
            continue;
        };
        let turn = entry_turn(entry_id(&entries[first.at]));
        if first.target.is_empty()
            || first.target != second.target
            || first.payload != second.payload
            || first.id == second.id
            || turn.is_none()
            || turn != entry_turn(entry_id(&entries[second.at]))
        {
            continue;
        }
        let failure = |call: &Call, until: usize| {
            outcomes.iter().find(|outcome| {
                outcome.at > call.at
                    && outcome.at < until
                    && outcome.call_id == call.id
                    && outcome.function_id == call.target
                    && outcome.is_error
            })
        };
        let (Some(first_result), Some(second_result)) =
            (failure(first, second.at), failure(second, entries.len()))
        else {
            continue;
        };
        let (Some(code), Some(second_code)) = (
            error_code(first_result.details),
            error_code(second_result.details),
        ) else {
            continue;
        };
        if code != second_code {
            continue;
        }
        if expected_probe(&first.target, code, &first.payload) {
            probes.push(ExcludedProbeV1 {
                session_id: session_id.into(),
                turn_id: turn.map(str::to_string),
                target: first.target.clone(),
                code: code.into(),
                calls: vec![first.id.clone(), second.id.clone()],
            });
            continue;
        }
        found.push(diagnostic(
            REPEATED_TOOL_ERROR,
            session_id,
            entries,
            second,
            &[&first.id, &second.id],
            &first.target,
            format!(
                "Calls {} and {} to {} used equivalent arguments; the second was sent after the \
                 first failed with `{code}` and failed with the same code.",
                first.id, second.id, first.target
            ),
            CorrelationV1::Unknown,
            BTreeSet::from([first.at, first_result.at, second.at, second_result.at]),
        ));
    }
    (found, probes)
}

#[allow(clippy::too_many_arguments)]
fn diagnostic(
    rule_id: &str,
    session_id: &str,
    entries: &[Value],
    anchor: &Call,
    call_ids: &[&str],
    target: &str,
    observation: String,
    correlation: CorrelationV1,
    positions: BTreeSet<usize>,
) -> DiagnosticV1 {
    let turn_id = entry_turn(entry_id(&entries[anchor.at])).map(str::to_string);
    let fingerprint = ids::sha256_text(&format!(
        "{rule_id}\n{session_id}\n{}\n{}",
        turn_id.as_deref().unwrap_or_default(),
        call_ids.join("\n")
    ));
    DiagnosticV1 {
        rule_id: rule_id.into(),
        rule_version: RULES_VERSION.into(),
        fingerprint,
        session_id: session_id.into(),
        turn_id,
        target: target.into(),
        observation,
        correlation,
        evidence: positions
            .into_iter()
            .map(|at| EntryRefV1 {
                session_id: session_id.into(),
                entry_id: entry_id(&entries[at]).into(),
            })
            .collect(),
    }
}

/// Protocol probes that are expected to fail and are not recovery attempts.
fn expected_probe(target: &str, code: &str, payload: &Value) -> bool {
    target == TRIGGERS_INFO
        && code.eq_ignore_ascii_case("NOT_FOUND")
        && payload
            .get("namespace")
            .is_none_or(|namespace| namespace.is_null() || namespace == "default")
}

fn error_code(details: &Value) -> Option<&str> {
    details["error"]["code"]
        .as_str()
        .or_else(|| details["code"].as_str())
        .filter(|code| !code.trim().is_empty())
}

/// A batch envelope lists contracts under `functions`; a single-id lookup is
/// the contract itself.
fn contract_items(value: &Value) -> Vec<&Value> {
    match value.get("functions").and_then(Value::as_array) {
        Some(items) => items.iter().collect(),
        None if value.is_object() => vec![value],
        None => Vec::new(),
    }
}

/// The raw contract a successful info result carried for `function_id`.
fn raw_contract<'a>(details: &'a Value, function_id: &str) -> Option<&'a Value> {
    contract_items(details).into_iter().find(|item| {
        item.get("function_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            == Some(function_id)
            && item.get("error").is_none()
            && ["request_schema", "request_format", "parameters"]
                .iter()
                .any(|key| item.get(*key).is_some_and(|schema| !schema.is_null()))
    })
}

/// The model-visible result: the first text block holding a JSON object.
fn content_json(content: &Value) -> Option<Value> {
    content.as_array()?.iter().find_map(|block| {
        block["text"]
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .filter(Value::is_object)
    })
}

/// What a model may see of an entry: the model-visible content, error details
/// and identity. Raw result `details` and opaque reasoning payloads are
/// dropped; JSON carried as text is decoded so secret-named keys inside it are
/// masked by the shared Harness masker, which also shortens long strings.
/// Masking is key-based: secrets in free text are not detected.
pub fn preview(entry: &Value) -> (Value, bool) {
    let projected = decode_embedded_json(&project(entry));
    let shown = harness::judge::bounded(&projected);
    let reduced = shown != projected;
    (shown, reduced)
}

fn project(entry: &Value) -> Value {
    let mut out = Map::new();
    out.insert("entry_id".into(), entry["entry_id"].clone());
    if let Some(message) = entry["message"].as_object() {
        let mut kept = Map::new();
        for (key, value) in message {
            match key.as_str() {
                "timestamp" | "native_stop_reason" => {}
                "details" => {
                    if message.get("is_error") == Some(&Value::Bool(true)) {
                        if let Some(error) = value.get("error") {
                            kept.insert("details".into(), serde_json::json!({ "error": error }));
                        }
                    }
                }
                "content" => {
                    let blocks = value
                        .as_array()
                        .map(|blocks| {
                            blocks
                                .iter()
                                .filter(|block| block["type"] != "redacted_thinking")
                                .map(|block| {
                                    let mut block = block.clone();
                                    if let Some(object) = block.as_object_mut() {
                                        object.remove("signature");
                                    }
                                    block
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    kept.insert(key.clone(), Value::Array(blocks));
                }
                _ => {
                    kept.insert(key.clone(), value.clone());
                }
            }
        }
        out.insert("message".into(), Value::Object(kept));
    }
    if let Some(custom) = entry["custom"].as_object() {
        let mut kept = custom.clone();
        if let Some(data) = kept.get_mut("data").and_then(Value::as_object_mut) {
            // `data.message` repeats `data.text` for replay.
            data.remove("message");
        }
        out.insert("custom".into(), Value::Object(kept));
    }
    Value::Object(out)
}

fn decode_embedded_json(value: &Value) -> Value {
    match value {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(parsed @ (Value::Object(_) | Value::Array(_))) => decode_embedded_json(&parsed),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(decode_embedded_json).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), decode_embedded_json(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use harness::types::content::ContentBlock;
    use harness::types::event::StopReason;
    use harness::types::message::{FunctionResultMessage, FunctionResultRoleTag};
    use serde_json::json;

    use super::*;

    const TURN: &str = "t_aaa";

    // Entries are serialized from the Harness's own message types, the way
    // the turn loop stores them.
    fn entry(entry_id: String, message: AgentMessage) -> Value {
        json!({ "entry_id": entry_id, "message": message })
    }

    fn assistant(step: u32, calls: Vec<ContentBlock>) -> Value {
        let mut message = harness::types::message::empty_assistant("p", "m");
        message.stop_reason = StopReason::FunctionCall;
        message.content = calls;
        entry(
            format!("e_{TURN}_{step}_assistant"),
            AgentMessage::Assistant(message),
        )
    }

    fn wrapped(id: &str, function: &str, payload: Value) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: "agent_trigger".into(),
            arguments: json!({"function": function, "description": "work", "payload": payload}),
        }
    }

    fn native(id: &str, function: &str, payload: Value) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: function.into(),
            arguments: payload,
        }
    }

    fn result(id: &str, function: &str, is_error: bool, details: Value, shown: Value) -> Value {
        entry(
            format!("e_{TURN}_{id}"),
            AgentMessage::FunctionResult(FunctionResultMessage {
                role: FunctionResultRoleTag::FunctionResult,
                function_call_id: id.into(),
                function_id: function.into(),
                content: vec![ContentBlock::text(shown.to_string())],
                details,
                is_error,
                timestamp: 1,
            }),
        )
    }

    fn error(id: &str, function: &str, code: &str) -> Value {
        result(
            id,
            function,
            true,
            json!({"error": {"code": code, "message": "nope"}}),
            json!({}),
        )
    }

    fn contract(function: &str) -> Value {
        json!({"function_id": function, "request_schema": {"type": "object"},
            "response_schema": {"type": "object"}})
    }

    fn marker(function: &str, source: &str) -> Value {
        json!({"function_id": function, "contract_status": "unchanged_in_context",
            "source_function_call_id": source})
    }

    fn notice(step: u32) -> Value {
        let message = json!({"role": "user", "timestamp": 1, "content": [
            {"type": "text", "text": "NOTE: the function registry changed during this conversation."}
        ]});
        json!({
            "entry_id": format!("e_{TURN}_{step}_notice_0"),
            "custom": {
                "custom_type": harness::window::MODEL_NOTICE,
                "data": harness::window::notice_data("registry-changed", &message),
            }
        })
    }

    fn rediscovery(with_notice: bool) -> Vec<Value> {
        let mut entries = vec![
            assistant(
                0,
                vec![wrapped(
                    "c1",
                    CONTRACT_INFO,
                    json!({"function_id": "crm::profile"}),
                )],
            ),
            result(
                "c1",
                CONTRACT_INFO,
                false,
                contract("crm::profile"),
                contract("crm::profile"),
            ),
        ];
        if with_notice {
            entries.push(notice(1));
        }
        entries.push(assistant(
            1,
            vec![wrapped(
                "c2",
                CONTRACT_INFO,
                json!({"function_id": "crm::profile"}),
            )],
        ));
        entries.push(result(
            "c2",
            CONTRACT_INFO,
            false,
            contract("crm::profile"),
            marker("crm::profile", "c1"),
        ));
        entries
    }

    #[test]
    fn entry_turn_parses_harness_entry_ids() {
        assert_eq!(entry_turn("e_t_abc_3_assistant"), Some("t_abc"));
        assert_eq!(entry_turn("e_t_abc_call_9"), Some("t_abc"));
        assert_eq!(entry_turn("e_t_abc"), Some("t_abc"));
        assert_eq!(entry_turn("e_idem_key"), None);
        assert_eq!(entry_turn("e_spawn_1"), None);
    }

    #[test]
    fn rediscovery_after_registry_notice_is_correlated_with_all_evidence() {
        let entries = rediscovery(true);
        let found = detect("s", &entries).diagnostics;
        assert_eq!(found.len(), 1);
        let diagnostic = &found[0];
        assert_eq!(diagnostic.rule_id, REPEATED_CONTRACT_DISCOVERY);
        assert_eq!(
            diagnostic.correlation,
            CorrelationV1::HarnessNoticeCorrelated
        );
        assert_eq!(diagnostic.turn_id.as_deref(), Some(TURN));
        let ids: Vec<_> = diagnostic
            .evidence
            .iter()
            .map(|e| e.entry_id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "e_t_aaa_0_assistant",
                "e_t_aaa_c1",
                "e_t_aaa_1_notice_0",
                "e_t_aaa_1_assistant",
                "e_t_aaa_c2"
            ]
        );
    }

    #[test]
    fn rediscovery_without_notice_has_unknown_cause() {
        let found = detect("s", &rediscovery(false)).diagnostics;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].correlation, CorrelationV1::Unknown);
    }

    #[test]
    fn batch_rediscovery_needs_every_contract_unchanged() {
        let batch = |items: Vec<Value>| json!({"functions": items});
        let mut entries = vec![
            assistant(
                0,
                vec![native(
                    "c1",
                    CONTRACT_INFO,
                    json!({"function_ids": ["a::x", "b::y"]}),
                )],
            ),
            result(
                "c1",
                CONTRACT_INFO,
                false,
                batch(vec![contract("a::x"), contract("b::y")]),
                batch(vec![contract("a::x"), contract("b::y")]),
            ),
            assistant(
                1,
                vec![native(
                    "c2",
                    CONTRACT_INFO,
                    json!({"function_ids": ["a::x", "b::y"]}),
                )],
            ),
            result(
                "c2",
                CONTRACT_INFO,
                false,
                batch(vec![contract("a::x"), contract("b::y")]),
                batch(vec![marker("a::x", "c1"), marker("b::y", "c1")]),
            ),
        ];
        assert_eq!(detect("s", &entries).diagnostics.len(), 1);
        // A new contract in the same answer is new information.
        entries[3] = result(
            "c2",
            CONTRACT_INFO,
            false,
            batch(vec![contract("a::x"), contract("b::y")]),
            batch(vec![marker("a::x", "c1"), contract("b::y")]),
        );
        assert!(detect("s", &entries).diagnostics.is_empty());
    }

    #[test]
    fn contract_resent_with_the_harness_note_counts_when_unchanged() {
        // The Harness's second answer to the same request: the full contract
        // again, with its already-in-context note.
        let resent = |details: Value| {
            let mut shown = contract("crm::profile");
            shown["note"] = json!("This contract was already in your context (call c1).");
            result("c2", CONTRACT_INFO, false, details, shown)
        };
        let mut entries = rediscovery(true);
        entries[4] = resent(contract("crm::profile"));
        let found = detect("s", &entries).diagnostics;
        assert_eq!(found.len(), 1);
        assert!(found[0].observation.contains("re-sent in full"));
        // Control: the contract really changed, so the lookup was needed.
        let mut changed = contract("crm::profile");
        changed["request_schema"] = json!({"type": "object", "required": ["id"]});
        entries[4] = resent(changed);
        assert!(detect("s", &entries).diagnostics.is_empty());
    }

    #[test]
    fn rediscovery_controls_new_contract_missing_source_and_errors() {
        // Contract resent in full: new information, not a marker.
        let mut entries = rediscovery(true);
        entries[4] = result(
            "c2",
            CONTRACT_INFO,
            false,
            contract("crm::profile"),
            contract("crm::profile"),
        );
        assert!(detect("s", &entries).diagnostics.is_empty());
        // Source call id that never produced the schema.
        let mut entries = rediscovery(true);
        entries[4] = result(
            "c2",
            CONTRACT_INFO,
            false,
            contract("crm::profile"),
            marker("crm::profile", "c9"),
        );
        assert!(detect("s", &entries).diagnostics.is_empty());
        // The source result was an error.
        let mut entries = rediscovery(true);
        entries[1] = result(
            "c1",
            CONTRACT_INFO,
            true,
            contract("crm::profile"),
            json!({}),
        );
        assert!(detect("s", &entries).diagnostics.is_empty());
        // The repeated call itself failed.
        let mut entries = rediscovery(true);
        entries[4]["message"]["is_error"] = json!(true);
        assert!(detect("s", &entries).diagnostics.is_empty());
    }

    fn repeated(code_b: &str, payload_b: Value) -> Vec<Value> {
        vec![
            assistant(
                0,
                vec![wrapped(
                    "a",
                    "crm::schedule",
                    json!({"day": 1, "slot": "am"}),
                )],
            ),
            error("a", "crm::schedule", "INVALID_SLOT"),
            assistant(1, vec![wrapped("b", "crm::schedule", payload_b)]),
            error("b", "crm::schedule", code_b),
        ]
    }

    #[test]
    fn repeated_error_matches_structurally_equal_payloads() {
        let entries = repeated("INVALID_SLOT", json!({"slot": "am", "day": 1}));
        let found = detect("s", &entries).diagnostics;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule_id, REPEATED_TOOL_ERROR);
        assert_eq!(found[0].target, "crm::schedule");
        assert_eq!(found[0].correlation, CorrelationV1::Unknown);
        assert_eq!(found[0].evidence.len(), 4);
    }

    #[test]
    fn repeated_error_controls_are_not_flagged() {
        // Corrected arguments.
        assert!(detect(
            "s",
            &repeated("INVALID_SLOT", json!({"day": 2, "slot": "am"}))
        )
        .diagnostics
        .is_empty());
        // A different error code.
        assert!(
            detect("s", &repeated("CONFLICT", json!({"day": 1, "slot": "am"})))
                .diagnostics
                .is_empty()
        );
        // The retry succeeded.
        let mut entries = repeated("INVALID_SLOT", json!({"day": 1, "slot": "am"}));
        entries[3] = result(
            "b",
            "crm::schedule",
            false,
            json!({"ok": true}),
            json!({"ok": true}),
        );
        assert!(detect("s", &entries).diagnostics.is_empty());
        // Both calls in one response: the first error was not visible yet.
        let entries = vec![
            assistant(
                0,
                vec![
                    wrapped("a", "crm::schedule", json!({"day": 1})),
                    wrapped("b", "crm::schedule", json!({"day": 1})),
                ],
            ),
            error("a", "crm::schedule", "INVALID_SLOT"),
            error("b", "crm::schedule", "INVALID_SLOT"),
        ];
        assert!(detect("s", &entries).diagnostics.is_empty());
        // Independent attempts in different turns.
        let mut entries = repeated("INVALID_SLOT", json!({"day": 1, "slot": "am"}));
        entries[2]["entry_id"] = json!("e_t_bbb_0_assistant");
        assert!(detect("s", &entries).diagnostics.is_empty());
        // Another call in between.
        let mut entries = repeated("INVALID_SLOT", json!({"day": 1, "slot": "am"}));
        entries.insert(2, assistant(5, vec![wrapped("c", "crm::list", json!({}))]));
        assert!(detect("s", &entries).diagnostics.is_empty());
        // No explicit error code.
        let mut entries = repeated("INVALID_SLOT", json!({"day": 1, "slot": "am"}));
        entries[1]["message"]["details"] = json!({});
        entries[3]["message"]["details"] = json!({});
        assert!(detect("s", &entries).diagnostics.is_empty());
    }

    #[test]
    fn default_namespace_trigger_probe_is_expected_but_other_namespaces_are_not() {
        let probe = |payload: Value| {
            vec![
                assistant(0, vec![wrapped("a", TRIGGERS_INFO, payload.clone())]),
                error("a", TRIGGERS_INFO, "NOT_FOUND"),
                assistant(1, vec![wrapped("b", TRIGGERS_INFO, payload)]),
                error("b", TRIGGERS_INFO, "NOT_FOUND"),
            ]
        };
        let quiet = detect("s", &probe(json!({"trigger_type": "x"})));
        assert!(quiet.diagnostics.is_empty());
        assert_eq!(
            quiet.excluded_probes.len(),
            1,
            "the skipped probe is recorded"
        );
        assert_eq!(quiet.excluded_probes[0].target, TRIGGERS_INFO);
        assert_eq!(quiet.excluded_probes[0].code, "NOT_FOUND");
        assert_eq!(quiet.excluded_probes[0].calls, ["a", "b"]);
        assert!(detect(
            "s",
            &probe(json!({"trigger_type": "x", "namespace": "default"}))
        )
        .diagnostics
        .is_empty());
        assert_eq!(
            detect(
                "s",
                &probe(json!({"trigger_type": "x", "namespace": "crm"}))
            )
            .diagnostics
            .len(),
            1
        );
    }

    #[test]
    fn fingerprints_distinguish_occurrences_of_the_same_function() {
        let mut entries = repeated("INVALID_SLOT", json!({"day": 1, "slot": "am"}));
        entries.extend([
            assistant(
                2,
                vec![wrapped(
                    "c",
                    "crm::schedule",
                    json!({"day": 1, "slot": "am"}),
                )],
            ),
            error("c", "crm::schedule", "INVALID_SLOT"),
        ]);
        let found = detect("s", &entries).diagnostics;
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].fingerprint, found[1].fingerprint);
    }

    #[test]
    fn unreadable_assistant_entries_are_counted() {
        let entries = vec![json!({"entry_id": "e_t_aaa_0_assistant",
            "message": {"role": "assistant", "content": "not blocks"}})];
        assert_eq!(detect("s", &entries).unreadable_messages, 1);
    }

    #[test]
    fn preview_masks_secrets_inside_json_text_and_drops_raw_details() {
        let entry = json!({"entry_id": "e", "message": {
            "role": "function_result", "function_call_id": "c", "function_id": "f",
            "content": [{"type": "text", "text": "{\"api_key\":\"private-value\",\"value\":42}"}],
            "details": {"api_key": "private-value", "raw": "x"}, "is_error": false, "timestamp": 1
        }});
        let (shown, reduced) = preview(&entry);
        let text = shown.to_string();
        assert!(!text.contains("private-value"), "{text}");
        assert!(reduced);
        assert_eq!(shown["message"]["content"][0]["text"]["value"], 42);
        assert!(shown["message"].get("details").is_none());
    }

    #[test]
    fn preview_keeps_error_details_and_reports_truncation() {
        let long = "x".repeat(2_000);
        let entry = json!({"entry_id": "e", "message": {
            "role": "function_result", "function_call_id": "c", "function_id": "f",
            "content": [{"type": "text", "text": long}],
            "details": {"error": {"code": "E", "message": "m"}, "raw": 1}, "is_error": true, "timestamp": 1
        }});
        let (shown, reduced) = preview(&entry);
        assert!(reduced);
        assert_eq!(
            shown["message"]["details"],
            json!({"error": {"code": "E", "message": "m"}})
        );
        let (plain, plain_reduced) = preview(&json!({"entry_id": "e", "custom": {
            "custom_type": "model_notice", "data": {"kind": "k", "text": "t", "message": {}}}}));
        assert!(!plain_reduced);
        assert!(plain["custom"]["data"].get("message").is_none());
    }
}
