//! `email::on-new-mail`: a private function whose handler returns the new-mail
//! event it is called with. The IDLE supervisor of each configured
//! `(account, folder)` calls it once per new message; observers select
//! messages with filters on `input.account` and `input.folder`.

use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Event;

/// Id of the function observers observe for new mail.
pub const ON_NEW_MAIL_FN: &str = "email::on-new-mail";

pub const ON_NEW_MAIL_DESCRIPTION: &str =
    "Called once per message IMAP IDLE pushes to a configured (account, folder); returns \
     the event: { account, folder, uid, message_id, from, subject, snippet, ts }. Observe it \
     with filters on input.account and input.folder. Only the email worker calls it. If the \
     IMAP server lacks the IDLE capability, the account fails at startup (E610).";

/// One new message in a watched `(account, folder)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NewMailEvent {
    /// Configured account name.
    pub account: String,
    /// IMAP folder the message landed in.
    pub folder: String,
    /// IMAP UID of the message in `folder`.
    pub uid: u32,
    pub message_id: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    /// First ~200 characters of the body.
    pub snippet: Option<String>,
    /// RFC 3339 `Date` header, when the message carries one.
    pub ts: Option<String>,
}

impl From<Event> for NewMailEvent {
    fn from(event: Event) -> Self {
        Self {
            account: event.account,
            folder: event.folder,
            uid: event.uid,
            message_id: event.message_id,
            from: event.from,
            subject: event.subject,
            snippet: event.snippet,
            ts: event.ts.map(|t| t.to_rfc3339()),
        }
    }
}

/// Registers `email::on-new-mail`: private (only this worker calls it), and
/// observable by any worker. iii delivers each call to every observer whose
/// filters match it.
pub fn register(iii: &IIIClient) {
    iii.register_function(
        ON_NEW_MAIL_FN,
        RegisterFunction::new(|event: NewMailEvent| Ok::<_, Error>(event))
            .description(ON_NEW_MAIL_DESCRIPTION)
            .private(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn event_keeps_every_field_and_formats_ts_as_rfc3339() {
        let event = Event {
            account: "support".into(),
            folder: "INBOX".into(),
            uid: 12345,
            message_id: Some("<abc@mx.example.com>".into()),
            from: Some("alice@example.com".into()),
            subject: Some("Ticket #42".into()),
            snippet: Some("hello".into()),
            ts: Some(Utc.with_ymd_and_hms(2026, 5, 28, 10, 14, 0).unwrap()),
        };
        let wire = serde_json::to_value(NewMailEvent::from(event)).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "account": "support",
                "folder": "INBOX",
                "uid": 12345,
                "message_id": "<abc@mx.example.com>",
                "from": "alice@example.com",
                "subject": "Ticket #42",
                "snippet": "hello",
                "ts": "2026-05-28T10:14:00+00:00",
            })
        );
    }
}
