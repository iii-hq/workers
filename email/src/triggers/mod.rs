pub mod dispatcher;
pub mod new_mail;
pub mod registry;

use chrono::{DateTime, Utc};
use serde::Serialize;

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

impl Event {
    /// The payload an `email::new-mail` binding and an `email::on-new-mail`
    /// observer both receive for this event.
    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({
            "account": self.account,
            "folder": self.folder,
            "uid": self.uid,
            "message_id": self.message_id,
            "from": self.from,
            "subject": self.subject,
            "snippet": self.snippet,
            "ts": self.ts.map(|t| t.to_rfc3339()),
        })
    }
}
