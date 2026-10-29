pub mod dispatcher;
pub mod new_mail;
pub mod registry;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Normalized event handed from `provider::imap::idle` to the dispatcher.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub account: String,
    pub folder: String,
    pub uid: u32,
    pub message_id: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub ts: Option<DateTime<Utc>>,
}

/// The payload an `email::new-mail` binding and an `email::on-new-mail`
/// observer both receive, and the request schema `email::on-new-mail`
/// publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewMailEvent {
    /// The configured account the message arrived on.
    pub account: String,
    /// The IMAP folder watched with IDLE (`INBOX` by default).
    pub folder: String,
    /// The message's IMAP UID in `folder`.
    pub uid: u32,
    /// The `Message-ID` header, when present.
    pub message_id: Option<String>,
    /// The `From` header, when present.
    pub from: Option<String>,
    /// The `Subject` header, when present.
    pub subject: Option<String>,
    /// The first ~200 characters of the body, when available.
    pub snippet: Option<String>,
    /// The message date, RFC 3339, when known.
    pub ts: Option<String>,
}

impl Event {
    /// The typed event an `email::new-mail` binding and an
    /// `email::on-new-mail` observer both receive for this event.
    pub fn new_mail_event(&self) -> NewMailEvent {
        NewMailEvent {
            account: self.account.clone(),
            folder: self.folder.clone(),
            uid: self.uid,
            message_id: self.message_id.clone(),
            from: self.from.clone(),
            subject: self.subject.clone(),
            snippet: self.snippet.clone(),
            ts: self.ts.map(|t| t.to_rfc3339()),
        }
    }

    /// The payload an `email::new-mail` binding and an `email::on-new-mail`
    /// observer both receive for this event: [`NewMailEvent`] as JSON.
    pub fn payload(&self) -> serde_json::Value {
        serde_json::to_value(self.new_mail_event()).unwrap_or(serde_json::Value::Null)
    }
}
