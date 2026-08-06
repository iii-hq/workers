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
    // A holding hook may mutate as it holds (approval-gate: park the call AND
    // stamp validated context onto it) — its mutations must not be dropped.
    Hold(HookMutations),
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
        /// Arguments as mutated by the chain up to (not including) the holder,
        /// checkpointed so a release executes the mutated call (issue #506).
        arguments: Value,
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
                HookOutcome::Hold(_) => {}
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
                HookOutcome::Hold(_) => {}
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
    /// `functions` globs match the target are consulted. `resume_after` is
    /// the hook-held release path (harness.md § `function::resolve`): the
    /// chain resumes AFTER the named holder (see [`chain_slice`]), over the
    /// checkpointed arguments it already mutated.
    pub async fn run_pre_trigger(
        &self,
        record: &TurnRecord,
        step: u64,
        call_id: &str,
        function_id: &str,
        arguments: &Value,
        resume_after: Option<&str>,
    ) -> PreTriggerOutcome {
        let mut args = arguments.clone();
        let mut annotations = Map::new();
        for binding in chain_slice(self.pre_trigger.ordered(), function_id, resume_after) {
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
                HookOutcome::Hold(m) => {
                    // Apply the holding hook's own rewrite before parking, so
                    // the release runs with the full chain's effective args.
                    if let Some(a) = m.arguments {
                        args = a;
                    }
                    merge(&mut annotations, m.annotations);
                    return PreTriggerOutcome::Hold {
                        held_by: binding.function_id.clone(),
                        arguments: args,
                        annotations,
                    };
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
                HookOutcome::Hold(_) => {
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

    /// `post_turn`: gate completion on the contract-valid result. Bindings are
    /// scoped by `sessions` globs. A template binding (`payload` set) calls a
    /// plain composition function (fp::pipe) with the result injected at
    /// `result_into` and reads its receipt as the verdict; an envelope binding
    /// speaks the ordinary hook contract (`deny` blocks). A deny re-prompts
    /// the turn through the validation-retry budget; `prompt` (from the
    /// binding's `retry_prompt`) replaces the generic nudge verbatim.
    pub async fn run_post_turn(
        &self,
        record: &TurnRecord,
        step: u64,
        result: &Value,
    ) -> Result<(), PostTurnDeny> {
        for binding in self.post_turn.ordered() {
            if !sessions_match(&binding, &record.session_id) {
                continue;
            }
            if let Some(template) = &binding.payload {
                // Template mode: no hook envelope, no mutations — a verdict.
                let payload = crate::functions::trigger_deliver::inject_at(
                    template.clone(),
                    binding.result_into.as_deref().unwrap_or("/value"),
                    result.clone(),
                );
                match self.invoke_value(&binding, "post_turn", payload).await {
                    Ok(response) => match validation_verdict(&response) {
                        Some(true) => {}
                        Some(false) => {
                            let reason =
                                format!("result rejected by validator {}", binding.function_id);
                            return Err(PostTurnDeny {
                                prompt: render_retry_prompt(&binding, &reason, Some(&response)),
                                reason,
                            });
                        }
                        None => {
                            // Contract confusion, not a real verdict: keep the
                            // generic text — a task-shaped custom prompt must
                            // not mask a broken validator.
                            return Err(PostTurnDeny::plain(format!(
                                "validator {} returned no verdict (expected `valid` or a pipe receipt)",
                                binding.function_id
                            )));
                        }
                    },
                    Err(message) => {
                        if let HookOutcome::Deny(reason) = self.on_error(&binding, message) {
                            return Err(PostTurnDeny::plain(reason));
                        }
                    }
                }
                continue;
            }
            let mut input = self.envelope(HookPoint::PostTurn, record, step);
            input["result"] = result.clone();
            match self.invoke(&binding, input).await {
                HookOutcome::Continue(_) => {}
                HookOutcome::Deny(reason) => {
                    return Err(PostTurnDeny {
                        prompt: render_retry_prompt(&binding, &reason, None),
                        reason,
                    })
                }
                HookOutcome::Hold(_) => {} // hold is not a post_turn verb
            }
        }
        Ok(())
    }

    /// Invoke one bound hook function, bounded by `timeout_ms` and resolved by
    /// `on_error`. A redelivered step re-runs hooks — they must be idempotent.
    async fn invoke(&self, binding: &HookBinding, input: Value) -> HookOutcome {
        let point = input
            .get("point")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        match self.invoke_value(binding, &point, input).await {
            Ok(value) => parse_output(value),
            Err(message) => self.on_error(binding, message),
        }
    }

    /// The raw bounded trigger shared by envelope and template invocations.
    async fn invoke_value(
        &self,
        binding: &HookBinding,
        point: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let baggage = [("iii.tag.kind", "harness.hook"), ("iii.hook.point", point)];
        let request = TriggerRequest {
            function_id: binding.function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(binding.timeout_ms),
        };
        // A hook target is dynamic: engine builtins stay in the default
        // namespace; sibling-worker functions follow this worker's namespace.
        let request: iii_sdk::protocol::TriggerRequestWithMetadata =
            match crate::trigger::route_namespace(&self.iii, &binding.function_id) {
                Some(namespace) => request.namespace(namespace),
                None => request.into(),
            };
        let fut = iii_helpers::observability::run_with_baggage(&baggage, self.iii.trigger(request));
        let bounded = tokio::time::timeout(
            Duration::from_millis(binding.timeout_ms.saturating_add(1_000)),
            fut,
        )
        .await;
        match bounded {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(format!("hook {} failed: {e}", binding.function_id)),
            Err(_) => Err(format!("hook {} timed out", binding.function_id)),
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
        Some("hold") => HookOutcome::Hold(parse_mutations(&value)),
        _ => HookOutcome::Continue(parse_mutations(&value)),
    }
}

/// Parse a hook's `mutations`/`annotations` — shared by the continue and hold
/// branches so a holding hook's rewrites are honored, not dropped.
fn parse_mutations(value: &Value) -> HookMutations {
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
    muts
}

fn merge(into: &mut Map<String, Value>, from: Map<String, Value>) {
    for (k, v) in from {
        into.insert(k, v);
    }
}

/// The bindings a `pre_trigger` run consults for this target: the glob-matched
/// chain, cut down to everything AFTER `resume_after` when resuming a released
/// hold — hooks up to and including the holder already ran and mutated the
/// checkpointed arguments, so re-running them would double-apply mutations
/// (and re-hold). If the holder is no longer bound, nothing runs: the chain
/// shape changed under the hold and the safe resume point is unknowable.
fn chain_slice(
    bindings: Vec<HookBinding>,
    function_id: &str,
    resume_after: Option<&str>,
) -> Vec<HookBinding> {
    let mut matched: Vec<HookBinding> = bindings
        .into_iter()
        .filter(|b| functions_match(b, function_id))
        .collect();
    let Some(holder) = resume_after else {
        return matched;
    };
    match matched.iter().position(|b| b.function_id == holder) {
        Some(i) => matched.split_off(i + 1),
        None => Vec::new(),
    }
}

/// Whether a pre/post_trigger binding's `functions` globs match the target
/// (no `functions` filter → all calls).
pub(super) fn functions_match(binding: &HookBinding, function_id: &str) -> bool {
    globs_match(binding.functions.as_deref(), function_id)
}

/// Whether a post_turn binding's `sessions` globs match the completing
/// session (no filter → every session).
fn sessions_match(binding: &HookBinding, session_id: &str) -> bool {
    globs_match(binding.sessions.as_deref(), session_id)
}

fn globs_match(globs: Option<&[String]>, candidate: &str) -> bool {
    match globs {
        // No filter, and an empty filter, both mean "every session".
        None | Some([]) => true,
        Some(globs) => {
            let mut builder = GlobSetBuilder::new();
            for g in globs {
                if let Ok(glob) = Glob::new(g) {
                    builder.add(glob);
                }
            }
            builder
                .build()
                .map(|set| set.is_match(candidate))
                .unwrap_or(false)
        }
    }
}

/// A `post_turn` denial: `reason` goes into logs and the failure record;
/// `prompt`, when the binding set `retry_prompt`, is the corrective message
/// sent to the model VERBATIM instead of the generic wrapper.
pub struct PostTurnDeny {
    pub reason: String,
    pub prompt: Option<String>,
}

impl PostTurnDeny {
    fn plain(reason: String) -> Self {
        PostTurnDeny {
            reason,
            prompt: None,
        }
    }
}

/// Render a binding's `retry_prompt`: `{reason}` = the deny reason,
/// `{value}` = the validator's measured value (fp::pipe receipt
/// `value_preview`; empty when unavailable).
fn render_retry_prompt(
    binding: &HookBinding,
    reason: &str,
    response: Option<&Value>,
) -> Option<String> {
    let template = binding.retry_prompt.as_deref()?;
    let value = response
        .and_then(|r| r.get("value_preview"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(
        template
            .replace("{reason}", reason)
            .replace("{value}", value),
    )
}

/// Template-mode verdict: `{ valid: bool }` is authoritative. An fp::pipe
/// receipt (recognised by its `steps` array) is invalid iff a failed
/// fp::when guard short-circuited it — the receipt OMITS `short_circuited`
/// on a full run (`skip_serializing_if`), so absence on a receipt = valid.
fn validation_verdict(response: &Value) -> Option<bool> {
    if let Some(b) = response.get("valid").and_then(Value::as_bool) {
        return Some(b);
    }
    if let Some(sc) = response.get("short_circuited").and_then(Value::as_bool) {
        return Some(!sc);
    }
    if response.get("steps").map(Value::is_array).unwrap_or(false) {
        return Some(true); // pipe receipt, no short-circuit flag = full run
    }
    None
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
            HookOutcome::Hold(m) => assert!(m.arguments.is_none()),
            _ => panic!("expected hold"),
        }
        // Legacy hooks may still send pending_timeout_ms; it is ignored.
        assert!(matches!(
            parse_output(json!({ "decision": "hold", "pending_timeout_ms": 1000 })),
            HookOutcome::Hold(_)
        ));
    }

    #[test]
    fn hold_keeps_mutations_and_annotations() {
        let out = parse_output(json!({
            "decision": "hold",
            "mutations": { "arguments": { "value": "expected+approved" } },
            "annotations": { "approved_by": "gate" }
        }));
        match out {
            HookOutcome::Hold(m) => {
                assert_eq!(m.arguments, Some(json!({ "value": "expected+approved" })));
                assert_eq!(m.annotations.get("approved_by"), Some(&json!("gate")));
            }
            _ => panic!("expected hold"),
        }
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

    fn binding(function_id: &str, priority: i64) -> HookBinding {
        HookBinding {
            function_id: function_id.into(),
            functions: Some(vec!["shell::*".into()]),
            sessions: None,
            payload: None,
            result_into: None,
            retry_prompt: None,
            priority,
            timeout_ms: 5000,
            fail_closed: true,
        }
    }

    fn ids(bindings: &[HookBinding]) -> Vec<&str> {
        bindings.iter().map(|b| b.function_id.as_str()).collect()
    }

    #[test]
    fn chain_slice_without_resume_runs_the_matched_chain() {
        let chain = vec![binding("mutate", 10), binding("gate", 20)];
        assert_eq!(
            ids(&chain_slice(chain, "shell::run", None)),
            vec!["mutate", "gate"]
        );
    }

    #[test]
    fn chain_slice_resumes_after_the_holder() {
        let chain = vec![
            binding("mutate", 10),
            binding("gate", 20),
            binding("audit", 30),
        ];
        // The mutator and the holder already ran — only `audit` remains.
        assert_eq!(
            ids(&chain_slice(chain.clone(), "shell::run", Some("gate"))),
            vec!["audit"]
        );
        // Holder last in the chain → nothing remains.
        assert!(chain_slice(chain, "shell::run", Some("audit")).is_empty());
    }

    #[test]
    fn chain_slice_runs_nothing_when_the_holder_was_unbound() {
        let chain = vec![binding("mutate", 10), binding("audit", 30)];
        assert!(chain_slice(chain, "shell::run", Some("gone")).is_empty());
    }

    #[test]
    fn chain_slice_positions_by_the_glob_matched_chain() {
        // A holder bound to a different target never matches; the released
        // call's matched chain decides the resume point.
        let mut other = binding("gate", 20);
        other.functions = Some(vec!["fs::*".into()]);
        let chain = vec![binding("mutate", 10), other, binding("audit", 30)];
        assert!(chain_slice(chain, "shell::run", Some("gate")).is_empty());
    }

    #[test]
    fn retry_prompt_renders_placeholders() {
        let mut b = binding("check", 0);
        b.retry_prompt = Some("Only {value} rows so far ({reason}). Insert more.".into());
        let receipt = json!({ "steps": [], "short_circuited": true, "value_preview": "4" });
        assert_eq!(
            render_retry_prompt(&b, "rejected", Some(&receipt)).unwrap(),
            "Only 4 rows so far (rejected). Insert more."
        );
        // Envelope mode has no receipt: {value} renders empty.
        assert_eq!(
            render_retry_prompt(&b, "rejected", None).unwrap(),
            "Only  rows so far (rejected). Insert more."
        );
        // No template → no override.
        assert!(render_retry_prompt(&binding("check", 0), "r", None).is_none());
    }

    #[test]
    fn validation_verdict_reads_valid_then_pipe_receipt() {
        // Explicit verdict is authoritative.
        assert_eq!(validation_verdict(&json!({ "valid": true })), Some(true));
        assert_eq!(validation_verdict(&json!({ "valid": false })), Some(false));
        // fp::pipe receipt: a failed fp::when guard short-circuits = invalid.
        assert_eq!(
            validation_verdict(&json!({ "short_circuited": true })),
            Some(false)
        );
        // Full-run receipt OMITS short_circuited (skip_serializing_if) —
        // the steps array alone must read as valid (live bug: a validated
        // turn looped to budget exhaustion on exactly this shape).
        assert_eq!(
            validation_verdict(
                &json!({ "steps": [{ "function": "fp::when" }], "value_preview": "18" })
            ),
            Some(true)
        );
        // `valid` wins over a receipt carrying both.
        assert_eq!(
            validation_verdict(&json!({ "valid": false, "short_circuited": false })),
            Some(false)
        );
        // No verdict at all.
        assert_eq!(validation_verdict(&json!({ "ok": 1 })), None);
        assert_eq!(validation_verdict(&Value::Null), None);
    }

    #[test]
    fn functions_filter_matches_globs() {
        let binding = HookBinding {
            function_id: "gate".into(),
            functions: Some(vec!["shell::*".into()]),
            sessions: None,
            payload: None,
            result_into: None,
            retry_prompt: None,
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
