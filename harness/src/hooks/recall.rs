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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    full_text_sha256: Option<String>,
    query: Option<QueryStamp>,
    continuation: Option<QueryStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extraction: Vec<ExtractionAttestation>,
}

/// No text is retained. An absent/evicted attestation is UNKNOWN, not ON.
/// Rows travel only inside Harness-owned private hook calls; JSON alone is
/// not an authentication mechanism for a consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExtractionAttestation {
    version: u8,
    session_id: String,
    turn_id: String,
    entry_id: String,
    text_sha256: String,
    policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bank: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    /// Memory's `memory_derived` stamp: the bank was resolved, not named.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    derived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal: Option<ExtractionTerminal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExtractionTerminal {
    status: String,
    exchange_finished: bool,
}

// Bounded independently of delivery receipts. Missing historical rows never
// inherit the next turn's policy/bank, including after retention eviction.
const MAX_EXTRACTION_TURNS: usize = 8;

fn full_stamp(entry: &LoadedEntry) -> Option<String> {
    full_message_stamp(entry.message.as_ref()?)
}

fn full_message_stamp(message: &AgentMessage) -> Option<String> {
    let AgentMessage::User(user) = message else {
        return None;
    };
    let text = ContentBlock::join_text(&user.content);
    if text.trim().is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(text.as_bytes())))
}

fn acknowledged_policy(
    annotations: &Value,
) -> (String, Option<String>, Option<String>, Option<String>, bool) {
    let bounded = |key: &str| {
        annotations
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .map(str::to_owned)
    };
    let bank = bounded("memory_bank");
    let policy = match annotations
        .get("memory_extraction")
        .and_then(Value::as_bool)
    {
        Some(false) => "off",
        Some(true) if bank.is_some() => "on",
        _ => "unknown",
    };
    (
        policy.into(),
        bank,
        bounded("memory_layer"),
        bounded("memory_agent"),
        annotations.get("memory_derived").and_then(Value::as_bool) == Some(true),
    )
}

