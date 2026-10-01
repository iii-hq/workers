//! Official Meta Cloud API access, isolated from native linked-device accounts.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    config::{self, Config},
    inbox::{ChatPage, Inbox, MessagePage, MessageView},
    registry::Registry,
    sessions::private_directory,
};

pub mod client;
pub mod webhook;

use client::{
    CloudClient, ConnectionCheck, SentMessage, TemplatePage, TemplateRequest, TemplatesRequest,
    TextRequest,
};

pub struct CloudService {
    client: CloudClient,
    archives: Mutex<Archives>,
}

#[derive(Default)]
struct Archives {
    opened: HashMap<PathBuf, Arc<Inbox>>,
    live_limit: Option<usize>,
}

impl CloudService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: CloudClient::new()?,
            archives: Mutex::new(Archives::default()),
        })
    }

    /// Opens display storage only; never registers a number or makes a network request.
    pub fn inbox(&self, config: &Config) -> Result<Arc<Inbox>> {
        ensure!(config.cloud.enabled, "Cloud API is disabled");
        let number = &config.cloud.phone_number_id;
        ensure!(
            !number.is_empty() && number.len() <= 32 && number.bytes().all(|b| b.is_ascii_digit()),
            "Cloud API phone_number_id must be numeric"
        );
        let root = config.data_dir.join(".cloud");
        let directory = root.join(number);
        let mut archives = self.archives.lock().unwrap();
        if let Some(inbox) = archives.opened.get(&directory) {
            return Ok(inbox.clone());
        }
        private_directory(&config.data_dir)?;
        private_directory(&root)?;
        private_directory(&directory)?;
        let inbox = Inbox::open(
            &directory,
            archives.live_limit.unwrap_or(config.max_inbox_messages),
        )?;
        archives.opened.insert(directory, inbox.clone());
        Ok(inbox)
    }

    pub fn apply_limit(&self, limit: usize) -> Result<()> {
        let mut archives = self.archives.lock().unwrap();
        for inbox in archives.opened.values() {
            inbox.set_limit(limit)?;
        }
        archives.live_limit = Some(limit);
        Ok(())
    }
}

/// Public Cloud IDs are international phone numbers or Meta business-scoped IDs.
/// Native JIDs and the internal archive prefix are intentionally not accepted.
pub fn normalize_chat_id(value: &str) -> Result<String> {
    let value = value.trim();
    if let Some((country, suffix)) = value.split_once('.') {
        let suffix = suffix.strip_prefix("ENT.").unwrap_or(suffix);
        ensure!(
            country.len() == 2
                && country.bytes().all(|b| b.is_ascii_uppercase())
                && !suffix.is_empty()
                && suffix.len() <= 128
                && suffix.bytes().all(|b| b.is_ascii_alphanumeric()),
            "invalid Cloud API business-scoped user ID"
        );
        return Ok(value.to_owned());
    }
    let digits: String = value
        .chars()
        .filter(|c| !matches!(c, ' ' | '(' | ')' | '-'))
        .collect();
    let digits = digits.strip_prefix('+').unwrap_or(&digits);
    ensure!(
        (7..=15).contains(&digits.len())
            && !digits.starts_with('0')
            && digits.bytes().all(|b| b.is_ascii_digit()),
        "Cloud API recipient must be an international phone number or business-scoped user ID; native JIDs are not supported"
    );
    Ok(format!("+{digits}"))
}

pub fn storage_chat_id(value: &str) -> Result<String> {
    Ok(format!("cloud:{}", normalize_chat_id(value)?))
}

pub fn webhook_path() -> String {
    format!(
        "/webhooks/whatsapp-cloud/{}",
        path_segment(config::config_id())
    )
}

fn path_segment(value: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to string");
        }
    }
    encoded
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CloudStatus {
    pub enabled: bool,
    /// Local readiness only. Does not imply remote authentication or delivery.
    pub configured: bool,
    pub missing_fields: Vec<String>,
    pub api_version: String,
    pub business_account_id: String,
    pub phone_number_id: String,
    pub access_token_set: bool,
    pub app_secret_set: bool,
    pub verify_token_set: bool,
    pub webhook_path: String,
    pub webhook_ready: bool,
}

