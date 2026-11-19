//! Which agent session made a change, and the ticket ↔ session links that follow from it.
//!
//! The harness runs every turn step inside W3C baggage carrying `iii.session.id` and `iii.message.id` (the turn
//! id) — `harness/src/functions/turn.rs` — and the engine forwards a caller's baggage to the function it calls.
//! So a kanban function called by an agent can read, from its own invocation, exactly which session called it:
//! no naming convention, no harness hook. A call with no such baggage (a person in the console, cron, a
//! workflow) has no session, and records none.
//!
//! [`board::record`](crate::board::record) stamps the caller on each activity entry it writes. A ticket's links
//! are then derived from its activity: one row per session that changed it, with when and how often.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::store::{Board, Ticket};

/// The baggage key the harness stamps with the session of the running turn.
pub const SESSION_KEY: &str = "iii.session.id";
/// The baggage key the harness stamps with the turn id.
pub const TURN_KEY: &str = "iii.message.id";

/// The agent session (and turn) a call came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub session_id: String,
    pub turn_id: Option<String>,
}

/// The session of the invocation this task is handling, if an agent's turn made the call.
pub fn caller() -> Option<Caller> {
    let baggage = iii_sdk::current_invocation()?.baggage?;
    parse_baggage(&baggage)
}

/// `iii.session.id` / `iii.message.id` from a W3C baggage header
/// (`key=value;prop=x,key2=value2`, values percent-encoded). An empty or missing session id is no caller.
pub fn parse_baggage(header: &str) -> Option<Caller> {
    let mut session = None;
    let mut turn = None;
    for member in header.split(',') {
        let entry = member.split(';').next().unwrap_or_default();
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        let value = percent_decode(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            SESSION_KEY => session = Some(value),
            TURN_KEY => turn = Some(value),
            _ => {}
        }
    }
    session.map(|session_id| Caller {
        session_id,
        turn_id: turn,
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// One agent session that changed one ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionLink {
    pub ticket_id: String,
    pub ticket_key: String,
    pub session_id: String,
    /// The first and the latest change this session made to the ticket (RFC 3339).
    pub first_at: String,
    pub last_at: String,
    /// How many activity entries (field changes, moves, comments…) the session wrote on the ticket.
    pub writes: u64,
    /// The kind of its latest entry: `ticket.created`, `ticket.updated`, `ticket.moved`, `comment.created`, …
    pub last_action: String,
    /// The actor it wrote as (an agent profile id).
    pub actor: String,
    /// The turn of its latest entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// The sessions that changed `ticket`, in the order they first touched it.
pub fn ticket_links(ticket: &Ticket) -> Vec<SessionLink> {
    let mut links: Vec<SessionLink> = Vec::new();
    for entry in &ticket.activity {
        let Some(session_id) = entry.session_id.as_deref() else {
            continue;
        };
        if let Some(link) = links.iter_mut().find(|link| link.session_id == session_id) {
            link.writes += 1;
            link.last_at = entry.at.clone();
            link.last_action = entry.kind.clone();
            link.actor = entry.actor.clone();
            link.turn_id = entry.turn_id.clone();
        } else {
            links.push(SessionLink {
                ticket_id: ticket.id.clone(),
                ticket_key: ticket.key.clone(),
                session_id: session_id.to_string(),
                first_at: entry.at.clone(),
                last_at: entry.at.clone(),
                writes: 1,
                last_action: entry.kind.clone(),
                actor: entry.actor.clone(),
                turn_id: entry.turn_id.clone(),
            });
        }
    }
    links
}

/// Which tickets to read links for. Everything empty: every live ticket on the board.
#[derive(Debug, Default, Clone)]
pub struct LinksFilter {
    /// One project's tickets (`Some("")`: the tickets in no project).
    pub project_id: Option<String>,
    /// One ticket, by uuid (already resolved).
    pub ticket_id: Option<String>,
    /// One session's tickets.
    pub session_id: Option<String>,
}

pub fn links(board: &Board, filter: &LinksFilter) -> Vec<SessionLink> {
    board
        .tickets
        .values()
        .filter(|ticket| ticket.deleted_at.is_none())
        .filter(|ticket| filter.ticket_id.as_deref().is_none_or(|id| ticket.id == id))
        .filter(|ticket| match filter.project_id.as_deref() {
            None => true,
            Some("") => ticket.project_id.is_none(),
            Some(project) => ticket.project_id.as_deref() == Some(project),
        })
        .flat_map(ticket_links)
        .filter(|link| {
            filter
                .session_id
                .as_deref()
                .is_none_or(|id| link.session_id == id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_session_and_turn_from_w3c_baggage() {
        let caller = parse_baggage(
            "iii.session.id=kan12-backend,iii.message.id=t_1;prop=x,iii.tag.kind=harness.turn",
        );
        assert_eq!(
            caller,
            Some(Caller {
                session_id: "kan12-backend".into(),
                turn_id: Some("t_1".into())
            })
        );
    }

    #[test]
    fn decodes_percent_encoded_values_and_ignores_other_members() {
        let caller =
            parse_baggage(" iii.tag.message=Fix%20it , iii.session.id = console-7f3a%2Ddeepwork ");
        assert_eq!(
            caller.map(|c| c.session_id),
            Some("console-7f3a-deepwork".into())
        );
    }

    #[test]
    fn no_session_member_is_no_caller() {
        assert_eq!(
            parse_baggage("iii.message.id=t_1,iii.tag.hidden=state"),
            None
        );
        assert_eq!(parse_baggage("iii.session.id="), None);
        assert_eq!(parse_baggage(""), None);
    }
}
