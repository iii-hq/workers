//! The in-path hook chain (harness.md § Chain, hold, and failure semantics).
//! Hooks for a point run in priority order, each seeing the payload as mutated
//! by the previous; the first `deny`/`hold` short-circuits. Failures resolve by
//! `on_error` (fail-closed `pre_*` deny, fail-open `post_*` skip). Mutations are
//! recorded into the written entry's `origin` via the returned annotations.

use std::time::Duration;

use globset::{Glob, GlobSetBuilder};
use iii_sdk::protocol::TriggerRequest;
use serde_json::{Map, Value};

use super::{HookBinding, HookPoint, HookRegistry};
use crate::types::content::ContentBlock;
use crate::types::turn::TurnRecord;

/// Mutations a `continue` hook may return.
#[derive(Default)]
struct HookMutations {
    system_prompt: Option<String>,
    append_messages: Vec<Value>,
    arguments: Option<Value>,
    content: Option<Vec<ContentBlock>>,
    details: Option<Value>,
    is_error: Option<bool>,
    annotations: Map<String, Value>,
}

enum HookOutcome {
    Continue(HookMutations),
    Deny(String),
    Hold,
}

/// Outcome of the `pre_generate` chain.
pub enum PreGenerateOutcome {
    Continue {
        system_prompt: Option<String>,
        append_messages: Vec<Value>,
        annotations: Map<String, Value>,
    },
    Deny(String),
}

/// Outcome of the `pre_trigger` chain.
pub enum PreTriggerOutcome {
    Continue {
        arguments: Value,
        annotations: Map<String, Value>,
    },
    Deny(String),
    Hold {
        held_by: String,
        annotations: Map<String, Value>,
    },
}

pub enum PostTriggerOutcome {
    Result {
        result: crate::trigger::ResultData,
        annotations: Map<String, Value>,
    },
    Hold {
        held_by: String,
        annotations: Map<String, Value>,
    },
}

impl HookRegistry {
    fn envelope(&self, point: HookPoint, record: &TurnRecord, step: u64) -> Value {
        let mut v = serde_json::json!({
            "point": point.input_name(),
            "session_id": record.session_id,
            "turn_id": record.turn_id,
            "step": step,
            "depth": record.depth,
        });
        if let Some(m) = &record.options.metadata {
            v["metadata"] = m.clone();
        }
        v
    }

    /// `pre_turn`: veto only. `Err(reason)` ends the turn.
    pub async fn run_pre_turn(&self, record: &TurnRecord, step: u64) -> Result<(), String> {
        for binding in self.pre_turn.ordered() {
            let input = self.envelope(HookPoint::PreTurn, record, step);
            match self.invoke(&binding, input).await {
                HookOutcome::Continue(_) => {}
                HookOutcome::Deny(reason) => return Err(reason),
                HookOutcome::Hold => {}
            }
        }
        Ok(())
    }

    /// `pre_generate`: extend the system prompt and append bounded messages, or
    /// veto. Each hook sees the prompt/messages as mutated by the previous.
    pub async fn run_pre_generate(
        &self,
        record: &TurnRecord,
        step: u64,
        base_system_prompt: Option<String>,
        base_messages: &[Value],
    ) -> PreGenerateOutcome {
        let mut system_prompt = base_system_prompt;
        let mut appended: Vec<Value> = Vec::new();
        let mut annotations = Map::new();
        for binding in self.pre_generate.ordered() {
            let mut messages = base_messages.to_vec();
            messages.extend(appended.iter().cloned());
            let mut input = self.envelope(HookPoint::PreGenerate, record, step);
            input["generate"] = serde_json::json!({
                "system_prompt": system_prompt.clone().unwrap_or_default(),
                "messages": messages,
                "model": record.options.model,
                "provider": record.options.provider.clone().unwrap_or_default(),
            });
            match self.invoke(&binding, input).await {
                HookOutcome::Continue(m) => {
                    if m.system_prompt.is_some() {
                        system_prompt = m.system_prompt;
                    }
                    appended.extend(m.append_messages);
                    merge(&mut annotations, m.annotations);
                }
                HookOutcome::Deny(reason) => return PreGenerateOutcome::Deny(reason),
                HookOutcome::Hold => {}
            }
        }
        PreGenerateOutcome::Continue {
            system_prompt,
            append_messages: appended,
            annotations,
        }
    }

