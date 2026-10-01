//! Typed native adapters. A send result is a submitted stanza, not a delivery receipt.
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::{Jid, RevokeType, SendResult, prelude::wa};

use crate::{registry::Registry, validate};

mod chats;
mod labels;
mod media;
mod newsletters;
mod status;
#[cfg(test)]
mod tests;

const MAX_TEXT: usize = 65_536;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionRequest {
    pub session_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    pub session_id: String,
    pub chat_id: String,
}

/// Acknowledges only completion of the native API call.
#[derive(Serialize, JsonSchema)]
pub struct Applied {
    pub applied: bool,
}

fn applied() -> Applied {
    Applied { applied: true }
}

#[derive(Serialize, JsonSchema)]
pub struct Fanout {
    pub addressed: usize,
    pub encrypted: usize,
    pub skipped_primary: bool,
    pub had_unregistered_device: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct SentMessage {
    pub message_id: String,
    pub chat_id: String,
    pub recipient_fanout: Option<Fanout>,
}

impl From<SendResult> for SentMessage {
    fn from(value: SendResult) -> Self {
        Self {
            message_id: value.message_id,
            chat_id: value.to.to_string(),
            recipient_fanout: value.recipient_fanout.map(|f| Fanout {
                addressed: f.addressed,
                encrypted: f.encrypted,
                skipped_primary: f.skipped_primary,
                had_unregistered_device: f.had_unregistered_device,
            }),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextRequest {
    pub session_id: String,
    pub chat_id: String,
    pub text: String,
}

fn text_message(text: &str) -> Result<wa::Message> {
    validate::text(text, "text", MAX_TEXT)?;
    Ok(wa::Message {
        conversation: Some(text.to_owned()),
        ..Default::default()
    })
}

fn message_id(id: &str) -> Result<()> {
    validate::text(id, "message_id", 200)?;
    ensure!(
        !id.chars().any(char::is_whitespace),
        "message_id must not contain whitespace"
    );
    Ok(())
}

fn optional_text(value: Option<&str>, field: &str, max: usize) -> Result<()> {
    if let Some(value) = value {
        ensure!(value.len() <= max, "{field} exceeds {max} bytes");
        ensure!(!value.contains('\0'), "{field} contains a null character");
    }
    Ok(())
}

/// Keeps an original sender's namespace, including LID, for message operations.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageKeyInput {
    pub chat_id: String,
    pub message_id: String,
    pub from_me: bool,
    pub participant_id: Option<String>,
}

impl MessageKeyInput {
    pub fn native(&self) -> Result<wa::MessageKey> {
        message_id(&self.message_id)?;
        let chat = validate::jid(&self.chat_id)?;
        let participant = self
            .participant_id
            .as_deref()
            .map(validate::jid)
            .transpose()?;
        if chat.is_group() && !self.from_me || chat.is_status_broadcast() {
            ensure!(
                participant.is_some(),
                "participant_id is required for this message key"
            );
        }
        if let Some(participant) = &participant {
            ensure!(
                participant.is_pn() || participant.is_lid(),
                "participant_id must identify a person"
            );
        }
        Ok(wa::MessageKey {
            remote_jid: Some(chat.to_string()),
            id: Some(self.message_id.clone()),
            from_me: Some(self.from_me),
            participant: participant.map(|j| j.to_string()),
        })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyRequest {
    pub session_id: String,
    pub chat_id: String,
    pub text: String,
    pub message_id: String,
    pub sender_id: String,
    pub quoted_text: String,
}

impl ReplyRequest {
    pub fn message(&self) -> Result<wa::Message> {
        message_id(&self.message_id)?;
        validate::text(&self.text, "text", MAX_TEXT)?;
        let quoted = text_message(&self.quoted_text)?;
        let sender = validate::jid(&self.sender_id)?;
        ensure!(
            sender.is_pn() || sender.is_lid(),
            "sender_id must identify a person"
        );
        Ok(wa::Message {
            extended_text_message: wa::message::ExtendedTextMessage {
                text: Some(self.text.clone()),
                context_info: wa::ContextInfo {
                    stanza_id: Some(self.message_id.clone()),
                    participant: Some(sender.to_string()),
                    remote_jid: Some(validate::jid(&self.chat_id)?.to_string()),
                    quoted_message: quoted.into(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub session_id: String,
    pub chat_id: String,
    pub message_id: String,
    pub text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteRequest {
    pub session_id: String,
    pub chat_id: String,
    pub message_id: String,
    /// Set only when revoking another member's message as a group admin.
    pub original_sender_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReactionRequest {
    pub session_id: String,
    pub target: MessageKeyInput,
    /// Empty removes this account's reaction.
    pub emoji: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationRequest {
    pub session_id: String,
    pub chat_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub name: Option<String>,
    pub address: Option<String>,
}

impl LocationRequest {
    pub fn message(&self) -> Result<wa::Message> {
        ensure!(
            self.latitude.is_finite() && (-90.0..=90.0).contains(&self.latitude),
            "latitude must be between -90 and 90"
        );
        ensure!(
            self.longitude.is_finite() && (-180.0..=180.0).contains(&self.longitude),
            "longitude must be between -180 and 180"
        );
        optional_text(self.name.as_deref(), "name", 1024)?;
        optional_text(self.address.as_deref(), "address", 4096)?;
        Ok(wa::Message {
            location_message: wa::message::LocationMessage {
                degrees_latitude: Some(self.latitude),
                degrees_longitude: Some(self.longitude),
                name: self.name.clone(),
                address: self.address.clone(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContactRequest {
    pub session_id: String,
    pub chat_id: String,
    pub display_name: String,
    /// A complete vCard, supplied by the caller.
    pub vcard: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PollRequest {
    pub session_id: String,
    pub chat_id: String,
    pub name: String,
    pub options: Vec<String>,
    pub selectable_count: u32,
}

impl PollRequest {
    pub fn validate(&self) -> Result<()> {
        validate::text(&self.name, "name", 255)?;
        ensure!(
            (2..=12).contains(&self.options.len()),
            "poll requires 2 to 12 options"
        );
        ensure!(
            self.selectable_count >= 1 && self.selectable_count as usize <= self.options.len(),
            "selectable_count must be between 1 and the number of options"
        );
        let mut distinct = std::collections::HashSet::new();
        for option in &self.options {
            validate::text(option, "option", 100)?;
            ensure!(distinct.insert(option), "poll options must be unique");
        }
        Ok(())
    }
}

#[derive(Serialize, JsonSchema)]
pub struct PollSent {
    pub message: SentMessage,
    pub message_secret_base64: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PollVoteRequest {
    pub session_id: String,
    pub chat_id: String,
    pub message_id: String,
    pub creator_id: String,
    pub message_secret_base64: String,
    pub option_names: Vec<String>,
}

fn fixed_bytes(value: &str, field: &str) -> Result<[u8; 32]> {
    let bytes = validate::media(value, 32)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{field} must contain exactly 32 decoded bytes"))
}

pub fn register(registry: &mut Registry) {
    registry.add::<TextRequest, SentMessage, _, _>(
        "whatsapp::messages::send_text",
        "Send a text message; returns submission metadata, not delivery confirmation.",
        |m, r| async move {
            let message = text_message(&r.text)?;
            let to = validate::jid(&r.chat_id)?;
            Ok(m.client(&r.session_id)
                .await?
                .send_message(to, message)
                .await?
                .into())
        },
    );
    registry.add::<ReplyRequest, SentMessage, _, _>(
        "whatsapp::messages::reply",
        "Reply quoting a text message with its original sender and stanza id.",
        |m, r| async move {
            let message = r.message()?;
            let to = validate::jid(&r.chat_id)?;
            Ok(m.client(&r.session_id)
                .await?
                .send_message(to, message)
                .await?
                .into())
        },
    );
    registry.add::<EditRequest, SentMessage, _, _>(
        "whatsapp::messages::edit",
        "Edit this account's text message in a direct or group chat.",
        |m, r| async move {
            message_id(&r.message_id)?;
            let message = text_message(&r.text)?;
            let to = validate::jid(&r.chat_id)?;
            ensure!(!to.is_newsletter(), "use newsletters::edit for channels");
            Ok(m.client(&r.session_id)
                .await?
                .edit_message(to, r.message_id, message)
                .await?
                .into())
        },
    );
    registry.add::<DeleteRequest, SentMessage, _, _>(
        "whatsapp::messages::delete",
        "Revoke a message for everyone; group admins may specify its original sender.",
        |m, r| async move {
            message_id(&r.message_id)?;
            let to = validate::jid(&r.chat_id)?;
            ensure!(!to.is_newsletter(), "use newsletters::revoke for channels");
            let revoke = match r.original_sender_id {
                Some(sender) => {
                    ensure!(to.is_group(), "admin revoke requires a group chat");
                    let original_sender = validate::jid(&sender)?;
                    ensure!(
                        original_sender.is_pn() || original_sender.is_lid(),
                        "original_sender_id must identify a person"
                    );
                    RevokeType::Admin { original_sender }
                }
                None => RevokeType::Sender,
            };
            Ok(m.client(&r.session_id)
                .await?
                .revoke_message(to, r.message_id, revoke)
                .await?
                .into())
        },
    );
    registry.add::<ReactionRequest, SentMessage, _, _>(
        "whatsapp::messages::react",
        "Set or remove a reaction using the original message key.",
        |m, r| async move {
            optional_text(Some(&r.emoji), "emoji", 64)?;
            let key = r.target.native()?;
            let to = validate::jid(&r.target.chat_id)?;
            ensure!(!to.is_newsletter(), "use newsletters::react for channels");
            ensure!(
                !to.is_group() || key.participant.is_some(),
                "participant_id is required for group reactions, including own messages"
            );
            Ok(m.client(&r.session_id)
                .await?
                .send_reaction(to, key, &r.emoji)
                .await?
                .into())
        },
    );
    registry.add::<LocationRequest, SentMessage, _, _>(
        "whatsapp::messages::send_location",
        "Send a static geographic location.",
        |m, r| async move {
            let message = r.message()?;
            let to = validate::jid(&r.chat_id)?;
            Ok(m.client(&r.session_id)
                .await?
                .send_message(to, message)
                .await?
                .into())
        },
    );
    registry.add::<ContactRequest, SentMessage, _, _>(
        "whatsapp::messages::send_contact",
        "Send a contact as a supplied vCard.",
        |m, r| async move {
            validate::text(&r.display_name, "display_name", 1024)?;
            validate::text(&r.vcard, "vcard", 65_536)?;
            ensure!(
                r.vcard.starts_with("BEGIN:VCARD") && r.vcard.trim_end().ends_with("END:VCARD"),
                "vcard must contain a complete BEGIN:VCARD / END:VCARD envelope"
            );
            let to = validate::jid(&r.chat_id)?;
            let message = wa::Message {
                contact_message: wa::message::ContactMessage {
                    display_name: Some(r.display_name),
                    vcard: Some(r.vcard),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            };
            Ok(m.client(&r.session_id)
                .await?
                .send_message(to, message)
                .await?
                .into())
        },
    );
    registry.add::<PollRequest, PollSent, _, _>(
        "whatsapp::messages::send_poll",
        "Create a poll and return the secret needed to process votes.",
        |m, r| async move {
            r.validate()?;
            let to = validate::jid(&r.chat_id)?;
            let (result, secret) = m
                .client(&r.session_id)
                .await?
                .polls()
                .create(to, &r.name, &r.options, r.selectable_count)
                .await?;
            Ok(PollSent {
                message: result.into(),
                message_secret_base64: BASE64.encode(secret),
            })
        },
    );
    registry.add::<PollVoteRequest, SentMessage, _, _>("whatsapp::messages::vote_poll", "Submit or clear a poll selection using the poll's original secret.", |m, r| async move {
        message_id(&r.message_id)?;
        let to = validate::jid(&r.chat_id)?;
        ensure!(to.is_pn() || to.is_lid() || to.is_group(), "encrypted poll votes require a direct or group chat; use newsletters::vote_poll for channels");
        let creator = validate::jid(&r.creator_id)?;
        ensure!(creator.is_pn() || creator.is_lid(), "creator_id must identify a person");
        let secret = fixed_bytes(&r.message_secret_base64, "message_secret_base64")?;
        ensure!(r.option_names.len() <= 12, "at most 12 option_names are allowed");
        for name in &r.option_names { validate::text(name, "option_name", 100)?; }
        Ok(m.client(&r.session_id).await?.polls().vote(to, &r.message_id, &creator, &secret, &r.option_names).await?.into())
    });
    media::register(registry);
    chats::register(registry);
    status::register(registry);
    newsletters::register(registry);
    labels::register(registry);
}
