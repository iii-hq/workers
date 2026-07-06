//! The fail-closed dispatch policy and the invocation-schema surface
//! (harness.md § Functions). The allow/deny globs are structural and final —
//! hooks run only after they pass.

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

/// A compiled allow/deny matcher. Fail-closed: a call is allowed only when it
/// matches an `allow` glob and no `deny` glob; an absent or empty allow-list
/// denies everything.
pub struct CompiledPolicy {
    allow: GlobSet,
    deny: GlobSet,
    allow_empty: bool,
}

impl CompiledPolicy {
    pub fn from(policy: Option<&FunctionPolicy>) -> Self {
        match policy {
            Some(p) => CompiledPolicy {
                allow: build_set(&p.allow),
                deny: build_set(&p.deny),
                allow_empty: p.allow.is_empty(),
            },
            None => CompiledPolicy {
                allow: GlobSet::empty(),
                deny: GlobSet::empty(),
                allow_empty: true,
            },
        }
    }

    /// Whether `function_id` may be dispatched (fail-closed).
    pub fn allows(&self, function_id: &str) -> bool {
        if self.allow_empty {
            return false;
        }
        self.allow.is_match(function_id) && !self.deny.is_match(function_id)
    }
}

/// Subset a child's requested policy against the parent's (harness.md §
/// Sub-agents: narrow, never escalate). The child's allow is the requested
/// globs kept only where the parent's allow covers them (or the parent's allow
/// inherited when the child requests none); deny is the union; a `None` parent
/// policy yields `None` (deny all) so an un-empowered parent can't empower a
/// child.
pub fn subset_policy(
    parent: Option<&FunctionPolicy>,
    requested: Option<&FunctionPolicy>,
) -> Option<FunctionPolicy> {
    let parent = parent?;
    let allow = match requested {
        Some(r) if !r.allow.is_empty() => r
            .allow
            .iter()
            .filter(|g| glob_covered(g, &parent.allow))
            .cloned()
            .collect(),
        _ => parent.allow.clone(),
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

/// Is a child allow glob covered by the parent's allow set? Conservative: an
/// exact match, a parent `*`, or a parent `<prefix>::*` covering the child.
fn glob_covered(child: &str, parent_globs: &[String]) -> bool {
    parent_globs.iter().any(|p| {
        p == child || p == "*" || (p.ends_with("::*") && child.starts_with(&p[..p.len() - 1]))
    })
}

fn build_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(glob) = Glob::new(p) {
            builder.add(glob);
        } else {
            tracing::warn!(pattern = %p, "ignoring invalid function glob");
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// The default dispatch policy for code mode (harness.md § Exposure modes):
/// the curated coding surface, natively exposed, applied when a code-mode
/// send/spawn carries no explicit `functions` policy. Sub-agents still
/// narrow through [`subset_policy`] — a code-mode child never escalates
/// past its parent.
pub fn code_mode_policy() -> FunctionPolicy {
    FunctionPolicy {
        allow: [
            "coder::*",
            "shell::exec",
            "shell::exec_bg",
            "shell::kill",
            "shell::status",
            "harness::spawn",
            "harness::todo",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        deny: vec![],
        expose: ExposeMode::Native,
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
            let (target, payload) =
                if function_id == AGENT_TRIGGER_NAME || expose == ExposeMode::AgentTrigger {
                    let target = arguments
                        .get("function")
                        .and_then(Value::as_str)
                        .unwrap_or(function_id)
                        .to_string();
                    let payload = arguments
                        .get("payload")
                        .cloned()
                        .unwrap_or(Value::Object(Default::default()));
                    (target, payload)
                } else {
                    (function_id.clone(), arguments.clone())
                };
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
            allow: allow.iter().map(|s| s.to_string()).collect(),
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
    fn empty_allow_denies_everything() {
        let p = CompiledPolicy::from(Some(&policy(&[], &[])));
        assert!(!p.allows("shell::run"));
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
    fn code_mode_policy_allows_the_coding_surface_only() {
        let p = code_mode_policy();
        assert_eq!(p.expose, ExposeMode::Native);
        let c = CompiledPolicy::from(Some(&p));
        for allowed in [
            "coder::context",
            "coder::read-file",
            "coder::update-file",
            "coder::apply-patch",
            "coder::worktree-add",
            "coder::search",
            "shell::exec",
            "shell::exec_bg",
            "shell::kill",
            "shell::status",
            "harness::spawn",
            "harness::todo",
        ] {
            assert!(c.allows(allowed), "{allowed} must be allowed");
        }
        for denied in [
            "engine::functions::list",
            "harness::send",
            "worker::add",
            "shell::fs::rm",
            "shell::list",
            "state::set",
        ] {
            assert!(!c.allows(denied), "{denied} must be denied");
        }
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
        assert_eq!(child.allow, vec!["shell::*", "fs::read"]);
        assert_eq!(child.deny, vec!["shell::rm"]);
    }

    #[test]
    fn subset_keeps_only_requested_globs_the_parent_covers() {
        let parent = policy(&["shell::*"], &[]);
        let requested = policy(&["shell::run", "fs::read"], &["shell::rm"]);
        let child = subset_policy(Some(&parent), Some(&requested)).unwrap();
        // shell::run is covered by shell::*; fs::read is not — it is dropped.
        assert_eq!(child.allow, vec!["shell::run"]);
        // deny is the union of parent and requested.
        assert_eq!(child.deny, vec!["shell::rm"]);
    }

    #[test]
    fn subset_child_cannot_escalate_to_star() {
        let parent = policy(&["shell::*"], &[]);
        let requested = policy(&["*"], &[]);
        let child = subset_policy(Some(&parent), Some(&requested)).unwrap();
        // "*" is not covered by "shell::*" → dropped → child allows nothing.
        assert!(child.allow.is_empty());
        assert!(!CompiledPolicy::from(Some(&child)).allows("fs::read"));
    }
}
