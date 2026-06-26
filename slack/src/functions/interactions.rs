//! `slack::interactions` — Block Kit interactivity ingress (HMAC-verified).
//! Slack posts `application/x-www-form-urlencoded` with a `payload=<json>` field.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::Value;

use crate::deps::Deps;
use crate::httpio::Verified;
use crate::types::InteractionPayload;
use crate::{dispatch, httpio};

pub const INTERACTIONS_ID: &str = "slack::interactions";

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let deps = deps.clone();
    iii.register_function(
        INTERACTIONS_ID,
        RegisterFunction::new_async(move |input: Value| {
            let deps = deps.clone();
            async move {
                handle(&deps, input).await;
                Ok::<_, Error>(Value::Null)
            }
        })
        .description("Slack interactivity ingress (block actions / view submissions)."),
    );
}

async fn handle(deps: &Arc<Deps>, input: Value) {
    let secret = deps.cfg().await.signing_secret.clone();
    let Verified::Ok(incoming, writer) =
        httpio::read_verified(&deps.iii, &input, secret.as_deref()).await
    else {
        return;
    };

    let Some(json) = extract_payload(&incoming.raw) else {
        httpio::respond(&writer, 400, "text/plain", b"missing payload").await;
        return;
    };
    let payload: InteractionPayload = match serde_json::from_str(&json) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "slack::interactions: invalid payload json");
            httpio::respond(&writer, 400, "text/plain", b"bad payload").await;
            return;
        }
    };

    // Ack immediately, then process.
    httpio::respond(&writer, 200, "text/plain", b"").await;
    let deps = deps.clone();
    tokio::spawn(async move {
        dispatch::process_interaction(&deps, payload).await;
    });
}

/// Extract and url-decode the `payload` field from a form-urlencoded body.
fn extract_payload(raw: &[u8]) -> Option<String> {
    let body = std::str::from_utf8(raw).ok()?;
    for pair in body.split('&') {
        if let Some(value) = pair.strip_prefix("payload=") {
            return urlencoding::decode(value).ok().map(|c| c.into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_urlencoded_payload() {
        let raw = b"payload=%7B%22type%22%3A%22block_actions%22%7D";
        assert_eq!(
            extract_payload(raw).as_deref(),
            Some("{\"type\":\"block_actions\"}")
        );
    }

    #[test]
    fn missing_payload_is_none() {
        assert!(extract_payload(b"foo=bar").is_none());
    }
}