/// Only ThroughObservers calls this after the actual Memory target answered,
/// with the transcript the step loaded before the call. Merged chain
/// annotations and transcript/client metadata are not inputs. State failures
/// discard proof, never prevent generation or become ON.
pub(super) async fn acknowledge_extraction(
    iii: &iii_sdk::IIIClient,
    input: &Value,
    response: &Value,
    entries: &[LoadedEntry],
    timeout: u64,
) -> Result<(), HarnessError> {
    let Some(session) = input["session_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
    else {
        return Ok(());
    };
    let Some(turn) = input["turn_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
    else {
        return Ok(());
    };
    let context = &input["recall_context"];
    if input["point"] != "pre_generate" || response["decision"].as_str() != Some("continue") {
        return Ok(());
    }
    if context["version"] != 1 || context["session_id"] != session || context["turn_id"] != turn {
        return Ok(());
    }
    let Some(entry_id) = context["source"]["entry_id"].as_str() else {
        return Ok(());
    };
    update_ledger(iii, session, timeout, |old| {
        let mut rows = ledger(old)?;
        // A turn's first acknowledged source/policy is immutable. A later
        // source cannot borrow that policy, even if the first row was edited.
        // Checked before any transcript work: every later step is a no-op.
        if rows
            .iter()
            .filter(|r| r.session_id == session)
            .flat_map(|r| &r.extraction)
            .any(|p| p.turn_id == turn)
        {
            return Ok(None);
        }
        let Some(entry) = entries.iter().find(|e| e.entry_id == entry_id) else {
            return Ok(None);
        };
        let Some(digest) = full_stamp(entry) else {
            return Ok(None);
        };
        let current_query = entry.message.as_ref().and_then(|m| query(entry_id, m));
        if context["query"] != serde_json::to_value(&current_query).expect("source query") {
            return Ok(None);
        }
        let current_query = current_query.map(stamp);
        let Some(row) = rows.iter_mut().find(|r| {
            r.session_id == session
                && r.entry_id == entry_id
                && context["source"]["kind"]
                    == serde_json::to_value(r.channel).expect("source channel")
                && r.full_text_sha256.as_ref() == Some(&digest)
                && r.query == current_query
        }) else {
            return Ok(None);
        };
        let (policy, bank, layer, agent, derived) = acknowledged_policy(&response["annotations"]);
        row.extraction.push(ExtractionAttestation {
            version: 1,
            session_id: session.into(),
            turn_id: turn.into(),
            entry_id: entry_id.into(),
            text_sha256: digest,
            policy,
            bank,
            layer,
            agent,
            derived,
            terminal: None,
        });
        let excess = row.extraction.len().saturating_sub(MAX_EXTRACTION_TURNS);
        drop(row.extraction.drain(..excess));
        Ok(Some(serde_json::to_value(rows).expect("extraction ledger")))
    })
    .await
}

/// What [`acknowledge_extraction`] reads of a hook input: never the prompt,
/// the messages or the tools a pre_generate input carries.
pub(super) fn acknowledgement_input(input: &Value) -> Value {
    let context = &input["recall_context"];
    json!({
        "point": input["point"], "session_id": input["session_id"], "turn_id": input["turn_id"],
        "recall_context": {"version": context["version"], "session_id": context["session_id"],
            "turn_id": context["turn_id"], "source": context["source"], "query": context["query"]},
    })
}

/// Terminal boundaries are recorded from the finalizer, never an event payload.
/// A turn that never received a Memory acknowledgement has no eligible proof.
/// A boundary is written once, except that an unfinished exchange may later be
/// marked finished (with the turn's status then); a finished one never moves.
pub(crate) async fn finish_extraction(deps: &Deps, record: &TurnRecord, exchange_finished: bool) {
    let timeout = deps.cfg().await.session_timeout_ms;
    let result = finish_extraction_inner(&deps.iii, record, exchange_finished, timeout).await;
    if let Err(error) = result {
        tracing::warn!(%error, "extraction boundary unavailable; proof remains ineligible");
    }
}

async fn finish_extraction_inner(
    iii: &iii_sdk::IIIClient,
    record: &TurnRecord,
    exchange_finished: bool,
    timeout: u64,
) -> Result<(), HarnessError> {
    let status = match record.status {
        crate::types::turn::TurnStatus::Completed => "completed",
        crate::types::turn::TurnStatus::Failed => "failed",
        crate::types::turn::TurnStatus::Cancelled => "cancelled",
        _ => return Ok(()),
    };
    update_ledger(iii, &record.session_id, timeout, |old| {
        let mut rows = ledger(old)?;
        let mut changed = false;
        for row in &mut rows {
            for proof in &mut row.extraction {
                if proof.session_id == record.session_id
                    && proof.turn_id == record.turn_id
                    && proof
                        .terminal
                        .as_ref()
                        .is_none_or(|t| exchange_finished && !t.exchange_finished)
                {
                    proof.terminal = Some(ExtractionTerminal {
                        status: status.into(),
                        exchange_finished,
                    });
                    changed = true;
                }
            }
        }
        Ok(changed.then(|| serde_json::to_value(rows).expect("extraction boundary")))
    })
    .await
}

pub(super) async fn project_extraction_history(
    iii: &iii_sdk::IIIClient,
    input: &mut Value,
    entries: &[LoadedEntry],
    timeout: u64,
) {
    let Some(session) = input["session_id"].as_str() else {
        return;
    };
    if input["recall_context"]["version"] != 1
        || input["recall_context"]["session_id"] != session
        || input["recall_context"]["turn_id"] != input["turn_id"]
    {
        return;
    }
    if let Ok(history) = extraction_history(iii, session, entries, timeout).await {
        input["recall_context"]["extraction_history"] = history;
    }
}

async fn extraction_history(
    iii: &iii_sdk::IIIClient,
    session: &str,
    entries: &[LoadedEntry],
    timeout: u64,
) -> Result<Value, HarnessError> {
    let value = crate::state::state_get(iii, SCOPE, session, timeout).await?;
    let rows: Vec<Receipt> = if value.is_null() {
        Vec::new()
    } else {
        serde_json::from_value(value)
            .map_err(|e| HarnessError::State(format!("invalid extraction history: {e}")))?
    };
    let mut proofs = Vec::new();
    for row in rows {
        if row.session_id != session {
            continue;
        }
        let Some(entry) = entries.iter().find(|e| e.entry_id == row.entry_id) else {
            continue;
        };
        if row.full_text_sha256 != full_stamp(entry)
            || row.query
                != entry
                    .message
                    .as_ref()
                    .and_then(|m| query(&entry.entry_id, m))
                    .map(stamp)
        {
            continue;
        }
        for proof in row.extraction {
            if proof.version == 1
                && proof.session_id == session
                && proof.entry_id == entry.entry_id
                && !proof.turn_id.is_empty()
                && proof.turn_id.len() <= 256
                && matches!(proof.policy.as_str(), "on" | "off" | "unknown")
                && (proof.policy != "on"
                    || proof
                        .bank
                        .as_ref()
                        .is_some_and(|b| !b.is_empty() && b.len() <= 256))
                && proof.terminal.as_ref().is_some_and(|t| {
                    matches!(t.status.as_str(), "completed" | "failed" | "cancelled")
                })
                && full_stamp(entry).as_ref() == Some(&proof.text_sha256)
            {
                proofs.push(json!({"attestation":proof,"source_kind":row.channel}));
            }
        }
    }
    Ok(
        json!({"version":1,"retention":{"delivery_entries":MAX_RECEIPTS,"turns_per_entry":MAX_EXTRACTION_TURNS},"missing":"unknown","rows":proofs}),
    )
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

fn ledger(value: &Value) -> Result<Vec<Receipt>, HarnessError> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    Vec::deserialize(value)
        .map_err(|e| HarnessError::State(format!("invalid recall receipt ledger: {e}")))
}

/// The one read-modify-CAS loop over a session's ledger. `edit` gets the
/// stored value (null when absent) and returns what to store, or `None` when
/// nothing changes. A lost CAS returns what is stored, so the retry needs no
/// second read.
async fn update_ledger(
    iii: &iii_sdk::IIIClient,
    session: &str,
    timeout: u64,
    mut edit: impl FnMut(&Value) -> Result<Option<Value>, HarnessError>,
) -> Result<(), HarnessError> {
    let mut old = crate::state::state_get(iii, SCOPE, &key(session), timeout).await?;
    let mut stalls = 0;
    // A burst of writers drains well inside this; a ledger that never settles
    // must not hold a delivery, an edit or a finalizer forever.
    for attempt in 1_u64..=64 {
        let Some(value) = edit(&old)? else {
            return Ok(());
        };
        let expected = (!old.is_null()).then(|| old.clone());
        let Some(current) =
            crate::state::cas_value(iii, SCOPE, &key(session), expected, value, timeout).await?
        else {
            return Ok(());
        };
        // A changed ledger means another writer landed, so a burst of any
        // size drains; only a store that keeps refusing its own value is stuck.
        if current == old {
            stalls += 1;
            if stalls == 8 {
                break;
            }
        }
        old = current;
        // Deterministic 1..=8 ms spacing so a burst does not hammer the hot key.
        tokio::time::sleep(std::time::Duration::from_millis(attempt.min(8))).await;
    }
    Err(HarnessError::State(
        "concurrent recall ledger update".into(),
    ))
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
        if delivery.continuation_entry_id.is_none() {
            // A long idempotency key must not fail an ordinary send: skip the
            // receipt, so this turn's recall source resolves as unknown.
            tracing::warn!(
                session,
                entry,
                "recall delivery ids over the ledger bound; delivering without recall authority"
            );
            return Ok(());
        }
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
        full_text_sha256: full_message_stamp(delivery.message),
        query: query(entry, delivery.message).map(stamp),
        continuation,
        extraction: Vec::new(),
    };
    let cfg = deps.cfg().await;
    update_ledger(&deps.iii, session, cfg.session_timeout_ms, |old| {
        let rows = ledger(old)?;
        if let Some(existing) = rows.iter().find(|x| x.entry_id == entry) {
            let mut delivery_only = existing.clone();
            delivery_only.extraction.clear();
            let mut incoming_delivery = r.clone();
            // Legacy receipts still deduplicate; they cannot certify full text.
            if delivery_only.full_text_sha256.is_none() {
                incoming_delivery.full_text_sha256 = None;
            }
            return if delivery_only == incoming_delivery {
                Ok(None)
            } else {
                Err(HarnessError::InvalidRequest(
                    "delivery entry id already has different provenance".into(),
                ))
            };
        }
        Ok(Some(
            serde_json::to_value(bounded_receipts(rows, r.clone())).expect("receipt ledger"),
        ))
    })
    .await
}

/// An operator edit can refresh only an existing direct receipt matching the
/// queued row's old text. Notification/agent/legacy rows never gain authority.
/// A fork or a forged queued row cannot borrow another session's receipt.
/// Never fails the edit: a receipt left unrefreshed no longer matches the row,
/// so that turn's recall source resolves as unknown, as on the send path.
pub(crate) async fn edit_direct(
    deps: &Deps,
    session: &str,
    entry: &str,
    old_message: &AgentMessage,
    new_message: &AgentMessage,
) {
    let timeout = deps.cfg().await.session_timeout_ms;
    let refreshed = update_ledger(&deps.iii, session, timeout, |old| {
        let mut rows = ledger(old)?;
        let Some(r) = rows
            .iter_mut()
            .find(|r| r.session_id == session && r.entry_id == entry)
        else {
            return Ok(None);
        };
        if r.channel != DeliveryChannel::Direct {
            return Ok(None);
        }
        if r.query == query(entry, new_message).map(stamp)
            && r.full_text_sha256 == full_message_stamp(new_message)
        {
            return Ok(None);
        }
        if r.query != query(entry, old_message).map(stamp)
            || r.full_text_sha256
                .as_ref()
                .is_some_and(|digest| Some(digest) != full_message_stamp(old_message).as_ref())
        {
            return Err(HarnessError::InvalidRequest(
                "queued edit does not match its direct delivery receipt".into(),
            ));
        }
        r.query = query(entry, new_message).map(stamp);
        r.full_text_sha256 = full_message_stamp(new_message);
        // Replacing the instruction is a new request, never implicit continuation.
        r.continuation = None;
        // Keep the ack-once tombstone: old digests no longer project, and a
        // source edit must not let the same turn acknowledge a second policy.
        Ok(Some(serde_json::to_value(rows).expect("edit ledger")))
    })
    .await;
    if let Err(error) = refreshed {
        tracing::warn!(session, entry, %error, "recall receipt not refreshed; queued edit lands without recall authority");
    }
}

/// Scoped lifecycle cleanup: no global scan, no transcript contents retained.
/// Retries a lost race (a finalizer stamping the same ledger) like every
/// other ledger writer, so it never fails the session deletion.
pub(crate) async fn purge(deps: &Deps, session: &str) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    // Never parses the ledger: a corrupt one must not block session deletion.
    update_ledger(&deps.iii, session, timeout, |old| {
        Ok((!old.is_null()).then_some(Value::Null))
    })
    .await
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
            // The query is bounded; an unrefreshed edit past the bound shows
            // only here. Legacy receipts carry no digest and still resolve.
            && r.full_text_sha256
                .as_ref()
                .is_none_or(|digest| full_stamp(entry).as_ref() == Some(digest))
    });
    // The send bounded the reference's age; here it only has to still precede
    // this entry. A window from the tail would drop it as the turn's own steps
    // append, or as a running turn writes ahead of a queued message.
    let before = entries
        .iter()
        .position(|e| e.entry_id == entry.entry_id)
        .map_or(entries, |at| &entries[..at]);
    let continuation = valid
        .and_then(|r| r.continuation.as_ref())
        .and_then(|prior| {
            before
                .iter()
                .rev()
                .find(|e| e.entry_id == prior.entry_id)
                .and_then(|e| e.message.as_ref().and_then(|m| query(&e.entry_id, m)))
                .filter(|q| stamp(q.clone()) == *prior)
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
    // Historical proof is deliberately not persisted in TurnRecord (whose
    // ordinary bookkeeping surface is public). The trusted Memory call alone
    // receives a fresh active-path projection from the private receipt ledger.
    Some(envelope(record, entry, authority.as_ref(), entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str, text: &str) -> LoadedEntry {
        serde_json::from_value(json!({"entry_id":id,"message":{"role":"user","content":[{"type":"text","text":text}],"timestamp":0},"origin":{"kind":"direct"}})).unwrap()
    }
    #[test]
    fn acknowledged_policy_carries_memory_derived_and_omits_it_when_false() {
        let derived = acknowledged_policy(
            &json!({"memory_extraction":true,"memory_bank":"main","memory_derived":true}),
        );
        assert_eq!(
            derived,
            ("on".into(), Some("main".into()), None, None, true)
        );
        let named = acknowledged_policy(&json!({"memory_extraction":true,"memory_bank":"blog"}));
        assert!(!named.4);
        let row = |derived| ExtractionAttestation {
            version: 1,
            session_id: "s".into(),
            turn_id: "t".into(),
            entry_id: "e".into(),
            text_sha256: "h".into(),
            policy: "on".into(),
            bank: Some("main".into()),
            layer: None,
            agent: None,
            derived,
            terminal: None,
        };
        assert_eq!(
            serde_json::to_value(row(true)).unwrap()["derived"],
            json!(true)
        );
        assert!(serde_json::to_value(row(false))
            .unwrap()
            .get("derived")
            .is_none());
    }
    #[test]
    fn acknowledgement_input_keeps_what_the_ack_reads_and_drops_the_prompt() {
        let context = json!({"version":1,"session_id":"s","turn_id":"t",
            "source":{"kind":"direct","entry_id":"e"},"query":{"entry_id":"e","text":"q"},
            "continuation":null,"extraction_history":{"rows":[]}});
        let input = json!({"point":"pre_generate","session_id":"s","turn_id":"t","step":3,
            "recall_context":context,"generate":{"messages":["the whole prompt"]}});
        let ack = acknowledgement_input(&input);
        assert!(ack.get("generate").is_none());
        assert!(ack["recall_context"].get("extraction_history").is_none());
        for key in ["point", "session_id", "turn_id"] {
            assert_eq!(ack[key], input[key]);
        }
        for key in ["version", "session_id", "turn_id", "source", "query"] {
            assert_eq!(ack["recall_context"][key], context[key]);
        }
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
                    full_text_sha256: None,
                    query: Some(stamp(Query {
                        entry_id: format!("e{n}"),
                        text: "Sensitive topic".into(),
                    })),
                    continuation: None,
                    extraction: Vec::new(),
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
                full_text_sha256: full_stamp(&e),
                query: e.message.as_ref().and_then(|m| query("e1", m)).map(stamp),
                continuation: None,
                extraction: Vec::new(),
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
            full_text_sha256: full_stamp(&current),
            query: current
                .message
                .as_ref()
                .and_then(|m| query("e1", m))
                .map(stamp),
            continuation: old.message.as_ref().and_then(|m| query("e0", m)).map(stamp),
            extraction: Vec::new(),
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
