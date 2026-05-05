//! Harness meta-worker. Composes the modular workers via `iii.worker.yaml`
//! runtime dependencies; this lib registers `harness::status` plus a
//! browser-facing `bridge::trigger` HTTP route that forwards arbitrary
//! `{function_id, payload}` calls onto the iii bus.

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, Value,
    III,
};
use serde_json::json;

pub const EXPECTED_WORKERS: &[&str] = &[
    "turn-orchestrator",
    "provider-router",
    "context-compaction",
    "session-tree",
    "session-corpus",
    "document-extract",
    "models-catalog",
    "auth-credentials",
    "auth-rbac",
    "audit-log",
    "policy-denylist",
    "dlp-scrubber",
    "guardrails",
    "llm-budget",
    "session-inbox",
    "hook-fanout",
    "shell-bash",
    "shell-filesystem",
    "shell-subagent",
    "provider-cli",
    "provider-anthropic",
    "provider-openai",
];

pub struct HarnessFunctionRefs {
    pub status: FunctionRef,
    pub bridge: FunctionRef,
}

impl HarnessFunctionRefs {
    pub fn unregister_all(self) {
        self.status.unregister();
        self.bridge.unregister();
    }
}

pub async fn register_with_iii(iii: &III) -> anyhow::Result<HarnessFunctionRefs> {
    let status = iii.register_function((
        RegisterFunctionMessage::with_id("harness::status".into()).with_description(
            "Returns the harness bundle name, version, and the list of expected runtime workers."
                .into(),
        ),
        |_payload: Value| async move {
            Ok::<_, IIIError>(json!({
                "ok": true,
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
                "expected_workers": EXPECTED_WORKERS,
            }))
        },
    ));

    let iii_for_bridge = iii.clone();
    let bridge = iii.register_function((
        RegisterFunctionMessage::with_id("bridge::trigger".into()).with_description(
            "Forward {function_id, payload} to iii.trigger and return the result. \
             Used by harness/web/ to reach the bus over HTTP."
                .into(),
        ),
        move |input: Value| {
            let iii = iii_for_bridge.clone();
            async move {
                // HTTP trigger wraps the request body as { body, query_params, headers, ... }.
                // Direct bus callers send { function_id, payload } at the top level.
                let body = input.get("body").cloned().unwrap_or(input);
                let function_id = body
                    .get("function_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing function_id".into()))?
                    .to_string();
                let inner = body.get("payload").cloned().unwrap_or_else(|| json!({}));
                let result = iii
                    .trigger(TriggerRequest {
                        function_id,
                        payload: inner,
                        action: None,
                        timeout_ms: None,
                    })
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                Ok::<_, IIIError>(json!({
                    "status_code": 200,
                    "headers": { "content-type": "application/json" },
                    "body": result,
                }))
            }
        },
    ));

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "bridge::trigger".into(),
        config: json!({ "api_path": "bridge/trigger", "http_method": "POST" }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(HarnessFunctionRefs { status, bridge })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_workers_is_unique_and_non_empty() {
        assert!(!EXPECTED_WORKERS.is_empty());
        let mut seen = std::collections::HashSet::new();
        for w in EXPECTED_WORKERS {
            assert!(seen.insert(*w), "duplicate worker in EXPECTED_WORKERS: {w}");
        }
    }
}
