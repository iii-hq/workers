use super::*;
use whatsapp_rust::{ChatStateType, PinDuration, PresenceStatus, SyncActionMessageRange};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiptRequest {
    pub session_id: String,
    pub chat_id: String,
    pub sender_id: Option<String>,
    pub message_ids: Vec<String>,
}

impl ReceiptRequest {
    fn targets(&self) -> Result<(Jid, Option<Jid>)> {
        let chat = validate::jid(&self.chat_id)?;
        let sender = self.sender_id.as_deref().map(validate::jid).transpose()?;
        ensure!(
            !self.message_ids.is_empty() && self.message_ids.len() <= 256,
            "provide 1 to 256 message_ids"
        );
        for id in &self.message_ids {
            message_id(id)?;
        }
        if chat.is_group() || chat.is_status_broadcast() {
            ensure!(
                sender.is_some(),
                "sender_id is required for group or status receipts"
            );
        }
        if let Some(sender) = &sender {
            ensure!(
                sender.is_pn() || sender.is_lid(),
                "sender_id must identify a person"
            );
        }
        Ok((chat, sender))
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Available,
    Unavailable,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresenceRequest {
    pub session_id: String,
    pub presence: Presence,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatState {
    Composing,
    Recording,
    Paused,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatStateRequest {
    pub session_id: String,
    pub chat_id: String,
    pub state: ChatState,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeMessage {
    pub key: MessageKeyInput,
    pub timestamp_ms: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageRange {
    pub last_message_timestamp_ms: i64,
    pub last_system_message_timestamp_ms: Option<i64>,
    #[serde(default)]
    pub messages: Vec<RangeMessage>,
}

impl MessageRange {
    pub(super) fn native(self, chat: &Jid) -> Result<SyncActionMessageRange> {
        ensure!(
            self.last_message_timestamp_ms > 0,
            "last_message_timestamp_ms must be positive"
        );
        ensure!(
            self.messages.len() <= 256,
            "message range allows at most 256 messages"
        );
        let mut messages = Vec::with_capacity(self.messages.len());
        for message in self.messages {
            ensure!(
                validate::jid(&message.key.chat_id)? == *chat,
                "range message belongs to a different chat"
            );
            messages.push((
                message.key.native()?,
                message_seconds(message.timestamp_ms)?,
            ));
        }
        Ok(whatsapp_rust::message_range(
            message_seconds(self.last_message_timestamp_ms)?,
            self.last_system_message_timestamp_ms
                .map(message_seconds)
                .transpose()?,
            messages,
        ))
    }
}

// Message ranges use Unix seconds; the enclosing app-state action and mute
// deadline use milliseconds. Keep the public timestamp convention explicit.
fn message_seconds(timestamp_ms: i64) -> Result<i64> {
    ensure!(
        timestamp_ms >= 1000,
        "message timestamp must be at least 1000 milliseconds since the Unix epoch"
    );
    Ok(timestamp_ms / 1000)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeRequest {
    pub session_id: String,
    pub chat_id: String,
    pub message_range: Option<MessageRange>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadChatRequest {
    pub session_id: String,
    pub chat_id: String,
    pub read: bool,
    pub message_range: Option<MessageRange>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteChatRequest {
    pub session_id: String,
    pub chat_id: String,
    pub delete_media: bool,
    pub message_range: Option<MessageRange>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearChatRequest {
    pub session_id: String,
    pub chat_id: String,
    pub delete_media: bool,
    pub delete_starred: bool,
    pub message_range: Option<MessageRange>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MuteRequest {
    pub session_id: String,
    pub chat_id: String,
    pub until_timestamp_ms: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageActionRequest {
    pub session_id: String,
    pub target: MessageKeyInput,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteLocalRequest {
    pub session_id: String,
    pub target: MessageKeyInput,
    pub delete_media: bool,
    pub message_timestamp_ms: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PinTime {
    Hours24,
    Days7,
    Days30,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinMessageRequest {
    pub session_id: String,
    pub target: MessageKeyInput,
    pub duration: PinTime,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeepRequest {
    pub session_id: String,
    pub target: MessageKeyInput,
    pub keep: bool,
}

pub(super) fn register(registry: &mut Registry) {
    macro_rules! chat_action {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<ChatRequest, Applied, _, _>($id, $description, |m, r| async move {
                let chat = validate::jid(&r.chat_id)?;
                m.client(&r.session_id)
                    .await?
                    .chat_actions()
                    .$method(&chat)
                    .await?;
                Ok(applied())
            });
        };
    }
    chat_action!(
        "whatsapp::chats::pin",
        "Pin a chat across linked devices.",
        pin_chat
    );
    chat_action!(
        "whatsapp::chats::unpin",
        "Unpin a chat across linked devices.",
        unpin_chat
    );
    chat_action!(
        "whatsapp::chats::unmute",
        "Unmute a chat across linked devices.",
        unmute_chat
    );
    chat_action!(
        "whatsapp::chats::lock",
        "Move a chat into the locked chats folder.",
        lock_chat
    );
    chat_action!(
        "whatsapp::chats::unlock",
        "Remove a chat from the locked chats folder.",
        unlock_chat
    );
    registry.add::<MuteRequest, Applied, _, _>(
        "whatsapp::chats::mute",
        "Mute a chat until a future timestamp or indefinitely when omitted.",
        |m, r| async move {
            let chat = validate::jid(&r.chat_id)?;
            let client = m.client(&r.session_id).await?;
            match r.until_timestamp_ms {
                Some(until) => client.chat_actions().mute_chat_until(&chat, until).await?,
                None => client.chat_actions().mute_chat(&chat).await?,
            }
            Ok(applied())
        },
    );
    macro_rules! archive {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<RangeRequest, Applied, _, _>($id, $description, |m, r| async move {
                let chat = validate::jid(&r.chat_id)?;
                let range = r
                    .message_range
                    .map(|range| range.native(&chat))
                    .transpose()?;
                m.client(&r.session_id)
                    .await?
                    .chat_actions()
                    .$method(&chat, range)
                    .await?;
                Ok(applied())
            });
        };
    }
    archive!(
        "whatsapp::chats::archive",
        "Archive a chat with an optional synchronized message range.",
        archive_chat
    );
    archive!(
        "whatsapp::chats::unarchive",
        "Unarchive a chat with an optional synchronized message range.",
        unarchive_chat
    );
    registry.add::<ReadChatRequest, Applied, _, _>(
        "whatsapp::chats::mark_read",
        "Set the chat's read/unread state across linked devices.",
        |m, r| async move {
            let chat = validate::jid(&r.chat_id)?;
            let range = r
                .message_range
                .map(|range| range.native(&chat))
                .transpose()?;
            m.client(&r.session_id)
                .await?
                .chat_actions()
                .mark_chat_as_read(&chat, r.read, range)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<DeleteChatRequest, Applied, _, _>(
        "whatsapp::chats::delete",
        "Delete a chat on this account's linked devices.",
        |m, r| async move {
            let chat = validate::jid(&r.chat_id)?;
            let range = r
                .message_range
                .map(|range| range.native(&chat))
                .transpose()?;
            m.client(&r.session_id)
                .await?
                .chat_actions()
                .delete_chat(&chat, r.delete_media, range)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<ClearChatRequest, Applied, _, _>(
        "whatsapp::chats::clear",
        "Clear messages while keeping the chat, with explicit starred/media flags.",
        |m, r| async move {
            let chat = validate::jid(&r.chat_id)?;
            let range = r
                .message_range
                .map(|range| range.native(&chat))
                .transpose()?;
            m.client(&r.session_id)
                .await?
                .chat_actions()
                .clear_chat(&chat, r.delete_starred, r.delete_media, range)
                .await?;
            Ok(applied())
        },
    );
    macro_rules! receipt {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<ReceiptRequest, Applied, _, _>($id, $description, |m, r| async move {
                let (chat, sender) = r.targets()?;
                let ids = r.message_ids.iter().map(String::as_str).collect::<Vec<_>>();
                m.client(&r.session_id)
                    .await?
                    .$method(&chat, sender.as_ref(), &ids)
                    .await?;
                Ok(applied())
            });
        };
    }
    receipt!(
        "whatsapp::messages::mark_read",
        "Send read receipts for specific messages, respecting native privacy settings.",
        mark_as_read
    );
    receipt!(
        "whatsapp::messages::mark_played",
        "Send played receipts for specific voice or video messages.",
        mark_as_played
    );
    registry.add::<PresenceRequest, Applied, _, _>(
        "whatsapp::chats::presence",
        "Set this account's available or unavailable presence.",
        |m, r| async move {
            let presence = match r.presence {
                Presence::Available => PresenceStatus::Available,
                Presence::Unavailable => PresenceStatus::Unavailable,
            };
            m.client(&r.session_id)
                .await?
                .presence()
                .set(presence)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<ChatRequest, Applied, _, _>(
        "whatsapp::chats::subscribe_presence",
        "Subscribe to presence updates for a person.",
        |m, r| async move {
            let jid = validate::jid(&r.chat_id)?;
            ensure!(
                jid.is_pn() || jid.is_lid(),
                "presence subscriptions require a person"
            );
            m.client(&r.session_id)
                .await?
                .presence()
                .subscribe(jid)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<ChatRequest, Applied, _, _>(
        "whatsapp::chats::unsubscribe_presence",
        "Unsubscribe from a person's presence updates.",
        |m, r| async move {
            let jid = validate::jid(&r.chat_id)?;
            ensure!(
                jid.is_pn() || jid.is_lid(),
                "presence subscriptions require a person"
            );
            m.client(&r.session_id)
                .await?
                .presence()
                .unsubscribe(&jid)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<ChatStateRequest, Applied, _, _>(
        "whatsapp::chats::state",
        "Send a typing, recording or paused indicator.",
        |m, r| async move {
            let to = validate::jid(&r.chat_id)?;
            let state = match r.state {
                ChatState::Composing => ChatStateType::Composing,
                ChatState::Recording => ChatStateType::Recording,
                ChatState::Paused => ChatStateType::Paused,
            };
            m.client(&r.session_id)
                .await?
                .chatstate()
                .send(&to, state)
                .await?;
            Ok(applied())
        },
    );
    macro_rules! star {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<MessageActionRequest, Applied, _, _>(
                $id,
                $description,
                |m, r| async move {
                    let key = r.target.native()?;
                    let chat = validate::jid(&r.target.chat_id)?;
                    let participant = key.participant.as_deref().map(validate::jid).transpose()?;
                    m.client(&r.session_id)
                        .await?
                        .chat_actions()
                        .$method(
                            &chat,
                            participant.as_ref(),
                            &r.target.message_id,
                            r.target.from_me,
                        )
                        .await?;
                    Ok(applied())
                },
            );
        };
    }
    star!(
        "whatsapp::messages::star",
        "Star a message on this account's linked devices.",
        star_message
    );
    star!(
        "whatsapp::messages::unstar",
        "Remove a message's star on linked devices.",
        unstar_message
    );
    registry.add::<DeleteLocalRequest, Applied, _, _>(
        "whatsapp::messages::delete_for_me",
        "Delete a message only on this account's linked devices.",
        |m, r| async move {
            let key = r.target.native()?;
            let chat = validate::jid(&r.target.chat_id)?;
            let participant = key.participant.as_deref().map(validate::jid).transpose()?;
            let timestamp = r.message_timestamp_ms.map(message_seconds).transpose()?;
            m.client(&r.session_id)
                .await?
                .chat_actions()
                .delete_message_for_me(
                    &chat,
                    participant.as_ref(),
                    &r.target.message_id,
                    r.target.from_me,
                    r.delete_media,
                    timestamp,
                )
                .await?;
            Ok(applied())
        },
    );
    registry.add::<PinMessageRequest, SentMessage, _, _>(
        "whatsapp::messages::pin",
        "Pin a message for all participants for 24 hours, 7 days or 30 days.",
        |m, r| async move {
            let key = r.target.native()?;
            let chat = validate::jid(&r.target.chat_id)?;
            let duration = match r.duration {
                PinTime::Hours24 => PinDuration::Hours24,
                PinTime::Days7 => PinDuration::Days7,
                PinTime::Days30 => PinDuration::Days30,
            };
            Ok(m.client(&r.session_id)
                .await?
                .pin_message(chat, key, duration)
                .await?
                .into())
        },
    );
    registry.add::<MessageActionRequest, SentMessage, _, _>(
        "whatsapp::messages::unpin",
        "Unpin a message for all participants.",
        |m, r| async move {
            let key = r.target.native()?;
            let chat = validate::jid(&r.target.chat_id)?;
            Ok(m.client(&r.session_id)
                .await?
                .unpin_message(chat, key)
                .await?
                .into())
        },
    );
    registry.add::<KeepRequest, SentMessage, _, _>(
        "whatsapp::messages::keep",
        "Keep or unkeep a disappearing message for all participants.",
        |m, r| async move {
            let key = r.target.native()?;
            let chat = validate::jid(&r.target.chat_id)?;
            Ok(m.client(&r.session_id)
                .await?
                .keep_message(chat, key, r.keep)
                .await?
                .into())
        },
    );
}