    /// `post_generate`: observe only (usage accounting, safety logging).
    pub async fn run_post_generate(&self, record: &TurnRecord, step: u64, message: Value) {
        for binding in self.post_generate.ordered() {
            let mut input = self.envelope(HookPoint::PostGenerate, record, step);
            input["generated"] = serde_json::json!({ "message": message });
            let _ = self.invoke(&binding, input).await;
        }
    }

    /// `pre_trigger`: deny / hold / rewrite arguments. Only bindings whose
    /// `functions` globs match the target are consulted.
    pub async fn run_pre_trigger(
        &self,
        record: &TurnRecord,
        step: u64,
        call_id: &str,
        function_id: &str,
        arguments: &Value,
    ) -> PreTriggerOutcome {
        let mut args = arguments.clone();
        let mut annotations = Map::new();
        for binding in self.pre_trigger.ordered() {
            if !functions_match(&binding, function_id) {
                continue;
            }
            let mut input = self.envelope(HookPoint::PreTrigger, record, step);
            input["call"] = serde_json::json!({
                "id": call_id,
                "function_id": function_id,
                "arguments": args,
            });
            match self.invoke(&binding, input).await {
                HookOutcome::Continue(m) => {
                    if let Some(a) = m.arguments {
                        args = a;
                    }
                    merge(&mut annotations, m.annotations);
                }
                HookOutcome::Deny(reason) => return PreTriggerOutcome::Deny(reason),
                HookOutcome::Hold => {
                    return PreTriggerOutcome::Hold {
                        held_by: binding.function_id.clone(),
                        annotations,
                    }
                }
            }
        }
        PreTriggerOutcome::Continue {
            arguments: args,
            annotations,
        }
    }

    /// `post_trigger`: rewrite result content / details / is_error (redaction,
    /// truncation). Returns the (possibly rewritten) result + annotations.
    pub async fn run_post_trigger(
        &self,
        record: &TurnRecord,
        step: u64,
        call_id: &str,
        function_id: &str,
        arguments: &Value,
        mut result: crate::trigger::ResultData,
    ) -> PostTriggerOutcome {
        let mut annotations = Map::new();
        for binding in self.post_trigger.ordered() {
            if !functions_match(&binding, function_id) {
                continue;
            }
            let mut input = self.envelope(HookPoint::PostTrigger, record, step);
            input["call"] = serde_json::json!({
                "id": call_id,
                "function_id": function_id,
                "arguments": arguments,
            });
            input["result"] = serde_json::json!({
                "function_call_id": call_id,
                "function_id": function_id,
                "content": result.content,
                "is_error": result.is_error,
                "details": result.details,
            });
            match self.invoke(&binding, input).await {
                HookOutcome::Continue(m) => {
                    if let Some(c) = m.content {
                        result.content = c;
                    }
                    if let Some(d) = m.details {
                        result.details = d;
                    }
                    if let Some(e) = m.is_error {
                        result.is_error = e;
                    }
                    merge(&mut annotations, m.annotations);
                }
                // Deny is not valid at post_trigger; fail-open like existing
                // post-trigger error handling.
                HookOutcome::Deny(_) => {}
                HookOutcome::Hold => {
                    return PostTriggerOutcome::Hold {
                        held_by: binding.function_id.clone(),
                        annotations,
                    };
                }
            }
        }
        PostTriggerOutcome::Result {
            result,
            annotations,
        }
    }

    /// Invoke one bound hook function, bounded by `timeout_ms` and resolved by
    /// `on_error`. A redelivered step re-runs hooks — they must be idempotent.
    async fn invoke(&self, binding: &HookBinding, input: Value) -> HookOutcome {
        let fut = self.iii.trigger(TriggerRequest {
            function_id: binding.function_id.clone(),
            payload: input,
            action: None,
            timeout_ms: Some(binding.timeout_ms),
        });
        let bounded = tokio::time::timeout(
            Duration::from_millis(binding.timeout_ms.saturating_add(1_000)),
            fut,
        )
        .await;
        match bounded {
            Ok(Ok(value)) => parse_output(value),
            Ok(Err(e)) => {
                self.on_error(binding, format!("hook {} failed: {e}", binding.function_id))
            }
            Err(_) => self.on_error(binding, format!("hook {} timed out", binding.function_id)),
        }
    }

