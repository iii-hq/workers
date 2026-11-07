//! Delivery provenance for recall. Only the internal delivery path writes
//! receipts; transcript origin and caller metadata are never authority.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::clients::session::LoadedEntry;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::{Delivery, DeliveryChannel};
use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;
use crate::types::turn::TurnRecord;

pub(crate) const SCOPE: &str = "harness_recall_delivery";
const MAX_TEXT_BYTES: usize = 3200;
const REFERENCE_WINDOW: usize = 64;
const MAX_RECEIPTS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Query {
    entry_id: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Receipt {
    session_id: String,
    entry_id: String,
    channel: DeliveryChannel,
    query: Option<QueryStamp>,
    continuation: Option<QueryStamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct QueryStamp {
    entry_id: String,
    digest: String,
}

fn stamp(q: Query) -> QueryStamp {
    QueryStamp {
        entry_id: q.entry_id,
        digest: format!("{:x}", Sha256::digest(q.text.as_bytes())),
    }
}

fn key(session: &str) -> String {
    session.into()
}

fn bounded_receipts(mut rows: Vec<Receipt>, incoming: Receipt) -> Vec<Receipt> {
    rows.push(incoming);
    let excess = rows.len().saturating_sub(MAX_RECEIPTS);
    drop(rows.drain(..excess));
    rows
}

fn query(entry_id: &str, message: &AgentMessage) -> Option<Query> {
    let AgentMessage::User(user) = message else {
        return None;
    };
    let text = ContentBlock::join_text(&user.content);
    if text.trim().is_empty() {
        return None;
    }
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let text = text[..end].to_owned();
    Some(Query {
        entry_id: entry_id.into(),
        text,
    })
}

async fn receipt(deps: &Deps, session: &str, entry: &str) -> Result<Option<Receipt>, HarnessError> {
    let cfg = deps.cfg().await;
    let value =
        crate::state::state_get(&deps.iii, SCOPE, &key(session), cfg.session_timeout_ms).await?;
    if value.is_null() {
        return Ok(None);
    }
    let rows: Vec<Receipt> = serde_json::from_value(value)
        .map_err(|e| HarnessError::State(format!("invalid recall receipt: {e}")))?;
    Ok(rows
        .into_iter()
        .find(|r| r.session_id == session && r.entry_id == entry))
}

/// Called before append/queue by every ordinary delivery path. A notification
/// cannot opt into continuation, nor can an invoked agent promote its channel.
pub(crate) async fn record_delivery(
    deps: &Deps,
    session: &str,
    delivery: &Delivery<'_>,
) -> Result<(), HarnessError> {
    let entry = delivery
        .entry_id
        .ok_or_else(|| HarnessError::InvalidRequest("delivery has no entry id".into()))?;
    if session.len() > 256 || entry.is_empty() || entry.len() > 256 {
        return Err(HarnessError::InvalidRequest(
            "invalid recall delivery identifiers".into(),
        ));
    }
    let mut continuation = None;
    if let Some(prior_id) = delivery.continuation_entry_id {
        if delivery.channel == DeliveryChannel::Notification
            || prior_id == entry
            || prior_id.len() > 256
        {
            return Err(HarnessError::InvalidRequest(
                "invalid recall continuation reference".into(),
            ));
        }
        // The production session resolver follows every page and rejects
        // malformed evidence; never read an unhydrated raw state reference.
        let entries = deps.session().await.messages_strict(session).await?;
        let prior = entries
            .iter()
            .rev()
            .take(REFERENCE_WINDOW)
            .find(|e| e.entry_id == prior_id)
            .and_then(|e| e.message.as_ref())
            .and_then(|m| query(prior_id, m))
            .ok_or_else(|| {
                HarnessError::InvalidRequest(
                    "continuation is not a recent user entry on this active path".into(),
                )
            })?;
        let authority = receipt(deps, session, prior_id)
            .await?
            .filter(|r| {
                r.channel == delivery.channel && r.query.as_ref() == Some(&stamp(prior.clone()))
            })
            .ok_or_else(|| {
                HarnessError::InvalidRequest("continuation has no matching delivery receipt".into())
            })?;
        // Exactly one hop; do not recursively inherit another prior topic.
        continuation = authority.query;
    }
    let r = Receipt {
        session_id: session.into(),
        entry_id: entry.into(),
        channel: delivery.channel,
        query: query(entry, delivery.message).map(stamp),
        continuation,
    };
    let cfg = deps.cfg().await;
    for _ in 0..8 {
        let old = crate::state::state_get(&deps.iii, SCOPE, &key(session), cfg.session_timeout_ms)
            .await?;
        let rows: Vec<Receipt> = if old.is_null() {
            Vec::new()
        } else {
            serde_json::from_value(old.clone())
                .map_err(|e| HarnessError::State(format!("invalid recall receipt ledger: {e}")))?
        };
        if let Some(existing) = rows.iter().find(|x| x.entry_id == entry) {
            return if existing == &r {
                Ok(())
            } else {
                Err(HarnessError::InvalidRequest(
                    "delivery entry id already has different provenance".into(),
                ))
            };
        }
        let value =
            serde_json::to_value(bounded_receipts(rows, r.clone())).expect("receipt ledger");
        if crate::state::cas_value(
            &deps.iii,
            SCOPE,
            &key(session),
            (!old.is_null()).then_some(old),
            value,
            cfg.session_timeout_ms,
        )
        .await?
        .is_none()
        {
            return Ok(());
        }
    }
    Err(HarnessError::State(
        "concurrent recall delivery ledger update".into(),
    ))
}

/// An operator edit can refresh only an existing direct receipt matching the
/// queued row's old text. Notification/agent/legacy rows never gain authority.
/// A fork or a forged queued row cannot borrow another session's receipt.
pub(crate) async fn edit_direct(
    deps: &Deps,
    session: &str,
    entry: &str,
    old_message: &AgentMessage,
    new_message: &AgentMessage,
) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    for _ in 0..8 {
        let old = crate::state::state_get(&deps.iii, SCOPE, &key(session), timeout).await?;
        if old.is_null() {
            return Ok(());
        }
        let mut rows: Vec<Receipt> = serde_json::from_value(old.clone())
            .map_err(|e| HarnessError::State(format!("invalid recall edit ledger: {e}")))?;
        let Some(r) = rows
            .iter_mut()
            .find(|r| r.session_id == session && r.entry_id == entry)
        else {
            return Ok(());
        };
        if r.channel != DeliveryChannel::Direct {
            return Ok(());
        }
        if r.query == query(entry, new_message).map(stamp) {
            return Ok(());
        }
        if r.query != query(entry, old_message).map(stamp) {
            return Err(HarnessError::InvalidRequest(
                "queued edit does not match its direct delivery receipt".into(),
            ));
        }
        r.query = query(entry, new_message).map(stamp);
        // Replacing the instruction is a new request, never implicit continuation.
        r.continuation = None;
        if crate::state::cas_value(
            &deps.iii,
            SCOPE,
            &key(session),
            Some(old),
            serde_json::to_value(rows).expect("edit ledger"),
            timeout,
        )
        .await?
        .is_none()
        {
            return Ok(());
        }
    }
    Err(HarnessError::State("concurrent queued receipt edit".into()))
}

/// Scoped lifecycle cleanup: no global scan, no transcript contents retained.
pub(crate) async fn purge(deps: &Deps, session: &str) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let old = crate::state::state_get(&deps.iii, SCOPE, &key(session), timeout).await?;
    if !old.is_null()
        && crate::state::cas_value(
            &deps.iii,
            SCOPE,
            &key(session),
            Some(old),
            Value::Null,
            timeout,
        )
        .await?
        .is_some()
    {
        return Err(HarnessError::State(
            "recall ledger changed during session deletion".into(),
        ));
    }
    Ok(())
}

