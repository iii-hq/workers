//! `web::fetch` registration + handler. Input is a raw `Value` (parsed
//! internally) so a malformed payload returns `invalid_payload` instead of
//! an SDK-level error. The whole handler is wrapped in catch_unwind so a
//! panic becomes a transport_error envelope rather than a dropped response.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use iii_sdk::{IIIError, RegisterFunction, III};
use serde_json::{json, Value};

use crate::config::{SharedConfig, WebConfig};
use crate::fetch::execute_fetch;
use crate::schemas::{request_schema, FetchPayload, TOOL_DESCRIPTION};

/// Parse + validate + execute. Always returns an envelope (never errors).
pub async fn handle(payload: Value, cfg: &WebConfig) -> Value {
    let parsed: FetchPayload = match serde_json::from_value(payload) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": "invalid_payload", "message": e.to_string() }),
    };
    if let Err(msg) = parsed.validate() {
        return json!({ "ok": false, "error": "invalid_payload", "message": msg });
    }
    execute_fetch(parsed, cfg).await
}

pub fn register(iii: &Arc<III>, shared: &SharedConfig) {
    let shared = shared.clone();
    iii.register_function(
        "web::fetch",
        RegisterFunction::new_async(move |payload: Value| {
            let cfg = shared.load_full(); // single snapshot per request
            async move {
                // Wrap the whole handler: a panic must become an envelope,
                // not a dropped response (the SDK runs us in a bare spawn
                // with no catch_unwind, so a panic would hang the caller).
                let result = AssertUnwindSafe(handle(payload, &cfg)).catch_unwind().await;
                let envelope = result.unwrap_or_else(|_| {
                    json!({ "ok": false, "error": "transport_error", "message": "internal error" })
                });
                Ok::<Value, IIIError>(envelope)
            }
        })
        .description(TOOL_DESCRIPTION)
        .request_format(request_schema()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebConfig;

    #[tokio::test]
    async fn invalid_payload_missing_url() {
        let out = handle(serde_json::json!({}), &WebConfig::default()).await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "invalid_payload");
    }

    #[tokio::test]
    async fn invalid_payload_bad_method() {
        let out = handle(serde_json::json!({ "url": "https://x.test/", "method": "FETCH" }), &WebConfig::default()).await;
        assert_eq!(out["error"], "invalid_payload");
    }

    #[tokio::test]
    async fn invalid_url_scheme() {
        let out = handle(serde_json::json!({ "url": "ftp://x.test/" }), &WebConfig::default()).await;
        assert_eq!(out["error"], "invalid_url");
    }
}