    fn on_error(&self, binding: &HookBinding, message: String) -> HookOutcome {
        if binding.fail_closed {
            tracing::warn!(function_id = %binding.function_id, %message, "hook failed closed (deny)");
            HookOutcome::Deny(message)
        } else {
            tracing::warn!(function_id = %binding.function_id, %message, "hook failed open (skip)");
            HookOutcome::Continue(HookMutations::default())
        }
    }
}

fn parse_output(value: Value) -> HookOutcome {
    if value.is_null() {
        return HookOutcome::Continue(HookMutations::default());
    }
    match value.get("decision").and_then(Value::as_str) {
        Some("deny") => HookOutcome::Deny(
            value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("denied by hook")
                .to_string(),
        ),
        Some("hold") => HookOutcome::Hold,
        _ => {
            let mut muts = HookMutations::default();
            if let Some(m) = value.get("mutations") {
                muts.system_prompt = m
                    .get("system_prompt")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(arr) = m.get("append_messages").and_then(Value::as_array) {
                    muts.append_messages = arr.clone();
                }
                muts.arguments = m.get("arguments").cloned();
                if let Some(c) = m.get("content") {
                    muts.content = serde_json::from_value(c.clone()).ok();
                }
                muts.details = m.get("details").cloned();
                muts.is_error = m.get("is_error").and_then(Value::as_bool);
            }
            if let Some(Value::Object(ann)) = value.get("annotations") {
                muts.annotations = ann.clone();
            }
            HookOutcome::Continue(muts)
        }
    }
}

fn merge(into: &mut Map<String, Value>, from: Map<String, Value>) {
    for (k, v) in from {
        into.insert(k, v);
    }
}

/// Whether a pre/post_trigger binding's `functions` globs match the target
/// (no `functions` filter → all calls).
fn functions_match(binding: &HookBinding, function_id: &str) -> bool {
    match &binding.functions {
        None => true,
        Some(globs) if globs.is_empty() => true,
        Some(globs) => {
            let mut builder = GlobSetBuilder::new();
            for g in globs {
                if let Ok(glob) = Glob::new(g) {
                    builder.add(glob);
                }
            }
            builder
                .build()
                .map(|set| set.is_match(function_id))
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_and_missing_decision_are_continue() {
        assert!(matches!(
            parse_output(Value::Null),
            HookOutcome::Continue(_)
        ));
        assert!(matches!(
            parse_output(json!({ "mutations": { "system_prompt": "x" } })),
            HookOutcome::Continue(_)
        ));
    }

    #[test]
    fn deny_and_hold_parse() {
        match parse_output(json!({ "decision": "deny", "reason": "nope" })) {
            HookOutcome::Deny(r) => assert_eq!(r, "nope"),
            _ => panic!("expected deny"),
        }
        match parse_output(json!({ "decision": "hold" })) {
            HookOutcome::Hold => {}
            _ => panic!("expected hold"),
        }
        // Legacy hooks may still send pending_timeout_ms; it is ignored.
        assert!(matches!(
            parse_output(json!({ "decision": "hold", "pending_timeout_ms": 1000 })),
            HookOutcome::Hold
        ));
    }

    #[test]
    fn continue_mutations_parse() {
        let out = parse_output(json!({
            "decision": "continue",
            "mutations": {
                "arguments": { "x": 1 },
                "content": [{ "type": "text", "text": "redacted" }],
                "is_error": true
            },
            "annotations": { "approved_by": "u_1" }
        }));
        match out {
            HookOutcome::Continue(m) => {
                assert_eq!(m.arguments, Some(json!({ "x": 1 })));
                assert!(m.content.is_some());
                assert_eq!(m.is_error, Some(true));
                assert_eq!(m.annotations.get("approved_by"), Some(&json!("u_1")));
            }
            _ => panic!("expected continue"),
        }
    }

    #[test]
    fn functions_filter_matches_globs() {
        let binding = HookBinding {
            function_id: "gate".into(),
            functions: Some(vec!["shell::*".into()]),
            priority: 0,
            timeout_ms: 5000,
            fail_closed: true,
        };
        assert!(functions_match(&binding, "shell::run"));
        assert!(!functions_match(&binding, "fs::read"));

        let all = HookBinding {
            functions: None,
            ..binding
        };
        assert!(functions_match(&all, "anything"));
    }
}