fn envelope(
    record: &TurnRecord,
    entry: &LoadedEntry,
    authority: Option<&Receipt>,
    entries: &[LoadedEntry],
) -> Value {
    let current = entry
        .message
        .as_ref()
        .and_then(|m| query(&entry.entry_id, m));
    let valid = authority.filter(|r| {
        r.session_id == record.session_id
            && r.entry_id == entry.entry_id
            && r.query == current.clone().map(stamp)
    });
    let continuation = valid
        .and_then(|r| r.continuation.as_ref())
        .and_then(|prior| {
            entries
                .iter()
                .rev()
                .take(REFERENCE_WINDOW)
                .filter(|e| e.entry_id != entry.entry_id)
                .filter_map(|e| e.message.as_ref().and_then(|m| query(&e.entry_id, m)))
                .find(|q| stamp(q.clone()) == *prior)
        });
    // A reference that left the active path must not silently revert to the
    // current chatter or inherit an unrelated history topic.
    let invalid_reference =
        valid.is_some_and(|r| r.continuation.is_some()) && continuation.is_none();
    let channel = valid.filter(|_| !invalid_reference).map(|r| r.channel);
    let kind = match channel {
        Some(DeliveryChannel::Direct) => "direct",
        Some(DeliveryChannel::InvokedAgent) => "invoked-agent",
        Some(DeliveryChannel::Notification) => "notification",
        None => "unknown",
    };
    json!({
        "version":1, "session_id":record.session_id, "turn_id":record.turn_id,
        "source":{"kind":kind, "entry_id":entry.entry_id},
        "query":current, "continuation":continuation
    })
}

