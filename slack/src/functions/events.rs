//! `slack::events` — Slack Events API ingress (HMAC-verified raw HTTP handler).

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::Value;

use crate::deps::Deps;
use crate::httpio::Verified;
use crate::types::SlackEnvelope;
use crate::{dispatch, httpio};

pub const EVENTS_ID: &str = "slack::events";

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let deps = deps.clone();
    iii.register_function(
        EVENTS_ID,
        RegisterFunction::new_async(move |input: Value| {
            let deps = deps.clone();
            async move {
                handle(&deps, input).await;
                Ok::<_, Error>(Value::Null)
            }
        })
        .description("Slack Events API ingress (signature-verified)."),
    );
}

async fn handle(deps: &Arc<Deps>, input: Value) {
    let secret = deps.cfg().await.signing_secret.clone();
    let Verified::Ok(incoming, writer) =
        httpio::read_verified(&deps.iii, &input, secret.as_deref()).await
    else {
        return;
    };

    let env: SlackEnvelope = match serde_json::from_slice(&incoming.raw) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "slack::events: invalid JSON body");
            httpio::respond(&writer, 400, "text/plain", b"bad request").await;
            return;
        }
    };

    match env {
        SlackEnvelope::UrlVerification { challenge } => {
            httpio::respond(&writer, 200, "text/plain", challenge.as_bytes()).await;
        }
        SlackEnvelope::EventCallback(cb) => {
            // Ack within Slack's 3s window, then process asynchronously.
            httpio::respond(&writer, 200, "application/json", b"{\"ok\":true}").await;
            let deps = deps.clone();
            tokio::spawn(async move {
                dispatch::process_event(&deps, *cb).await;
            });
        }
        SlackEnvelope::Other => {
            httpio::respond(&writer, 200, "application/json", b"{\"ok\":true}").await;
        }
    }
}
