//! The structural dispatch policy and the invocation-schema surface
//! (harness.md § Functions). A resolved policy permits everything by default
//! and subtracts deny globs; an explicit legacy allow-list can narrow it.
//! These globs are final — hooks run only after they pass.

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde_json::{json, Value};

use crate::types::content::ContentBlock;
use crate::types::message::AssistantMessage;
use crate::types::model::AgentFunction;
use crate::types::turn::{ExposeMode, FunctionPolicy};

/// The single generic invocation surface name (default exposure).
pub const AGENT_TRIGGER_NAME: &str = "agent_trigger";
/// The synthetic output-contract surface name (harness-consumed, never
/// dispatched).
pub const SUBMIT_RESULT_NAME: &str = "submit_result";

/// A compiled allow/deny matcher. A PRESENT policy with no `allow` permits
/// every function not matched by `deny`. A legacy explicit allow-list narrows
/// that universe, including an empty list which denies everything. A missing
/// policy remains deny-all for unscoped/internal callers.
pub struct CompiledPolicy {
    allow: Option<GlobSet>,
    deny: GlobSet,
    policy_missing: bool,
    policy_invalid: bool,
}

impl CompiledPolicy {
    pub fn from(policy: Option<&FunctionPolicy>) -> Self {
        match policy {
            Some(p) => {
                let (allow, allow_invalid) = match p.allow.as_deref() {
                    Some(patterns) => {
                        let (set, invalid) = build_set(patterns);
                        (Some(set), invalid)
                    }
                    None => (None, false),
                };
                let (deny, deny_invalid) = build_set(&p.deny);
                CompiledPolicy {
                    allow,
                    deny,
                    policy_missing: false,
                    policy_invalid: allow_invalid || deny_invalid,
                }
            }
            None => CompiledPolicy {
                allow: None,
                deny: GlobSet::empty(),
                policy_missing: true,
                policy_invalid: false,
            },
        }
    }

    /// Whether `function_id` may be dispatched.
    pub fn allows(&self, function_id: &str) -> bool {
        if self.policy_missing || self.policy_invalid {
            return false;
        }
        self.allow
            .as_ref()
            .is_none_or(|allow| allow.is_match(function_id))
            && !self.deny.is_match(function_id)
    }
}

/// Subset a child's requested policy against the parent's (harness.md §
/// Sub-agents: narrow, never escalate). Missing `allow` means unrestricted,
/// but a child still inherits a parent's explicit allow ceiling. When both
/// sides carry legacy allow-lists, only requested globs covered by the parent
/// survive. Denies are unioned, so a child may narrow but never remove a
/// parent's restriction. A `None` parent policy yields `None` (deny all).
pub fn subset_policy(
    parent: Option<&FunctionPolicy>,
    requested: Option<&FunctionPolicy>,
) -> Option<FunctionPolicy> {
    let parent = parent?;
    let requested_allow = requested.and_then(|r| r.allow.as_deref());
    let allow = match (parent.allow.as_deref(), requested_allow) {
        (None, None) => None,
        (None, Some(child)) => Some(child.to_vec()),
        (Some(parent), None) => Some(parent.to_vec()),
        (Some(parent), Some(child)) => Some(
            child
                .iter()
                .filter(|g| glob_covered(g, parent))
                .cloned()
                .collect(),
        ),
    };
    let mut deny = parent.deny.clone();
    if let Some(r) = requested {
        deny.extend(r.deny.iter().cloned());
    }
    let expose = requested.map(|r| r.expose).unwrap_or(parent.expose);
    Some(FunctionPolicy {
        allow,
        deny,
        expose,
    })
}

/// Cap a requested policy at the operator's configured baseline (ask mode).
/// Missing allow-lists are unrestricted; explicit lists remain positive
/// ceilings. Denies are unioned and the request's exposure choice is kept.
/// An absent request or baseline remains deny-all.
///
/// Coverage is [`glob_covered`] (exact id, `*`, or `<prefix>::*`), so the
/// result is a conservative UNDER-approximation of a set intersection for
/// two explicit prefix-glob allow-lists.
pub fn clamp_policy(
    baseline: Option<&FunctionPolicy>,
    requested: Option<&FunctionPolicy>,
) -> Option<FunctionPolicy> {
    let baseline = baseline?;
    let requested = requested?;

    let baseline_allow = baseline.allow.as_deref();
    let requested_allow = requested.allow.as_deref();
    let baseline_unrestricted = is_unrestricted(baseline_allow);
    let requested_unrestricted = is_unrestricted(requested_allow);
    let allow = match (baseline_unrestricted, requested_unrestricted) {
        (true, true) => None,
        (true, false) => Some(requested_allow.unwrap_or_default().to_vec()),
        (false, true) => Some(baseline_allow.unwrap_or_default().to_vec()),
        (false, false) => Some(
            baseline_allow
                .unwrap_or_default()
                .iter()
                .filter(|g| glob_covered(g, requested_allow.unwrap_or_default()))
                .cloned()
                .collect(),
        ),
    };
    let mut deny = baseline.deny.clone();
    deny.extend(requested.deny.iter().cloned());
    Some(FunctionPolicy {
        allow,
        deny,
        expose: requested.expose,
    })
}

