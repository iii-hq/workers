//! State-store and function-executor traits, plus their iii-backed
//! implementations and the `__from_approval` marker plumbing.
//!
//! The traits exist purely as test seams — unit tests swap in
//! `InMemoryStateBus` / `FakeExecutor` while production code uses the
//! `Iii*` implementations that call iii directly. No new abstractions
//! beyond what's needed for that seam.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

use crate::config::InterceptorRule;

/// Look up the [`InterceptorRule`] for `function_id`, if one is configured.
/// Pure helper; no I/O. Used by the gate's intercept flow and by the
/// production [`IiiFunctionExecutor`] to decide whether to inject the
/// `__from_approval` marker.
pub(crate) fn rule_for<'a>(
    rules: &'a [InterceptorRule],
    function_id: &str,
) -> Option<&'a InterceptorRule> {
    rules.iter().find(|r| r.function_id == function_id)
}

/// Stamp the `__from_approval` marker onto a function call's args when the
/// rule asks for it. The marker carries `{ call_id, session_id }` so the
/// target function can validate the call came through approval-gate (via
/// `approval::lookup_record`) instead of via direct trigger bypass.
///
/// Idempotent on shape: object args get the marker merged in; null args
/// become `{ __from_approval: ... }`; any other shape (array, scalar)
/// gets wrapped as `{ payload, __from_approval: ... }` so it stays
/// recoverable on the target side.
pub(crate) fn merge_from_approval_marker_if_needed(
    inject: bool,
    args: Value,
    function_call_id: &str,
    session_id: &str,
) -> Value {
    if !inject {
        return args;
    }
    let marker = json!({
        "call_id": function_call_id,
        "session_id": session_id,
    });
    match args {
        Value::Object(mut m) => {
            m.insert("__from_approval".into(), marker);
            Value::Object(m)
        }
        other if other.is_null() => json!({ "__from_approval": marker }),
        other => json!({
            "payload": other,
            "__from_approval": marker,
        }),
    }
}

/// Abstraction over the iii state bus — the kv layer where pending and
/// resolved approval records live. Exists so unit tests can swap in a
/// `BTreeMap`-backed fake; production uses [`IiiStateBus`].
#[async_trait]
pub trait StateBus: Send + Sync {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), IIIError>;
    async fn get(&self, scope: &str, key: &str) -> Option<Value>;
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value>;
}

/// Invokes an iii function with arguments and returns its result or an
/// error string. Abstracted so tests can stub the underlying call.
#[async_trait]
pub trait FunctionExecutor: Send + Sync {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        function_call_id: &str,
        session_id: &str,
    ) -> Result<Value, String>;
}

/// Production [`FunctionExecutor`] backed by `iii.trigger`.
pub struct IiiFunctionExecutor {
    pub iii: III,
    pub rules: Arc<Vec<InterceptorRule>>,
}

#[async_trait]
impl FunctionExecutor for IiiFunctionExecutor {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        function_call_id: &str,
        session_id: &str,
    ) -> Result<Value, String> {
        let inject =
            rule_for(self.rules.as_slice(), function_id).is_some_and(|r| r.inject_approval_marker);
        let payload =
            merge_from_approval_marker_if_needed(inject, args, function_call_id, session_id);
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// Production [`StateBus`] backed by iii's `state::*` builtins.
pub struct IiiStateBus(pub III);

#[async_trait]
impl StateBus for IiiStateBus {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), IIIError> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": scope, "key": key, "value": value }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map(|_| ())
    }
    async fn get(&self, scope: &str, key: &str) -> Option<Value> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: None,
            })
            .await
            .ok()
            .filter(|v| !v.is_null())
    }
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
        let resp = self
            .0
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": scope, "prefix": prefix }),
                action: None,
                timeout_ms: None,
            })
            .await
            .unwrap_or_else(|_| json!({ "items": [] }));
        // Engine may return either {"items": [...]} or a plain Array.
        if let Some(arr) = resp.as_array() {
            return arr.clone();
        }
        resp.get("items")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.get("value").cloned().unwrap_or(entry))
            .collect()
    }
}

/// Return the list of function ids whose interceptor asks the gate to
/// inject `__from_approval` without asserting that the target validates it.
/// Empty list ⇒ config is safe to register. Pure — exposed for tests and
/// for the boot-time check in `register`.
pub fn unverified_marker_targets(rules: &[InterceptorRule]) -> Vec<&str> {
    rules
        .iter()
        .filter(|r| r.inject_approval_marker && !r.marker_target_verified)
        .map(|r| r.function_id.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_from_approval_inserts_marker_when_inject_true() {
        let m = merge_from_approval_marker_if_needed(
            true,
            json!({"command": "git"}),
            "call-1",
            "sess-1",
        );
        let inner = m.get("__from_approval").unwrap();
        assert_eq!(inner["call_id"], "call-1");
        assert_eq!(inner["session_id"], "sess-1");
        assert_eq!(m["command"], "git");
    }

    #[test]
    fn merge_from_approval_noop_when_inject_false() {
        let j = json!({"a": 1});
        let out = merge_from_approval_marker_if_needed(false, j.clone(), "c", "s");
        assert_eq!(out, j);
    }

    #[test]
    fn merge_from_approval_wraps_null_args_in_marker_only() {
        let m = merge_from_approval_marker_if_needed(true, Value::Null, "c1", "s1");
        let obj = m.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("__from_approval"));
    }

    #[test]
    fn merge_from_approval_wraps_scalar_args_in_payload() {
        let out = merge_from_approval_marker_if_needed(true, json!("scalar"), "c1", "s1");
        assert_eq!(out["payload"], json!("scalar"));
        assert_eq!(out["__from_approval"]["call_id"], "c1");
        assert_eq!(out["__from_approval"]["session_id"], "s1");
    }

    #[test]
    fn rule_for_returns_matching_rule() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: Some("shell::classify_argv".into()),
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: true,
            },
            InterceptorRule {
                function_id: "other::fn".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
                marker_target_verified: false,
            },
        ];
        let r = rule_for(&rules, "shell::exec").expect("match");
        assert_eq!(r.classifier.as_deref(), Some("shell::classify_argv"));
        assert!(r.inject_approval_marker);
    }

    #[test]
    fn rule_for_returns_none_when_absent() {
        let rules = vec![InterceptorRule {
            function_id: "x::y".into(),
            classifier: None,
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
            marker_target_verified: false,
        }];
        assert!(rule_for(&rules, "missing::id").is_none());
    }
}
