//! `email::on-new-mail`: the observable function an IMAP `IDLE` push calls
//! once per new message, and the dispatcher that calls it.

pub mod dispatcher;
pub mod new_mail;

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
