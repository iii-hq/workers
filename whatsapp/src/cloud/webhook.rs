//! Authenticated Meta webhook ingress through the existing iii HTTP worker.
use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use hmac::{Hmac, KeyInit, Mac};
use iii_sdk::{
    IIIClient, RegisterFunction,
    channel::{ChannelReader, StreamChannelRef},
    channels::ChannelDirection,
    protocol::RegisterTriggerInput,
    trigger::Trigger,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::{
    config::CloudConfig,
    inbox::{ExternalReceipt, MessageView},
    sessions::SessionManager,
};

const VERIFY: &str = "whatsapp::cloud::webhook::verify";
const RECEIVE: &str = "whatsapp::cloud::webhook::receive";
const MAX_BODY_BYTES: usize = 3 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 16_384;
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(8);

// HTTP adds other routing fields. They are intentionally ignored, including
// `body`: its JSON projection cannot authenticate the original wire bytes.
#[derive(Default, Deserialize, JsonSchema)]
struct VerifyRequest {
    #[serde(default)]
    query_params: HashMap<String, String>,
}

#[derive(Default, Deserialize, JsonSchema)]
struct ReceiveRequest {
    #[serde(default)]
    headers: HashMap<String, String>,
    request_body: Option<StreamChannelRef>,
}

#[derive(Serialize, JsonSchema)]
struct HttpResponse {
    status_code: u16,
    headers: HashMap<String, String>,
    body: String,
}

impl HttpResponse {
    fn text(status_code: u16, body: impl Into<String>) -> Self {
        Self {
            status_code,
            // Without this header, iii HTTP JSON-quotes string responses,
            // which would invalidate Meta's verification challenge.
            headers: HashMap::from([("content-type".into(), "text/plain; charset=utf-8".into())]),
            body: body.into(),
        }
    }
}

/// Register inert routes even while disabled, so live configuration can enable
/// or disable verification and delivery without restarting native sessions.
/// The caller retains the handles and unregisters them on shutdown.
pub fn register(
    iii: &Arc<IIIClient>,
    manager: Arc<SessionManager>,
    engine_url: String,
) -> Result<Vec<Trigger>> {
    let for_verify = manager.clone();
    iii.register_function(
        VERIFY,
        RegisterFunction::new(move |request: VerifyRequest| {
            Ok::<_, iii_sdk::Error>(verify(&for_verify.config().cloud, &request.query_params))
        })
        .description("Internal: verify an authenticated Meta webhook subscription challenge")
        .metadata(json!({"internal": true, "trace_hidden": true})),
    );
    iii.register_function(
        RECEIVE,
        RegisterFunction::new_async(move |request: ReceiveRequest| {
            let manager = manager.clone();
            let engine_url = engine_url.clone();
            async move { Ok::<_, iii_sdk::Error>(receive(manager, &engine_url, request).await) }
        })
        .description("Internal: authenticate and archive a Meta Cloud API webhook delivery")
        .metadata(json!({"internal": true, "trace_hidden": true})),
    );
    let mut routes: Vec<Trigger> = Vec::new();
    for (id, method) in [(VERIFY, "GET"), (RECEIVE, "POST")] {
        match iii.register_trigger(RegisterTriggerInput::new(
            "http",
            id,
            json!({"api_path": super::webhook_path(), "http_method": method, "public_webhook": true}),
        )) {
            Ok(trigger) => routes.push(trigger),
            Err(error) => {
                for route in routes {
                    route.unregister();
                }
                return Err(error.into());
            }
        }
    }
    Ok(routes)
}

fn verify(config: &CloudConfig, query: &HashMap<String, String>) -> HttpResponse {
    if !config.enabled || config.verify_token.trim().is_empty() {
        return HttpResponse::text(503, "Webhook verification is not configured");
    }
    let valid = query
        .get("hub.mode")
        .is_some_and(|mode| mode == "subscribe")
        && query.get("hub.verify_token").is_some_and(|token| {
            token.len() <= 16_384 && token_matches(&config.verify_token, token)
        });
    if !valid {
        return HttpResponse::text(403, "Invalid verification token or mode");
    }
    match query.get("hub.challenge") {
        Some(challenge) if !challenge.is_empty() && challenge.len() <= 4096 => {
            HttpResponse::text(200, challenge)
        }
        _ => HttpResponse::text(400, "Missing or invalid verification challenge"),
    }
}

fn token_matches(expected: &str, actual: &str) -> bool {
    // Compare fixed-size MAC outputs using the crypto library's constant-time
    // comparison; no secret-dependent string equality or early byte exits.
    let mut expected_mac = Hmac::<Sha256>::new_from_slice(b"whatsapp-webhook-verify-token")
        .expect("HMAC accepts any key size");
    let mut actual_mac = expected_mac.clone();
    expected_mac.update(expected.as_bytes());
    actual_mac.update(actual.as_bytes());
    expected_mac
        .verify_slice(&actual_mac.finalize().into_bytes())
        .is_ok()
}

async fn receive(
    manager: Arc<SessionManager>,
    engine_url: &str,
    request: ReceiveRequest,
) -> HttpResponse {
    let config = manager.config();
    if !config.cloud.enabled || config.cloud.app_secret.trim().is_empty() {
        return HttpResponse::text(503, "Webhook delivery is not configured");
    }
    let Some(signature) = header(&request.headers, "x-hub-signature-256") else {
        return HttpResponse::text(403, "Invalid signature");
    };
    if signature_bytes(signature).is_none() {
        return HttpResponse::text(403, "Invalid signature");
    }
    if header(&request.headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_BODY_BYTES)
    {
        return HttpResponse::text(413, "Payload too large");
    }
    let Some(channel) = request.request_body.as_ref().filter(|channel| {
        matches!(channel.direction, ChannelDirection::Read)
            && !channel.channel_id.is_empty()
            && !channel.access_key.is_empty()
    }) else {
        return HttpResponse::text(400, "Original request body is required");
    };
    let reader = ChannelReader::new(engine_url, channel);
    let result = tokio::time::timeout(RECEIVE_TIMEOUT, async {
        let mut raw = Vec::new();
        loop {
            match reader.next_binary().await {
                Ok(Some(chunk)) => {
                    if chunk.len() > MAX_BODY_BYTES.saturating_sub(raw.len()) {
                        return HttpResponse::text(413, "Payload too large");
                    }
                    raw.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => return HttpResponse::text(503, "Unable to read delivery; retry later"),
            }
        }
        if !verify_signature(&config.cloud.app_secret, &raw, signature) {
            return HttpResponse::text(403, "Invalid signature");
        }
        let batch = match parse_delivery(&config.cloud, &raw) {
            Ok(batch) => batch,
            Err(_) => return HttpResponse::text(400, "Invalid webhook delivery"),
        };
        if batch.messages.is_empty() && batch.receipts.is_empty() {
            return HttpResponse::text(200, "Accepted");
        }
        let accepted = tokio::task::spawn_blocking(move || -> Result<()> {
            // Do not commit a delivery authenticated for an obsolete account or
            // secret after the operator has changed/disabled the integration.
            ensure!(
                manager.config().cloud == config.cloud,
                "Cloud configuration changed"
            );
            manager
                .cloud()
                .inbox(&config)?
                .record_external(&batch.messages, &batch.receipts)?;
            Ok(())
        })
        .await;
        match accepted {
            Ok(Ok(())) => HttpResponse::text(200, "Accepted"),
            _ => HttpResponse::text(503, "Unable to store delivery; retry later"),
        }
    })
    .await;
    let _ = reader.close().await;
    result.unwrap_or_else(|_| HttpResponse::text(503, "Delivery timed out; retry later"))
}

fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn signature_bytes(signature: &str) -> Option<[u8; 32]> {
    let value = signature.strip_prefix("sha256=")?;
    let mut digest = [0; 32];
    hex::decode_to_slice(value, &mut digest).ok()?;
    Some(digest)
}

fn verify_signature(secret: &str, raw: &[u8], signature: &str) -> bool {
    let Some(digest) = signature_bytes(signature) else {
        return false;
    };
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(raw);
    mac.verify_slice(&digest).is_ok()
}

#[derive(Default)]
struct DeliveryBatch {
    messages: Vec<MessageView>,
    receipts: Vec<ExternalReceipt>,
}

fn parse_delivery(config: &CloudConfig, raw: &[u8]) -> Result<DeliveryBatch> {
    ensure!(raw.len() <= MAX_BODY_BYTES, "payload exceeds limit");
    let body: Value = serde_json::from_slice(raw)?;
    ensure!(
        body["object"] == "whatsapp_business_account",
        "unexpected webhook object"
    );
    let mut batch = DeliveryBatch::default();
    for entry in body["entry"].as_array().context("missing entries")? {
        let account = entry["id"].as_str().context("missing business account")?;
        if account != config.business_account_id {
            continue;
        }
        for change in entry["changes"].as_array().context("missing changes")? {
            let field = change["field"].as_str().context("missing change field")?;
            if field != "messages" {
                continue;
            }
            let value = &change["value"];
            let number = value["metadata"]["phone_number_id"]
                .as_str()
                .context("missing phone number")?;
            if number != config.phone_number_id {
                continue;
            }
            ensure!(
                value["messaging_product"] == "whatsapp",
                "unexpected messaging product"
            );
            let mut names = HashMap::new();
            if let Some(contacts) = value.get("contacts") {
                for contact in contacts.as_array().context("invalid contacts")? {
                    let Some(name) = contact["profile"]["name"].as_str() else {
                        continue;
                    };
                    for field in ["wa_id", "user_id"] {
                        if let Some(id) = contact[field]
                            .as_str()
                            .and_then(|raw| super::normalize_chat_id(raw).ok())
                        {
                            names.insert(id, clipped(name, 512));
                        }
                    }
                }
            }
            if let Some(messages) = value.get("messages") {
                for message in messages.as_array().context("invalid messages")? {
                    ensure!(
                        batch.messages.len() + batch.receipts.len() < 1000,
                        "delivery exceeds item limit"
                    );
                    let sender = external_identity(message, "from", "from_user_id")?;
                    let message_id = message_id(message)?;
                    let seconds = message["timestamp"]
                        .as_str()
                        .context("missing timestamp")?
                        .parse::<i64>()?;
                    ensure!(seconds >= 0, "invalid timestamp");
                    let timestamp_ms = seconds
                        .checked_mul(1000)
                        .context("timestamp exceeds limit")?;
                    let (kind, text) = preview(message)?;
                    batch.messages.push(MessageView {
                        message_id,
                        chat_id: super::storage_chat_id(&sender)?,
                        sender_name: names.get(&sender).cloned(),
                        sender_id: sender,
                        from_me: false,
                        timestamp_ms,
                        kind: kind.into(),
                        text,
                        source: "live".into(),
                        status: None,
                    });
                }
            }
            if let Some(statuses) = value.get("statuses") {
                for status in statuses.as_array().context("invalid statuses")? {
                    let state = status["status"].as_str().context("missing status")?;
                    if !matches!(state, "sent" | "delivered" | "read" | "failed") {
                        continue;
                    }
                    ensure!(
                        batch.messages.len() + batch.receipts.len() < 1000,
                        "delivery exceeds item limit"
                    );
                    let recipient = external_identity(status, "recipient_id", "recipient_user_id")?;
                    batch.receipts.push(ExternalReceipt {
                        message_id: message_id(status)?,
                        chat_id: super::storage_chat_id(&recipient)?,
                        status: state.into(),
                    });
                }
            }
        }
    }
    Ok(batch)
}

fn external_identity(value: &Value, phone: &str, user_id: &str) -> Result<String> {
    let raw = value
        .get(phone)
        .filter(|value| !value.is_null())
        .or_else(|| value.get(user_id))
        .and_then(Value::as_str)
        .context("missing identity")?;
    super::normalize_chat_id(raw)
}

fn message_id(value: &Value) -> Result<String> {
    let id = value["id"].as_str().context("missing message ID")?;
    ensure!(
        !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control),
        "invalid message ID"
    );
    Ok(id.to_owned())
}

fn clipped(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn preview(message: &Value) -> Result<(&'static str, String)> {
    let kind = message["type"].as_str().context("missing message type")?;
    let (kind, text) = match kind {
        "text" => (
            "text",
            message["text"]["body"].as_str().context("missing text")?,
        ),
        "image" | "video" | "audio" | "document" | "sticker" => {
            ensure!(message[kind].is_object(), "missing media description");
            let caption = message[kind]
                .get("caption")
                .map(|v| v.as_str().context("invalid caption"))
                .transpose()?
                .unwrap_or("");
            let label = match kind {
                "image" => "image",
                "video" => "video",
                "audio" => "audio",
                "document" => "document",
                _ => "sticker",
            };
            (label, caption)
        }
        "button" => (
            "text",
            message["button"]["text"].as_str().unwrap_or("Button reply"),
        ),
        "interactive" => (
            "text",
            message["interactive"]["button_reply"]["title"]
                .as_str()
                .or_else(|| message["interactive"]["list_reply"]["title"].as_str())
                .unwrap_or("Interactive reply"),
        ),
        "reaction" => (
            "reaction",
            message["reaction"]["emoji"]
                .as_str()
                .unwrap_or("Reaction removed"),
        ),
        "location" => (
            "location",
            message["location"]["name"].as_str().unwrap_or("Location"),
        ),
        "contacts" => ("contact", "Shared contact"),
        "system" => ("system", "Account update"),
        _ => ("unknown", "Unsupported message"),
    };
    Ok((kind, clipped(text, MAX_TEXT_BYTES)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> CloudConfig {
        CloudConfig {
            enabled: true,
            business_account_id: "123".into(),
            phone_number_id: "456".into(),
            app_secret: "test-app-secret".into(),
            verify_token: "test-verify-token".into(),
            ..CloudConfig::default()
        }
    }

    fn sign(raw: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(config().app_secret.as_bytes()).unwrap();
        mac.update(raw);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn message(id: &str) -> Value {
        json!({"from":"15551234567", "id":id, "timestamp":"1700000000", "type":"text", "text":{"body":"A test message"}})
    }

    fn change(messages: Vec<Value>) -> Value {
        json!({"field":"messages", "value":{"messaging_product":"whatsapp", "metadata":{"phone_number_id":"456"}, "contacts":[{"wa_id":"15551234567", "profile":{"name":"Test sender"}}], "messages":messages}})
    }

    fn payload(changes: Vec<Value>) -> Value {
        json!({"object":"whatsapp_business_account", "entry":[{"id":"123", "changes":changes}]})
    }

    fn parse(value: &Value) -> Result<DeliveryBatch> {
        parse_delivery(&config(), &serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn signature_authenticates_original_bytes_including_whitespace() {
        let raw = b"{ \"object\": \"whatsapp_business_account\",\n \"entry\": [] }\n";
        let signature = sign(raw);
        assert!(verify_signature(&config().app_secret, raw, &signature));
        let normalized =
            serde_json::to_vec(&serde_json::from_slice::<Value>(raw).unwrap()).unwrap();
        assert!(!verify_signature(
            &config().app_secret,
            &normalized,
            &signature
        ));
        assert!(!verify_signature("wrong-secret", raw, &signature));
        for invalid in ["", "sha1=00", "sha256=00", "sha256=not-hex"] {
            assert!(!verify_signature(&config().app_secret, raw, invalid));
        }
        assert_eq!(
            header(
                &HashMap::from([("X-Hub-Signature-256".into(), signature.clone())]),
                "x-hub-signature-256"
            ),
            Some(signature.as_str())
        );
    }

    #[test]
    fn verification_returns_an_unquoted_plaintext_challenge_and_rejects_bad_tokens() {
        let mut query = HashMap::from([
            ("hub.mode".into(), "subscribe".into()),
            ("hub.verify_token".into(), config().verify_token),
            ("hub.challenge".into(), "00123456".into()),
        ]);
        let response = verify(&config(), &query);
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, "00123456");
        assert_eq!(
            response.headers["content-type"],
            "text/plain; charset=utf-8"
        );
        query.insert("hub.verify_token".into(), "wrong".into());
        assert_eq!(verify(&config(), &query).status_code, 403);
        assert_eq!(verify(&CloudConfig::default(), &query).status_code, 503);
        query.insert("hub.verify_token".into(), config().verify_token);
        query.remove("hub.challenge");
        assert_eq!(verify(&config(), &query).status_code, 400);
    }

    #[test]
    fn handles_every_matching_entry_and_change_without_cross_account_data() {
        let mut data = payload(vec![
            change(vec![message("one"), message("two")]),
            change(vec![message("three")]),
        ]);
        let mut other_number = change(vec![message("wrong-number")]);
        other_number["value"]["metadata"]["phone_number_id"] = "999".into();
        data["entry"].as_array_mut().unwrap().extend([
            json!({"id":"999", "changes":[change(vec![message("wrong-account")])]}),
            json!({"id":"123", "changes":[other_number, change(vec![message("four")])]}),
        ]);
        let batch = parse(&data).unwrap();
        assert_eq!(
            batch
                .messages
                .iter()
                .map(|m| m.message_id.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three", "four"]
        );
        assert_eq!(batch.messages[0].chat_id, "cloud:+15551234567");
        assert_eq!(
            batch.messages[0].sender_name.as_deref(),
            Some("Test sender")
        );
        assert_eq!(batch.messages[0].timestamp_ms, 1_700_000_000_000);
        assert!(!batch.messages[0].from_me);
        assert!(
            parse(&json!({"object":"whatsapp_business_account", "entry":[{"id":"999"}]}))
                .unwrap()
                .messages
                .is_empty()
        );
    }

    #[test]
    fn accepts_bsuid_messages_and_statuses_without_phone_numbers() {
        let mut inbound = message("private-user");
        inbound.as_object_mut().unwrap().remove("from");
        inbound["from_user_id"] = "BR.ENT.abc123".into();
        let mut event = change(vec![inbound]);
        event["value"]["contacts"] =
            json!([{"user_id":"BR.ENT.abc123", "profile":{"name":"Private test sender"}}]);
        event["value"]["statuses"] =
            json!([{"id":"outgoing", "status":"read", "recipient_user_id":"BR.ENT.abc123"}]);
        let batch = parse(&payload(vec![event])).unwrap();
        assert_eq!(batch.messages[0].chat_id, "cloud:BR.ENT.abc123");
        assert_eq!(batch.messages[0].sender_id, "BR.ENT.abc123");
        assert_eq!(
            batch.messages[0].sender_name.as_deref(),
            Some("Private test sender")
        );
        assert_eq!(batch.receipts[0].chat_id, "cloud:BR.ENT.abc123");
        assert_eq!(batch.receipts[0].status, "read");
    }

    #[test]
    fn accepts_supported_receipts_and_bounds_complete_deliveries() {
        let mut event = change(Vec::new());
        event["value"]["statuses"] = json!([
            {"id":"one", "status":"sent", "recipient_id":"15551234567"},
            {"id":"one", "status":"delivered", "recipient_id":"15551234567"},
            {"id":"one", "status":"read", "recipient_id":"15551234567"},
            {"id":"two", "status":"failed", "recipient_id":null, "recipient_user_id":"BR.abc123"},
            {"id":"future", "status":"future-status"}
        ]);
        let batch = parse(&payload(vec![event])).unwrap();
        assert_eq!(
            batch
                .receipts
                .iter()
                .map(|receipt| receipt.status.as_str())
                .collect::<Vec<_>>(),
            ["sent", "delivered", "read", "failed"]
        );
        assert!(parse(&payload(vec![change(vec![message("one"); 1001])])).is_err());
    }

    #[tokio::test]
    async fn rejects_unsigned_or_projected_bodies_without_opening_a_channel() {
        let directory = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::Config {
            data_dir: directory.path().to_owned(),
            cloud: config(),
            ..crate::config::Config::default()
        };
        config.cloud.access_token = "test-access-token".into();
        let manager = SessionManager::open(config).unwrap();
        assert_eq!(
            receive(
                manager.clone(),
                "ws://unused.invalid",
                ReceiveRequest::default()
            )
            .await
            .status_code,
            403
        );
        let request = serde_json::from_value(json!({"headers":{"X-Hub-Signature-256":sign(b"{}")}, "body":payload(vec![change(vec![message("one")])])})).unwrap();
        assert_eq!(
            receive(manager.clone(), "ws://unused.invalid", request)
                .await
                .status_code,
            400
        );
        let mut config = manager.config();
        config.cloud.enabled = false;
        manager.apply_config(config).unwrap();
        assert_eq!(
            receive(manager, "ws://unused.invalid", ReceiveRequest::default())
                .await
                .status_code,
            503
        );
    }

    #[test]
    fn only_archives_whitelisted_previews_and_rejects_malformed_matching_batches() {
        let mut media = message("image");
        media["type"] = "image".into();
        media["image"] = json!({"id":"private-media-id", "url":"https://example.invalid/private", "caption":"Picture caption"});
        let mut unknown = message("unknown");
        unknown["type"] = "new-future-format".into();
        unknown["new-future-format"] = json!({"secret":"must not be copied"});
        let batch = parse(&payload(vec![change(vec![media, unknown])])).unwrap();
        assert_eq!(batch.messages[0].kind, "image");
        assert_eq!(batch.messages[0].text, "Picture caption");
        assert_eq!(batch.messages[1].text, "Unsupported message");
        let encoded = serde_json::to_string(&batch.messages).unwrap();
        assert!(!encoded.contains("private-media-id"));
        assert!(!encoded.contains("must not be copied"));
        let mut invalid = message("invalid");
        invalid["timestamp"] = "9223372036854775807".into();
        assert!(parse(&payload(vec![change(vec![message("valid"), invalid])])).is_err());
        assert!(parse(&json!({"object":"other", "entry":[]})).is_err());
        assert!(parse_delivery(&config(), &vec![b' '; MAX_BODY_BYTES + 1]).is_err());
        assert_eq!(clipped("éé", 3), "é");
    }

    #[test]
    fn duplicate_deliveries_are_idempotent_in_the_archive() {
        let directory = tempfile::TempDir::new().unwrap();
        let inbox = crate::inbox::Inbox::open(directory.path(), 100).unwrap();
        let batch = parse(&payload(vec![change(vec![
            message("same"),
            message("same"),
        ])]))
        .unwrap();
        inbox
            .record_external(&batch.messages, &batch.receipts)
            .unwrap();
        inbox
            .record_external(&batch.messages, &batch.receipts)
            .unwrap();
        let page = inbox.list_messages("cloud:+15551234567", None, 50).unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].message_id, "same");
    }
}
