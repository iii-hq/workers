//! Slack Socket Mode: dial out to Slack over a WebSocket and receive events with
//! no public URL. Selected when `app_token` (`xapp-`) is configured. Each frame
//! is acknowledged within Slack's 3-second window, then routed through the same
//! `dispatch` handlers as the HTTP ingress.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::errors::Error;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::deps::Deps;
use crate::dispatch;
use crate::types::{EventCallback, InteractionPayload};

#[derive(Debug, Deserialize)]
struct SocketFrame {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    envelope_id: Option<String>,
    #[serde(default)]
    payload: Option<Value>,
}

/// Run the Socket Mode loop until cancelled, reconnecting with backoff.
pub async fn run(deps: Arc<Deps>, cancel: CancellationToken) {
    let mut backoff = 1u64;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        match connect_once(&deps, &cancel).await {
            Ok(()) => backoff = 1, // clean disconnect — reconnect promptly
            Err(e) => {
                tracing::warn!(error = %e, backoff, "socket mode connection failed");
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = sleep(Duration::from_secs(backoff)) => {}
                }
                backoff = (backoff * 2).min(30);
            }
        }
    }
}

async fn connect_once(deps: &Arc<Deps>, cancel: &CancellationToken) -> Result<(), Error> {
    let url = open_connection(deps).await?;
    let (ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| Error::Handler(format!("socket connect: {e}")))?;
    tracing::info!("socket mode connected");
    let (mut write, mut read) = ws.split();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            msg = read.next() => {
                let Some(msg) = msg else { return Ok(()); };
                let msg = msg.map_err(|e| Error::Handler(format!("socket read: {e}")))?;
                let Message::Text(text) = msg else {
                    if matches!(msg, Message::Close(_)) { return Ok(()); }
                    continue;
                };
                let frame: SocketFrame = match serde_json::from_str(&text) {
                    Ok(f) => f,
                    Err(e) => { tracing::warn!(error = %e, "socket frame parse"); continue; }
                };

                match frame.kind.as_deref() {
                    Some("hello") => tracing::info!("socket mode hello"),
                    Some("disconnect") => {
                        tracing::info!("socket mode disconnect frame; reconnecting");
                        return Ok(());
                    }
                    _ => {}
                }

                // Acknowledge first (3s rule), then dispatch.
                if let Some(id) = &frame.envelope_id {
                    let ack = json!({ "envelope_id": id }).to_string();
                    if let Err(e) = write.send(Message::Text(ack.into())).await {
                        tracing::warn!(error = %e, "socket ack failed");
                        return Ok(());
                    }
                    dispatch_frame(deps, &frame).await;
                }
            }
        }
    }
}

async fn dispatch_frame(deps: &Arc<Deps>, frame: &SocketFrame) {
    let Some(payload) = frame.payload.clone() else {
        return;
    };
    match frame.kind.as_deref() {
        Some("events_api") => match serde_json::from_value::<EventCallback>(payload) {
            Ok(cb) => dispatch::process_event(deps, cb).await,
            Err(e) => tracing::warn!(error = %e, "socket events_api payload"),
        },
        Some("interactive") => match serde_json::from_value::<InteractionPayload>(payload) {
            Ok(p) => dispatch::process_interaction(deps, p).await,
            Err(e) => tracing::warn!(error = %e, "socket interactive payload"),
        },
        _ => {}
    }
}

/// `apps.connections.open` with the app-level token returns a fresh wss URL.
async fn open_connection(deps: &Arc<Deps>) -> Result<String, Error> {
    let token = deps
        .cfg()
        .await
        .app_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| Error::Handler("socket mode: app_token not configured".into()))?;
    let resp: Value = deps
        .http
        .post("https://slack.com/api/apps.connections.open")
        .bearer_auth(&token)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| Error::Handler(format!("apps.connections.open http: {}", e.without_url())))?
        .json()
        .await
        .map_err(|e| Error::Handler(format!("apps.connections.open json: {e}")))?;
    if resp.get("ok").and_then(Value::as_bool) != Some(true) {
        let err = resp
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(Error::Handler(format!(
            "apps.connections.open failed: {err}"
        )));
    }
    resp.get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Handler("apps.connections.open: missing url".into()))
}