/// Latest persisted user entry, not model notices appended by assembly/hooks.
/// Receipt failure is recall-only fail-closed; generation and rules continue.
pub(crate) async fn resolve(
    deps: &Deps,
    record: &TurnRecord,
    entries: &[LoadedEntry],
) -> Option<Value> {
    let entry = entries.iter().rev().find(|e| {
        matches!(
            e.message.as_ref(),
            Some(AgentMessage::User(_) | AgentMessage::Custom(_))
        )
    })?;
    let authority = receipt(deps, &record.session_id, &entry.entry_id)
        .await
        .ok()
        .flatten();
    Some(envelope(record, entry, authority.as_ref(), entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str, text: &str) -> LoadedEntry {
        serde_json::from_value(json!({"entry_id":id,"message":{"role":"user","content":[{"type":"text","text":text}],"timestamp":0},"origin":{"kind":"direct"}})).unwrap()
    }
    #[test]
    fn recall_provenance_query_bounds_unicode_without_splitting_codepoints() {
        let message = AgentMessage::user_text("é".repeat(2000));
        let q = query("e-long", &message).unwrap();
        assert_eq!(q.text.len(), 3200);
        assert_eq!(q.text.chars().count(), 1600);
        assert_eq!(q.entry_id, "e-long");
    }
    #[test]
    fn recall_receipt_retention_is_bounded_and_has_no_plaintext() {
        let mut rows = Vec::new();
        for n in 0..130 {
            rows = bounded_receipts(
                rows,
                Receipt {
                    session_id: "s".into(),
                    entry_id: format!("e{n}"),
                    channel: DeliveryChannel::Direct,
                    query: Some(stamp(Query {
                        entry_id: format!("e{n}"),
                        text: "Sensitive topic".into(),
                    })),
                    continuation: None,
                },
            );
        }
        assert_eq!(rows.len(), 128);
        assert_eq!(rows.first().unwrap().entry_id, "e2");
        assert_eq!(rows.last().unwrap().entry_id, "e129");
        assert!(!serde_json::to_string(&rows)
            .unwrap()
            .contains("Sensitive topic"));
    }
    #[test]
    fn recall_provenance_requires_receipt_not_writer_origin() {
        let record = crate::types::turn::tests::record();
        let e = entry("e1", "Ledgerly PostgreSQL");
        assert_eq!(
            envelope(&record, &e, None, &[])["source"]["kind"],
            "unknown"
        );
        for (channel, expected) in [
            (DeliveryChannel::Direct, "direct"),
            (DeliveryChannel::InvokedAgent, "invoked-agent"),
            (DeliveryChannel::Notification, "notification"),
        ] {
            let r = Receipt {
                session_id: record.session_id.clone(),
                entry_id: e.entry_id.clone(),
                channel,
                query: e.message.as_ref().and_then(|m| query("e1", m)).map(stamp),
                continuation: None,
            };
            assert_eq!(
                envelope(&record, &e, Some(&r), &[])["source"]["kind"],
                expected
            );
            let wrong = Receipt {
                session_id: "another".into(),
                ..r
            };
            assert_eq!(
                envelope(&record, &e, Some(&wrong), &[])["source"]["kind"],
                "unknown"
            );
        }
    }
    #[test]
    fn recall_provenance_continuation_revalidates_active_path_and_text() {
        let record = crate::types::turn::tests::record();
        let old = entry("e0", "Ledgerly PostgreSQL");
        let current = entry("e1", "Continue.");
        let r = Receipt {
            session_id: record.session_id.clone(),
            entry_id: "e1".into(),
            channel: DeliveryChannel::Direct,
            query: current
                .message
                .as_ref()
                .and_then(|m| query("e1", m))
                .map(stamp),
            continuation: old.message.as_ref().and_then(|m| query("e0", m)).map(stamp),
        };
        assert_eq!(
            envelope(&record, &current, Some(&r), &[old.clone(), current.clone()])["continuation"]
                ["text"],
            "Ledgerly PostgreSQL"
        );
        assert_eq!(
            envelope(&record, &current, Some(&r), std::slice::from_ref(&current))["source"]["kind"],
            "unknown"
        );
        assert_eq!(
            envelope(
                &record,
                &current,
                Some(&r),
                &[entry("e0", "Different topic"), current.clone()]
            )["source"]["kind"],
            "unknown"
        );
    }
}

#[cfg(test)]
#[path = "recall_tests.rs"]
mod transport_tests;