/// Is a child allow glob covered by the parent's allow set? Conservative: an
/// exact match, a parent `*`, or a parent `<prefix>::*` covering the child.
fn glob_covered(child: &str, parent_globs: &[String]) -> bool {
    parent_globs.iter().any(|p| {
        p == child || p == "*" || (p.ends_with("::*") && child.starts_with(&p[..p.len() - 1]))
    })
}

fn is_unrestricted(allow: Option<&[String]>) -> bool {
    allow.is_none_or(|patterns| patterns.iter().any(|pattern| pattern == "*"))
}

fn build_set(patterns: &[String]) -> (GlobSet, bool) {
    let mut builder = GlobSetBuilder::new();
    let mut invalid = false;
    for p in patterns {
        if p.is_empty() {
            invalid = true;
            tracing::warn!("empty function glob makes the dispatch policy fail closed");
        } else if let Ok(glob) = Glob::new(p) {
            builder.add(glob);
        } else {
            invalid = true;
            tracing::warn!(pattern = %p, "invalid function glob makes the dispatch policy fail closed");
        }
    }
    match builder.build() {
        Ok(set) => (set, invalid),
        Err(error) => {
            tracing::warn!(%error, "function glob set failed to compile; policy fails closed");
            (GlobSet::empty(), true)
        }
    }
}

/// The single `agent_trigger` schema attached by default — the model triggers
/// any allowed function via `{ function, payload }`.
pub fn agent_trigger_schema() -> AgentFunction {
    AgentFunction {
        name: AGENT_TRIGGER_NAME.to_string(),
        description:
            "Trigger any allowed iii function by id. Discover what is callable at runtime \
                      via engine::functions::list / engine::functions::info."
                .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "function": { "type": "string", "description": "Target iii function id." },
                "payload": { "type": "object", "description": "Arguments for the target function." }
            },
            "required": ["function"]
        }),
        label: None,
        execution_mode: Some("sequential".to_string()),
    }
}

/// The synthetic `submit_result` schema (output-contract fallback). Its
/// `parameters` are the contract's output schema.
pub fn submit_result_schema(output_schema: Option<&Value>) -> AgentFunction {
    let parameters = output_schema
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));
    AgentFunction {
        name: SUBMIT_RESULT_NAME.to_string(),
        description: "Submit the final result for this turn. Call this exactly once when the job \
                      is done; its arguments are the turn's deliverable."
            .to_string(),
        parameters,
        label: None,
        execution_mode: Some("sequential".to_string()),
    }
}

/// What a planned call resolves to once unwrapped from the assistant message.
#[derive(Debug, Clone, PartialEq)]
pub enum CallKind {
    /// An `agent_trigger` wrapper or a native call to a real iii function.
    Trigger,
    /// The synthetic `submit_result` — consumed by the harness.
    SubmitResult,
}

/// One function call extracted from an assistant message, with the
/// `agent_trigger` wrapper unwrapped to the concrete target.
#[derive(Debug, Clone)]
pub struct PlannedCall {
    pub id: String,
    pub function_id: String,
    pub arguments: Value,
    pub kind: CallKind,
}

