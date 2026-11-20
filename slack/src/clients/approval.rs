//! `approval::*` — resolve held tool calls (optional `approval-gate` worker).

use iii_sdk::CallRequest;
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use serde_json::json;

fn resolve_payload(
    session_id: &str,
    function_call_id: &str,
    allow: bool,
    namespace: Option<&str>,
) -> serde_json::Value {
    let mut payload = json!({
        "session_id": session_id,
        "function_call_id": function_call_id,
        "decision": if allow { "allow" } else { "deny" },
    });
    if let Some(ns) = namespace {
        payload["namespace"] = json!(ns);
    }
    payload
}

fn approve_always_payload(
    session_id: &str,
    function_id: &str,
    namespace: Option<&str>,
) -> serde_json::Value {
    let mut payload = json!({ "session_id": session_id, "function_id": function_id });
    if let Some(ns) = namespace {
        payload["namespace"] = json!(ns);
    }
    payload
}

pub async fn resolve(
    iii: &IIIClient,
    session_id: &str,
    function_call_id: &str,
    allow: bool,
    namespace: Option<&str>,
    timeout_ms: u64,
) -> Result<(), Error> {
    let payload = resolve_payload(session_id, function_call_id, allow, namespace);
    iii.call(CallRequest {
        function_id: "approval::resolve".into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

pub async fn approve_always(
    iii: &IIIClient,
    session_id: &str,
    function_id: &str,
    namespace: Option<&str>,
    timeout_ms: u64,
) -> Result<(), Error> {
    let payload = approve_always_payload(session_id, function_id, namespace);
    iii.call(CallRequest {
        function_id: "approval::approve-always".into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApprovalCallback;

    #[test]
    fn requests_omit_namespace_when_none() {
        assert!(
            resolve_payload("s", "c", true, None)
                .get("namespace")
                .is_none()
        );
        assert!(
            approve_always_payload("s", "f", None)
                .get("namespace")
                .is_none()
        );
    }

    #[test]
    fn requests_carry_namespace_when_set() {
        assert_eq!(
            resolve_payload("s", "c", false, Some("laptop"))["namespace"],
            "laptop"
        );
        assert_eq!(
            approve_always_payload("s", "f", Some("laptop"))["namespace"],
            "laptop"
        );
    }

    #[test]
    fn old_callback_without_namespace_still_parses() {
        let old = json!({
            "session_id": "s", "function_call_id": "c", "function_id": "f",
            "channel": "C1", "message_ts": "1.2",
        });
        let cb: ApprovalCallback = serde_json::from_value(old).unwrap();
        assert!(cb.namespace.is_none());
        assert!(
            serde_json::to_value(&cb)
                .unwrap()
                .get("namespace")
                .is_none()
        );
    }
}
