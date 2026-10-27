//! The request one recorded step sent to the model, rebuilt from the
//! session's transcript with THIS build's code: the eval loop's level-1
//! replay samples it under two builds to see whether a change keeps the
//! model from a mistake it made there, or ([`sizes`]) measures every step
//! of a run to see what a change does to what the model is sent.
//!
//! Pure — no engine, no context worker. The system prompt is this build's
//! identity, the recorded skills index and the frozen runtime aid; the tools
//! are the `agent_trigger` surface; the messages are the window, where every
//! call this build refuses before it runs (`trigger::arguments_degraded`)
//! carries this build's refusal. A result the recording got from a worker
//! stays as recorded. Context assembly does not run, so a step that
//! compacted or pruned rebuilds differently: `req` is the fingerprint
//! [`super::generate_origin`] stamped on the recorded step, for the caller
//! to compare.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::LoadedEntry;
use crate::types::message::AgentMessage;
use crate::types::model::AgentFunction;
use crate::types::turn::ExposeMode;

#[derive(Debug, Deserialize)]
pub struct ReplayInput {
    /// The session path up to the step: every entry logged before its
    /// assistant entry.
    pub entries: Vec<LoadedEntry>,
    pub turn_id: String,
    pub step: u64,
    pub model: String,
    #[serde(default)]
    pub provider: Option<String>,
    /// The session's frozen skills index, when the recording kept it.
    #[serde(default)]
    pub skills: Option<String>,
}

/// A `router::complete` request, plus `req`.
#[derive(Debug, Serialize)]
pub struct ReplayRequest {
    pub model: String,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
    pub messages: Vec<Value>,
    pub tools: Vec<AgentFunction>,
    pub req: Value,
}

pub fn rebuild(input: &ReplayInput) -> ReplayRequest {
    let mut entries = input.entries.clone();
    refuse_as_this_build(&mut entries);
    let entries = &entries;
    let anchor = super::compaction_anchor("replay", entries);
    let mut window = crate::window::build(
        entries,
        anchor.window_start,
        watermark(entries, &input.turn_id, input.step).as_deref(),
        Some(&crate::ids::assistant_entry_id(&input.turn_id, input.step)),
    );
    if crate::window::binds_thinking(&input.model) {
        crate::window::strip_thinking_logged_before(&mut window, &entries[..anchor.record_index]);
    }
    let aid = crate::window::frozen_runtime_context(entries);
    let system_prompt = Some(super::compose_system_prompt(
        Some(crate::prompt::DEFAULT),
        input.skills.as_deref(),
        aid.as_deref(),
    ));
    // Pinned at send from the first user message (crate::language).
    let language = entries
        .iter()
        .find_map(|entry| match &entry.message {
            Some(message @ AgentMessage::User(_)) => Some(message),
            _ => None,
        })
        .and_then(crate::language::detect);
    let tools = super::provider_tools(ExposeMode::AgentTrigger, &[], language.as_ref());
    let mut messages: Vec<Value> = window
        .candidate
        .iter()
        .map(|(_, message)| serde_json::to_value(message).unwrap_or(Value::Null))
        .collect();
    super::patch_orphaned_calls(&mut messages);
    crate::clients::router::strip_file_blocks(&mut messages);
    let req = json!({
        "system_sha": super::json_sha(&system_prompt.as_deref()),
        "tools_sha": super::json_sha(&serde_json::to_value(&tools).unwrap_or(Value::Null)),
        "messages_sha": super::json_sha(&messages),
        "n": messages.len(),
    });
    ReplayRequest {
        model: input.model.clone(),
        provider: input.provider.clone(),
        system_prompt,
        messages,
        tools,
        req,
    }
}

/// Every recorded call this build refuses before it runs gets this build's
/// refusal as its result.
fn refuse_as_this_build(entries: &mut [LoadedEntry]) {
    let mut calls = HashMap::new();
    for entry in entries.iter_mut() {
        match &mut entry.message {
            Some(AgentMessage::Assistant(assistant)) => {
                for call in crate::policy::plan_calls(assistant, ExposeMode::AgentTrigger) {
                    calls.insert(
                        call.id,
                        (call.function_id, call.arguments, assistant.stop_reason),
                    );
                }
            }
            Some(AgentMessage::FunctionResult(result)) => {
                let Some((target, arguments, stop)) = calls.get(&result.function_call_id) else {
                    continue;
                };
                if crate::trigger::arguments_degraded(arguments) {
                    let refusal =
                        crate::trigger::truncated_arguments_result(target, arguments, *stop);
                    result.content = refusal.content;
                    result.details = refusal.details;
                    result.is_error = refusal.is_error;
                }
            }
            _ => {}
        }
    }
}

/// A whole recorded run, for [`sizes`].
#[derive(Debug, Deserialize)]
pub struct SizesInput {
    pub entries: Vec<LoadedEntry>,
    pub model: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub skills: Option<String>,
}