/// Extract the planned calls from an assistant message in content order. In
/// `agent_trigger` exposure the wrapper carries `{ function, payload }`; in
/// native exposure the block's `function_id` is the target directly.
pub fn plan_calls(message: &AssistantMessage, expose: ExposeMode) -> Vec<PlannedCall> {
    let mut out = Vec::new();
    for block in &message.content {
        if let ContentBlock::FunctionCall {
            id,
            function_id,
            arguments,
        } = block
        {
            if function_id == SUBMIT_RESULT_NAME {
                out.push(PlannedCall {
                    id: id.clone(),
                    function_id: SUBMIT_RESULT_NAME.to_string(),
                    arguments: arguments.clone(),
                    kind: CallKind::SubmitResult,
                });
                continue;
            }
            let (target, mut payload) =
                if function_id == AGENT_TRIGGER_NAME || expose == ExposeMode::AgentTrigger {
                    let target = arguments
                        .get("function")
                        .and_then(Value::as_str)
                        .unwrap_or(function_id)
                        .to_string();
                    let payload = match arguments.get("payload") {
                        // A stringified payload is always wrong (the schema says
                        // object): recover the object when the text starts with
                        // one. Leading-value parse, not from_str — models that
                        // stringify also tend to append stray closing braces,
                        // which a strict parse rejects as trailing data.
                        Some(Value::String(s)) => serde_json::Deserializer::from_str(s)
                            .into_iter::<Value>()
                            .next()
                            .and_then(Result::ok)
                            .filter(Value::is_object)
                            .unwrap_or_else(|| Value::String(s.clone())),
                        Some(v) => v.clone(),
                        // No `payload` key: the model flattened the target's
                        // arguments beside `function` (or called the target name
                        // as the tool). Hoist them instead of dropping them —
                        // dropping yields a misleading `missing field` error for
                        // fields the model visibly sent.
                        None => {
                            let mut map = match arguments {
                                Value::Object(m) => m.clone(),
                                _ => Default::default(),
                            };
                            map.remove("function");
                            Value::Object(map)
                        }
                    };
                    (target, payload)
                } else {
                    (function_id.clone(), arguments.clone())
                };
            // Truncation markers on the wrapper must survive the unwrap: a
            // max_tokens cut landing after a complete `payload` salvages to
            // `{function, payload, _partial: true}`, and unwrapping the
            // payload verbatim would shed the marker — bypassing the turn
            // loop's refusal to execute provider-degraded arguments.
            if let Value::Object(p) = &mut payload {
                for marker in ["_partial", "_raw"] {
                    if let Some(v) = arguments.get(marker) {
                        if !p.contains_key(marker) {
                            p.insert(marker.to_string(), v.clone());
                        }
                    }
                }
            }
            out.push(PlannedCall {
                id: id.clone(),
                function_id: target,
                arguments: payload,
                kind: CallKind::Trigger,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow: &[&str], deny: &[&str]) -> FunctionPolicy {
        FunctionPolicy {
            allow: Some(allow.iter().map(|s| s.to_string()).collect()),
            deny: deny.iter().map(|s| s.to_string()).collect(),
            expose: ExposeMode::AgentTrigger,
        }
    }

    fn deny_only(deny: &[&str]) -> FunctionPolicy {
        FunctionPolicy {
            allow: None,
            deny: deny.iter().map(|s| s.to_string()).collect(),
            expose: ExposeMode::AgentTrigger,
        }
    }

    #[test]
    fn absent_policy_denies_everything() {
        let p = CompiledPolicy::from(None);
        assert!(!p.allows("shell::run"));
        assert!(!p.allows("anything"));
    }

    #[test]
    fn deny_only_wire_policy_is_unrestricted_outside_its_denies() {
        let decoded: FunctionPolicy =
            serde_json::from_value(json!({ "deny": ["configuration::*"] })).unwrap();
        assert!(decoded.allow.is_none());
        let compiled = CompiledPolicy::from(Some(&decoded));
        assert!(compiled.allows("shell::run"));
        assert!(!compiled.allows("configuration::set"));
    }

    #[test]
    fn children_inherit_parent_policy_including_orchestration() {
        // A child that requests nothing inherits the parent's FULL policy —
        // same permissions as the main agent, orchestration included.
        let inherited = subset_policy(Some(&policy(&["*"], &[])), None).unwrap();
        let compiled = CompiledPolicy::from(Some(&inherited));
        for id in ["state::set", "harness::spawn", "engine::register_trigger"] {
            assert!(compiled.allows(id), "{id} must be inherited by default");
        }

        // Escalation is still impossible: a narrow parent doesn't grant
        // orchestration just because the child asks for it.
        let requested = policy(&["harness::spawn"], &[]);
        let capped = subset_policy(Some(&policy(&["state::*"], &[])), Some(&requested)).unwrap();
        assert!(!CompiledPolicy::from(Some(&capped)).allows("harness::spawn"));

        // A parentless spawn has nothing to inherit — deny-all stays deny-all.
        assert!(subset_policy(None, None).is_none());
    }

    #[test]
    fn empty_allow_denies_everything() {
        let p = CompiledPolicy::from(Some(&policy(&[], &[])));
        assert!(!p.allows("shell::run"));
    }

    #[test]
    fn deny_only_policy_allows_everything_else() {
        let p = CompiledPolicy::from(Some(&deny_only(&["configuration::*"])));
        assert!(p.allows("shell::run"));
        assert!(p.allows("engine::functions::list"));
        assert!(!p.allows("configuration::set"));
    }

    #[test]
    fn allow_glob_matches_prefix() {
        let p = CompiledPolicy::from(Some(&policy(&["shell::*"], &[])));
        assert!(p.allows("shell::run"));
        assert!(p.allows("shell::fs::read"));
        assert!(!p.allows("fs::read"));
    }

    #[test]
    fn deny_overrides_allow() {
        let p = CompiledPolicy::from(Some(&policy(&["*"], &["shell::*"])));
        assert!(p.allows("fs::read"));
        assert!(!p.allows("shell::run"));
    }

    #[test]
    fn malformed_or_empty_globs_fail_the_whole_policy_closed() {
        for malformed in ["[", ""] {
            let deny = deny_only(&[malformed]);
            let compiled = CompiledPolicy::from(Some(&deny));
            assert!(
                !compiled.allows("shell::run"),
                "malformed deny {malformed:?} must not widen access"
            );

            let allow = policy(&[malformed], &[]);
            let compiled = CompiledPolicy::from(Some(&allow));
            assert!(
                !compiled.allows("shell::run"),
                "malformed allow {malformed:?} must fail closed"
            );
        }
    }

    #[test]
    fn ask_clamp_caps_a_wildcard_request_at_the_baseline() {
        let baseline = policy(&["state::get", "coder::read-file"], &[]);
        let requested = policy(&["*"], &["coder::*"]);
        let clamped = clamp_policy(Some(&baseline), Some(&requested)).unwrap();
        let compiled = CompiledPolicy::from(Some(&clamped));
        assert!(compiled.allows("state::get"));
        // Outside the baseline, wildcard or not.
        assert!(!compiled.allows("state::set"));
        // The request's own denies still bite inside the baseline.
        assert!(!compiled.allows("coder::read-file"));
    }

    #[test]
    fn ask_clamp_intersects_prefix_requests_with_the_baseline() {
        let baseline = policy(&["state::get", "state::list", "harness::status"], &[]);
        let requested = policy(&["state::*"], &[]);
        let clamped = clamp_policy(Some(&baseline), Some(&requested)).unwrap();
        let compiled = CompiledPolicy::from(Some(&clamped));
        assert!(compiled.allows("state::get"));
        assert!(compiled.allows("state::list"));
        // In the baseline but not requested — intersection, not union.
        assert!(!compiled.allows("harness::status"));
        // Requested but outside the baseline.
        assert!(!compiled.allows("state::set"));
    }

    #[test]
    fn ask_clamp_never_widens_a_deny_all_request() {
        let baseline = policy(&["state::get"], &[]);
        assert!(clamp_policy(Some(&baseline), None).is_none());
        let empty = policy(&[], &[]);
        let clamped = clamp_policy(Some(&baseline), Some(&empty));
        assert!(!CompiledPolicy::from(clamped.as_ref()).allows("state::get"));
    }

    #[test]
    fn ask_clamp_without_a_usable_baseline_denies_everything() {
        let requested = policy(&["*"], &[]);
        assert!(clamp_policy(None, Some(&requested)).is_none());
        // A hollow baseline (empty allow) is deny-all, not a hole to widen through.
        let hollow = policy(&[], &["x"]);
        let clamped = clamp_policy(Some(&hollow), Some(&requested));
        assert!(!CompiledPolicy::from(clamped.as_ref()).allows("state::get"));
    }

    #[test]
    fn ask_clamp_keeps_the_requested_exposure() {
        let baseline = policy(&["state::get"], &[]);
        let mut requested = policy(&["*"], &[]);
        requested.expose = ExposeMode::Native;
        let clamped = clamp_policy(Some(&baseline), Some(&requested)).unwrap();
        assert_eq!(clamped.expose, ExposeMode::Native);
    }

    #[test]
    fn ask_clamp_intersects_an_exact_id_request() {
        // The exact-match arm of glob_covered, exercised directly on clamp.
        let baseline = policy(&["state::get", "state::list"], &[]);
        let requested = policy(&["state::get", "state::set"], &[]);
        let compiled =
            CompiledPolicy::from(clamp_policy(Some(&baseline), Some(&requested)).as_ref());
        assert!(compiled.allows("state::get")); // in both
        assert!(!compiled.allows("state::list")); // baseline only — intersection, not union
        assert!(!compiled.allows("state::set")); // requested only — outside baseline
    }

    #[test]
    fn ask_clamp_preserves_a_baseline_deny() {
        // An operator-configured baseline deny must survive the clamp (the doc
        // promises denies are unioned); every other clamp test uses deny: [].
        let baseline = policy(&["state::get", "state::list"], &["state::list"]);
        let requested = policy(&["*"], &[]);
        let compiled =
            CompiledPolicy::from(clamp_policy(Some(&baseline), Some(&requested)).as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("state::list")); // baseline deny bites through the clamp
    }

    #[test]
    fn ask_clamp_wildcard_baseline_preserves_narrow_request() {
        let baseline = policy(&["*"], &[]);
        let requested = policy(&["state::get"], &[]);
        let compiled =
            CompiledPolicy::from(clamp_policy(Some(&baseline), Some(&requested)).as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("state::set"));
    }

    #[test]
    fn ask_clamp_deny_only_policies_union_denies() {
        let baseline = deny_only(&["configuration::*"]);
        let requested = deny_only(&["router::chat"]);
        let clamped = clamp_policy(Some(&baseline), Some(&requested)).unwrap();
        assert!(clamped.allow.is_none());
        let compiled = CompiledPolicy::from(Some(&clamped));
        assert!(compiled.allows("shell::run"));
        assert!(!compiled.allows("configuration::set"));
        assert!(!compiled.allows("router::chat"));
    }

    #[test]
    fn ask_clamp_under_approximates_a_glob_baseline_fail_closed() {
        // Documented conservative behavior: a glob baseline entry survives only
        // when a request glob COVERS it, so a narrower exact request against a
        // glob baseline denies everything. Surprising but fail-closed, never open.
        let baseline = policy(&["coder::*"], &[]);
        let requested = policy(&["coder::read-file"], &[]);
        let compiled =
            CompiledPolicy::from(clamp_policy(Some(&baseline), Some(&requested)).as_ref());
        assert!(!compiled.allows("coder::read-file"));
    }

    use crate::types::content::ContentBlock;
    use crate::types::message::{empty_assistant, AssistantMessage};
    use serde_json::json;

    fn assistant_with(calls: Vec<ContentBlock>) -> AssistantMessage {
        let mut m = empty_assistant("anthropic", "claude");
        m.content = calls;
        m
    }

    #[test]
    fn agent_trigger_call_is_unwrapped_to_target() {
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: json!({ "function": "shell::run", "payload": { "cmd": "ls" } }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "shell::run");
        assert_eq!(calls[0].kind, CallKind::Trigger);
        assert_eq!(calls[0].arguments, json!({ "cmd": "ls" }));
    }

    // A wrapper call whose arguments carry no resolvable `function` (null —
    // e.g. a local model emitted unparseable args the provider degraded)
    // keeps the wrapper name as its target: the dispatch loop matches that
    // sentinel and fails locally instead of triggering `agent_trigger` on
    // the engine (a guaranteed function_not_found).
    #[test]
    fn wrapper_call_without_target_keeps_wrapper_sentinel() {
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: Value::Null,
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].function_id, AGENT_TRIGGER_NAME);
        assert_eq!(calls[0].kind, CallKind::Trigger);
    }

    #[test]
    fn wrapper_truncation_marker_survives_payload_unwrap() {
        // Salvage of a max_tokens-cut wrapper can keep a complete `payload`
        // beside the `_partial` marker; the unwrapped call must still carry
        // it so the dispatch loop refuses to execute possibly-partial intent.
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: json!({
                "function": "state::set",
                "payload": { "scope": "s", "key": "k" },
                "_partial": true
            }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].function_id, "state::set");
        assert_eq!(calls[0].arguments["_partial"], true);
        assert_eq!(calls[0].arguments["scope"], "s");
    }

    #[test]
    fn flattened_agent_trigger_args_are_hoisted_into_payload() {
        // The model put the target's arguments beside `function` instead of
        // inside `payload` — they must reach the target, not vanish into {}.
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: json!({ "function": "engine::register_trigger", "trigger_type": "state", "config": { "key": "k" } }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].function_id, "engine::register_trigger");
        assert_eq!(
            calls[0].arguments,
            json!({ "trigger_type": "state", "config": { "key": "k" } })
        );
    }

    #[test]
    fn native_shaped_call_under_agent_trigger_exposure_keeps_arguments() {
        // The model hallucinated the target id as the tool name with real args.
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: "engine::triggers::info".into(),
            arguments: json!({ "id": "state" }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].function_id, "engine::triggers::info");
        assert_eq!(calls[0].arguments, json!({ "id": "state" }));
    }

    #[test]
    fn stringified_payload_object_is_recovered() {
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: json!({ "function": "shell::run", "payload": "{\"cmd\":\"ls\"}" }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].arguments, json!({ "cmd": "ls" }));
    }

    #[test]
    fn stringified_payload_with_trailing_garbage_is_recovered() {
        // The live failure shape: a valid stringified object plus a stray `}`.
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: json!({ "function": "state::set", "payload": "{\"key\":\"a\",\"value\":{\"x\":1}}}" }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(
            calls[0].arguments,
            json!({ "key": "a", "value": { "x": 1 } })
        );
    }

    #[test]
    fn non_object_string_payload_is_left_verbatim() {
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: AGENT_TRIGGER_NAME.into(),
            arguments: json!({ "function": "shell::run", "payload": "not json" }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].arguments, json!("not json"));
    }

    #[test]
    fn native_call_uses_block_function_id_directly() {
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: "fs::read".into(),
            arguments: json!({ "path": "/etc/hosts" }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::Native);
        assert_eq!(calls[0].function_id, "fs::read");
        assert_eq!(calls[0].arguments, json!({ "path": "/etc/hosts" }));
    }

    #[test]
    fn submit_result_is_classified_separately() {
        let msg = assistant_with(vec![ContentBlock::FunctionCall {
            id: "fc_1".into(),
            function_id: SUBMIT_RESULT_NAME.into(),
            arguments: json!({ "category": "billing" }),
        }]);
        let calls = plan_calls(&msg, ExposeMode::AgentTrigger);
        assert_eq!(calls[0].kind, CallKind::SubmitResult);
        assert_eq!(calls[0].arguments, json!({ "category": "billing" }));
    }

    #[test]
    fn subset_none_parent_yields_none() {
        assert!(subset_policy(None, Some(&policy(&["*"], &[]))).is_none());
    }

    #[test]
    fn subset_inherits_parent_allow_when_request_omits_it() {
        let parent = policy(&["shell::*", "fs::read"], &["shell::rm"]);
        let child = subset_policy(Some(&parent), None).unwrap();
        assert_eq!(
            child.allow,
            Some(vec!["shell::*".to_string(), "fs::read".to_string()])
        );
        assert_eq!(child.deny, vec!["shell::rm"]);
    }

    #[test]
    fn subset_keeps_only_requested_globs_the_parent_covers() {
        let parent = policy(&["shell::*"], &[]);
        let requested = policy(&["shell::run", "fs::read"], &["shell::rm"]);
        let child = subset_policy(Some(&parent), Some(&requested)).unwrap();
        // shell::run is covered by shell::*; fs::read is not — it is dropped.
        assert_eq!(child.allow, Some(vec!["shell::run".to_string()]));
        // deny is the union of parent and requested.
        assert_eq!(child.deny, vec!["shell::rm"]);
    }

    #[test]
    fn subset_child_cannot_escalate_to_star() {
        let parent = policy(&["shell::*"], &[]);
        let requested = policy(&["*"], &[]);
        let child = subset_policy(Some(&parent), Some(&requested)).unwrap();
        // "*" is not covered by "shell::*" → dropped → child allows nothing.
        assert_eq!(child.allow, Some(vec![]));
        assert!(!CompiledPolicy::from(Some(&child)).allows("fs::read"));
    }

    #[test]
    fn subset_deny_only_child_inherits_and_adds_denies() {
        let parent = deny_only(&["configuration::*"]);
        let requested = deny_only(&["router::chat"]);
        let child = subset_policy(Some(&parent), Some(&requested)).unwrap();
        assert!(child.allow.is_none());
        let compiled = CompiledPolicy::from(Some(&child));
        assert!(compiled.allows("shell::run"));
        assert!(!compiled.allows("configuration::set"));
        assert!(!compiled.allows("router::chat"));
    }
}