pub fn status(config: &Config) -> CloudStatus {
    let cloud = &config.cloud;
    let missing_fields: Vec<_> = [
        ("business_account_id", &cloud.business_account_id),
        ("phone_number_id", &cloud.phone_number_id),
        ("access_token", &cloud.access_token),
        ("app_secret", &cloud.app_secret),
        ("verify_token", &cloud.verify_token),
    ]
    .into_iter()
    .filter(|(_, value)| value.trim().is_empty())
    .map(|(field, _)| field.to_owned())
    .collect();
    CloudStatus {
        enabled: cloud.enabled,
        configured: cloud.enabled && missing_fields.is_empty(),
        missing_fields,
        api_version: cloud.api_version.clone(),
        business_account_id: cloud.business_account_id.clone(),
        phone_number_id: cloud.phone_number_id.clone(),
        access_token_set: !cloud.access_token.trim().is_empty(),
        app_secret_set: !cloud.app_secret.trim().is_empty(),
        verify_token_set: !cloud.verify_token.trim().is_empty(),
        webhook_path: webhook_path(),
        webhook_ready: cloud.enabled
            && !cloud.app_secret.trim().is_empty()
            && !cloud.verify_token.trim().is_empty(),
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChatsRequest {
    query: Option<String>,
    cursor: Option<String>,
    #[serde(default = "chat_limit")]
    #[schemars(range(min = 1, max = 200))]
    limit: usize,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MessagesRequest {
    chat_id: String,
    cursor: Option<String>,
    #[serde(default = "message_limit")]
    #[schemars(range(min = 1, max = 200))]
    limit: usize,
}

fn chat_limit() -> usize {
    100
}
fn message_limit() -> usize {
    50
}

fn outward(message: &mut MessageView) {
    if let Some(chat) = message.chat_id.strip_prefix("cloud:") {
        message.chat_id = chat.to_owned();
    }
    if let Some(sender) = message.sender_id.strip_prefix("cloud:") {
        message.sender_id = sender.to_owned();
    }
}

async fn retain_send(
    inbox: Arc<Inbox>,
    config: &Config,
    sent: &SentMessage,
    kind: &str,
    text: String,
) -> Result<()> {
    let message = MessageView {
        message_id: sent.message_id.clone(),
        chat_id: storage_chat_id(&sent.chat_id)?,
        sender_id: config.cloud.phone_number_id.clone(),
        sender_name: None,
        from_me: true,
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX),
        kind: kind.to_owned(),
        text,
        source: "sent".into(),
        status: Some("submitted".into()),
    };
    tokio::task::spawn_blocking(move || inbox.record_external(&[message], &[])).await
        .map_err(anyhow::Error::from).and_then(|result| result)
        .with_context(|| format!("Cloud message {} was accepted but could not be saved locally; do not resend automatically", sent.message_id))?;
    Ok(())
}

pub fn register(registry: &mut Registry) {
    registry.add::<Empty, CloudStatus, _, _>(
        "whatsapp::cloud::status",
        "Read local Cloud API readiness without contacting Meta or returning credentials",
        |manager, _| async move { Ok(status(&manager.config())) },
    );
    registry.add::<Empty, ConnectionCheck, _, _>("whatsapp::cloud::check_connection", "Check configured phone, business account membership and webhook subscriptions using read-only Meta requests", |manager, _| async move {
        manager.cloud().client.check_connection(&manager.config()).await
    });
    registry.add::<TemplatesRequest, TemplatePage, _, _>(
        "whatsapp::cloud::templates::list",
        "List existing Meta templates without creating or changing them",
        |manager, req| async move {
            manager
                .cloud()
                .client
                .list_templates(&manager.config(), &req)
                .await
        },
    );
    registry.add::<TextRequest, SentMessage, _, _>("whatsapp::cloud::messages::send_text", "Submit one Cloud API text message; requires an open customer service window and never retries automatically", |manager, req| async move {
        // Validate before opening storage or attempting a remote mutation.
        normalize_chat_id(&req.chat_id)?;
        let config = manager.config();
        let inbox = manager.cloud().inbox(&config)?;
        let sent = manager.cloud().client.send_text(&config, &req).await?;
        retain_send(inbox, &config, &sent, "text", req.text).await?;
        Ok(sent)
    });
    registry.add::<TemplateRequest, SentMessage, _, _>("whatsapp::cloud::messages::send_template", "Submit one existing approved Cloud API template; delivery is reported by webhook and sends are never retried automatically", |manager, req| async move {
        normalize_chat_id(&req.chat_id)?;
        let config = manager.config();
        let inbox = manager.cloud().inbox(&config)?;
        let sent = manager.cloud().client.send_template(&config, &req).await?;
        let text = format!("[Template: {} ({})]", req.name, req.language);
        retain_send(inbox, &config, &sent, "template", text).await?;
        Ok(sent)
    });
    registry.add::<ChatsRequest, ChatPage, _, _>("whatsapp::cloud::chats::list", "List retained Cloud API conversations without reading native sessions or sending read receipts", |manager, req| async move {
        let inbox = manager.cloud().inbox(&manager.config())?;
        let mut page = tokio::task::spawn_blocking(move || inbox.list_chats(req.query.as_deref(), req.cursor.as_deref(), req.limit)).await??;
        for chat in &mut page.chats {
            if chat.title == chat.chat_id { chat.title = chat.title.strip_prefix("cloud:").unwrap_or(&chat.title).to_owned(); }
            chat.chat_id = chat.chat_id.strip_prefix("cloud:").context("invalid Cloud archive chat")?.to_owned();
            if let Some(message) = &mut chat.last_message { outward(message); }
        }
        Ok(page)
    });
    registry.add::<MessagesRequest, MessagePage, _, _>("whatsapp::cloud::messages::list", "Read retained Cloud API messages; no historical Meta webhook fetch or automatic read receipt", |manager, req| async move {
        let chat = storage_chat_id(&req.chat_id)?;
        let inbox = manager.cloud().inbox(&manager.config())?;
        let mut page = tokio::task::spawn_blocking(move || inbox.list_messages(&chat, req.cursor.as_deref(), req.limit)).await??;
        page.messages.iter_mut().for_each(outward);
        Ok(page)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_identifiers_never_accept_native_sessions_or_paths() {
        assert_eq!(
            normalize_chat_id(" +1 (202) 555-0123 ").unwrap(),
            "+12025550123"
        );
        assert_eq!(
            normalize_chat_id("5511999999999").unwrap(),
            "+5511999999999"
        );
        for value in ["US.123Ab", "BR.ENT.ab12"] {
            assert_eq!(normalize_chat_id(value).unwrap(), value);
        }
        for value in [
            "123@s.whatsapp.net",
            "123@g.us",
            "cloud:+12025550123",
            "../123",
            "0012025550123",
            "us.123",
            "US.",
            "US.ENT.",
            "US.a/b",
            "+1234567890123456",
        ] {
            assert!(normalize_chat_id(value).is_err(), "{value}");
        }
        assert_eq!(
            storage_chat_id("12025550123").unwrap(),
            "cloud:+12025550123"
        );
    }

    #[test]
    fn status_exposes_presence_without_secret_values() {
        let mut config = Config::default();
        config.cloud.access_token = "test-secret-token".into();
        config.cloud.app_secret = "test-app-secret".into();
        config.cloud.verify_token = "test-verify-token".into();
        let value = serde_json::to_value(status(&config)).unwrap();
        assert_eq!(value["access_token_set"], true);
        assert_eq!(value["configured"], false);
        assert!(!value.to_string().contains("test-"));
        assert_eq!(path_segment("tenant/a b?ç"), "tenant%2Fa%20b%3F%C3%A7");
    }

    #[tokio::test]
    async fn failed_archive_write_keeps_the_known_send_acceptance_explicit() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = Inbox::open(directory.path(), 10).unwrap();
        inbox.close().unwrap();
        let sent = SentMessage {
            message_id: "wamid.accepted".into(),
            chat_id: "+12025550123".into(),
            status: "accepted".into(),
            message_status: None,
        };
        let error = retain_send(
            inbox,
            &Config::default(),
            &sent,
            "text",
            "Already accepted".into(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("wamid.accepted was accepted"));
        assert!(error.contains("do not resend automatically"));
    }

    #[test]
    fn archives_are_lazy_separate_and_apply_live_retention() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = Config {
            data_dir: directory.path().into(),
            ..Config::default()
        };
        config.cloud.enabled = true;
        config.cloud.phone_number_id = "111".into();
        let service = CloudService::new().unwrap();
        assert!(!directory.path().join(".cloud").exists());
        let first = service.inbox(&config).unwrap();
        assert!(Arc::ptr_eq(&first, &service.inbox(&config).unwrap()));
        config.cloud.phone_number_id = "222".into();
        let second = service.inbox(&config).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        let message = |id: &str, time| MessageView {
            message_id: id.into(),
            chat_id: "cloud:+12025550123".into(),
            sender_id: "+12025550123".into(),
            sender_name: None,
            from_me: false,
            timestamp_ms: time,
            kind: "text".into(),
            text: id.into(),
            source: "live".into(),
            status: None,
        };
        first
            .record_external(&[message("older", 1), message("newer", 2)], &[])
            .unwrap();
        assert!(second.list_chats(None, None, 100).unwrap().chats.is_empty());
        service.apply_limit(1).unwrap();
        let page = first.list_messages("cloud:+12025550123", None, 50).unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].message_id, "newer");
        assert!(page.coverage.retention_truncated);
        // An in-flight request may still hold the old configuration snapshot.
        config.cloud.phone_number_id = "333".into();
        let third = service.inbox(&config).unwrap();
        assert_eq!(
            third
                .list_chats(None, None, 100)
                .unwrap()
                .coverage
                .message_limit,
            1
        );
    }
}