/// One recorded step's rebuilt request, measured.
#[derive(Debug, Serialize)]
pub struct StepSize {
    pub entry_id: String,
    /// Characters the model reads: the system prompt, the tools and each
    /// message's content.
    pub chars: usize,
    pub req: Value,
}

/// Every step of a recorded run rebuilt, at the recorded trajectory.
pub fn sizes(input: &SizesInput) -> Vec<StepSize> {
    input
        .entries
        .iter()
        .enumerate()
        .filter_map(|(at, entry)| {
            let (turn_id, step) = step_of(&entry.entry_id)?;
            let request = rebuild(&ReplayInput {
                entries: input.entries[..at].to_vec(),
                turn_id,
                step,
                model: input.model.clone(),
                provider: input.provider.clone(),
                skills: input.skills.clone(),
            });
            let chars = request.system_prompt.as_ref().map_or(0, String::len)
                + serde_json::to_string(&request.tools).map_or(0, |text| text.len())
                + request
                    .messages
                    .iter()
                    .map(|message| message["content"].to_string().len())
                    .sum::<usize>();
            Some(StepSize {
                entry_id: entry.entry_id.clone(),
                chars,
                req: request.req,
            })
        })
        .collect()
}

/// `e_<turn>_<step>_assistant`.
fn step_of(entry_id: &str) -> Option<(String, u64)> {
    let (turn, step) = entry_id
        .strip_prefix("e_")?
        .strip_suffix("_assistant")?
        .rsplit_once('_')?;
    Some((turn.to_string(), step.parse().ok()?))
}

/// The watermark the previous step kept: the last entry it loaded, the one
/// before the first it wrote (`e_<turn>_<step>_…`).
fn watermark(entries: &[LoadedEntry], turn_id: &str, step: u64) -> Option<String> {
    let prefix = format!("e_{turn_id}_{}_", step.checked_sub(1)?);
    let first = entries
        .iter()
        .position(|e| e.entry_id.starts_with(&prefix))?;
    entries[..first].last().map(|e| e.entry_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(values: Value) -> Vec<LoadedEntry> {
        serde_json::from_value(values).unwrap()
    }

    #[test]
    fn a_step_rebuilds_from_its_transcript_with_this_builds_prompt_and_tools() {
        let input = ReplayInput {
            entries: entries(json!([
                {"entry_id": "u0", "message": {"role": "user", "timestamp": 1,
                    "content": [{"type": "text", "text": "Please read the project files and fix the failing test."}]}},
                {"entry_id": "e_t_0_runtime_context",
                    "custom": {"custom_type": "runtime_context", "data": {"aid": "Your session id is s."}}},
                {"entry_id": "e_t_0_assistant", "message": {"role": "assistant", "timestamp": 2,
                    "model": "m", "provider": "p", "stop_reason": "end",
                    "content": [{"type": "text", "text": "Reading."}]}},
            ])),
            turn_id: "t".into(),
            step: 1,
            model: "m".into(),
            provider: Some("p".into()),
            skills: Some("<available_skills></available_skills>".into()),
        };
        let request = rebuild(&input);
        let prompt = request.system_prompt.as_deref().unwrap();
        assert!(prompt.starts_with(crate::prompt::DEFAULT.trim_end_matches('\n')));
        assert!(prompt.ends_with("<available_skills></available_skills>\n\nYour session id is s."));
        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.tools[0].name, crate::policy::AGENT_TRIGGER_NAME);
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.req["n"], 2);
        assert_eq!(watermark(&input.entries, "t", 1).as_deref(), Some("u0"));
        assert_eq!(watermark(&input.entries, "t", 0), None);

        let measured = sizes(&SizesInput {
            entries: input.entries.clone(),
            model: "m".into(),
            provider: None,
            skills: None,
        });
        assert_eq!(measured.len(), 1, "one assistant step");
        assert_eq!(measured[0].entry_id, "e_t_0_assistant");
        assert!(measured[0].chars > crate::prompt::DEFAULT.len());
    }

    #[test]
    fn a_call_this_build_refuses_carries_this_builds_refusal() {
        let mut path = entries(json!([
            {"entry_id": "a", "message": {"role": "assistant", "timestamp": 1, "model": "m",
                "provider": "p", "stop_reason": "function_call", "content": [{"type": "function_call",
                "id": "c1", "function_id": "agent_trigger", "arguments": {"_raw": "{\"function\": \"x::y"}}]}},
            {"entry_id": "r", "message": {"role": "function_result", "timestamp": 2,
                "function_call_id": "c1", "function_id": "agent_trigger", "is_error": true,
                "details": {}, "content": [{"type": "text", "text": "an older wording"}]}},
        ]));
        refuse_as_this_build(&mut path);
        let Some(AgentMessage::FunctionResult(result)) = &path[1].message else {
            panic!("a function result")
        };
        let text = crate::types::content::ContentBlock::join_text(&result.content);
        assert!(text.contains("did not run"), "{text}");
        assert_eq!(result.details["error"], "arguments_truncated");
    }
}
