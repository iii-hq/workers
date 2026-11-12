//! Authenticated history intake; never reads authority from queue job JSON.
//! Recovery rechecks the persisted evidence against the actual source path.
use crate::{deps::Deps, hooks::PreGenerateInput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    pub status: String,
    pub exchange_finished: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub version: u8,
    pub session_id: String,
    pub turn_id: String,
    pub entry_id: String,
    pub text_sha256: String,
    pub policy: String,
    pub bank: Option<String>,
    pub layer: Option<String>,
    pub agent: Option<String>,
    /// The stamp's `memory_derived`; absent on an older row.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
    pub terminal: Terminal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub source_kind: String,
    pub attestation: Attestation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retention {
    delivery_entries: usize,
    turns_per_entry: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    version: u8,
    retention: Retention,
    missing: String,
    rows: Vec<Row>,
}
// Conservative budgets, not truncation: pending snapshots retain at most
// 96KiB text; larger bounded sources retain full SHA-256 provenance instead.
// JSON keys/strings reserve six bytes per UTF-8 byte for escaping. Checks
// begin AFTER SDK network JSON decoding; they cannot bound that decoding.
pub(crate) const ROW_BYTES: usize = 16 * 1024;
pub(crate) const HISTORY_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const SOURCE_BYTES: usize = 1024 * 1024;
pub(crate) const PATH_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const TEXT_BYTES: usize = 96 * 1024;
pub(crate) const RECORD_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn charge(total: &mut usize, amount: usize, cap: usize) -> Result<(), String> {
    *total = total
        .checked_add(amount)
        .filter(|n| *n <= cap)
        .ok_or("history byte bound")?;
    Ok(())
}

/// Borrowed traversal bounds unknown fields, metadata, depth and node count
/// before cloning/deserialization, without allocating a serialized buffer.
pub(crate) fn value_bytes(value: &Value, cap: usize) -> Result<usize, String> {
    fn walk(
        v: &Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
        cap: usize,
    ) -> Result<(), String> {
        if depth > 32 {
            return Err("history nesting bound".into());
        }
        charge(nodes, 1, 65536)?;
        charge(bytes, 2, cap)?;
        match v {
            Value::String(s) => charge(
                bytes,
                s.len().checked_mul(6).ok_or("history byte overflow")?,
                cap,
            )?,
            Value::Array(a) => {
                for v in a {
                    charge(bytes, 1, cap)?;
                    walk(v, depth + 1, nodes, bytes, cap)?;
                }
            }
            Value::Object(o) => {
                for (k, v) in o {
                    charge(
                        bytes,
                        k.len()
                            .checked_mul(6)
                            .and_then(|n| n.checked_add(4))
                            .ok_or("history byte overflow")?,
                        cap,
                    )?;
                    walk(v, depth + 1, nodes, bytes, cap)?;
                }
            }
            _ => charge(bytes, 32, cap)?,
        }
        Ok(())
    }
    let (mut nodes, mut bytes) = (0, 0);
    walk(value, 0, &mut nodes, &mut bytes, cap)?;
    Ok(bytes)
}

/// Typed rows are checked before copying; fixed overhead covers schema keys
/// and punctuation. This deliberately overestimates the persisted JSON size.
pub(crate) fn rows_bytes(rows: &[Row]) -> Result<usize, String> {
    rows_bytes_iter(rows.iter())
}
pub(crate) fn rows_bytes_iter<'a>(
    rows: impl IntoIterator<Item = &'a Row>,
) -> Result<usize, String> {
    let mut count = 0;
    let mut total = 2;
    let mut turns = HashSet::new();
    for row in rows {
        charge(&mut count, 1, 1024)?;
        let a = &row.attestation;
        let mut size = 1024;
        for s in [
            Some(row.source_kind.as_str()),
            Some(a.session_id.as_str()),
            Some(a.turn_id.as_str()),
            Some(a.entry_id.as_str()),
            Some(a.text_sha256.as_str()),
            Some(a.policy.as_str()),
            a.bank.as_deref(),
            a.layer.as_deref(),
            a.agent.as_deref(),
            Some(a.terminal.status.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            if !bounded(s) {
                return Err("history string bound".into());
            }
            charge(
                &mut size,
                s.len().checked_mul(6).ok_or("history byte overflow")?,
                ROW_BYTES,
            )?;
        }
        if !turns.insert(a.turn_id.as_str()) {
            return Err("history duplicate turn".into());
        }
        charge(&mut total, size, HISTORY_BYTES)?;
    }
    Ok(total)
}

pub(crate) fn bounded(s: &str) -> bool {
    s.len() <= 256 && !s.trim().is_empty()
}

// Intake and persisted model snapshots have independent finite budgets.
pub(crate) const SOURCE_TEXT_BYTES: usize = 1024 * 1024;

/// Actual JSON cost for known text only; unknown fields keep the conservative
/// sixfold budget and depth/node limits before any source clone.
pub(crate) fn source_bytes(item: &Value) -> Result<usize, String> {
    fn string_bytes(text: &str) -> Result<usize, String> {
        let mut size = 2;
        for byte in text.bytes() {
            let cost = match byte {
                b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
                0..=31 => 6,
                _ => 1,
            };
            charge(&mut size, cost, SOURCE_BYTES)?;
        }
        Ok(size)
    }
    let mut size = 2;
    let object = item.as_object().ok_or("history source schema")?;
    for (key, value) in object {
        charge(&mut size, key.len() * 6 + 4, SOURCE_BYTES)?;
        if key != "message" {
            charge(&mut size, value_bytes(value, SOURCE_BYTES)?, SOURCE_BYTES)?;
            continue;
        }
        let message = value.as_object().ok_or("history message schema")?;
        for (key, value) in message {
            charge(&mut size, key.len() * 6 + 4, SOURCE_BYTES)?;
            if key != "content" {
                charge(&mut size, value_bytes(value, SOURCE_BYTES)?, SOURCE_BYTES)?;
                continue;
            }
            content_bytes(Some(value))?;
            if let Some(text) = value.as_str() {
                charge(&mut size, string_bytes(text)?, SOURCE_BYTES)?;
            } else if let Some(parts) = value.as_array() {
                charge(&mut size, 2, SOURCE_BYTES)?;
                if parts.len() > 4096 {
                    return Err("history content block bound".into());
                }
                for part in parts {
                    if part["type"] == "text" && part.is_object() {
                        charge(&mut size, 3, SOURCE_BYTES)?;
                        for (key, value) in part.as_object().unwrap() {
                            charge(&mut size, key.len() * 6 + 4, SOURCE_BYTES)?;
                            let bytes = match (key.as_str(), value.as_str()) {
                                ("text", Some(text)) => string_bytes(text)?,
                                _ => value_bytes(value, SOURCE_BYTES)?,
                            };
                            charge(&mut size, bytes, SOURCE_BYTES)?;
                        }
                    } else {
                        charge(&mut size, value_bytes(part, SOURCE_BYTES)?, SOURCE_BYTES)?;
                    }
                }
            } else {
                charge(&mut size, value_bytes(value, SOURCE_BYTES)?, SOURCE_BYTES)?;
            }
        }
    }
    Ok(size)
}

/// Measure before joining; separators count even for empty text blocks.
pub(crate) fn content_bytes(content: Option<&Value>) -> Result<usize, String> {
    let mut bytes = 0;
    let mut first = true;
    if let Some(parts) = content.and_then(Value::as_array) {
        for text in parts
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
        {
            if !first {
                charge(&mut bytes, 1, SOURCE_TEXT_BYTES)?;
            }
            first = false;
            charge(&mut bytes, text.len(), SOURCE_TEXT_BYTES)?;
        }
    } else if let Some(text) = content.and_then(Value::as_str) {
        charge(&mut bytes, text.len(), SOURCE_TEXT_BYTES)?;
    }
    Ok(bytes)
}

/// Validate the identifiers of a borrowed SDK reply before DeltaReader or
/// root recovery clone them. Sizes are not capped here: a run captures only
/// what the model reads of a long entry, never a failed read.
pub(crate) fn source_page(reply: &Value) -> Result<(), String> {
    if let Some(cursor) = reply.get("next_cursor").and_then(Value::as_str) {
        if !bounded(cursor) {
            return Err("history cursor byte bound".into());
        }
    }
    let items = reply["messages"]
        .as_array()
        .filter(|items| items.len() <= 100)
        .ok_or("history page bound")?;
    for item in items {
        if !item["entry_id"].as_str().is_some_and(bounded) {
            return Err("history entry id byte bound".into());
        }
        for field in [
            "turn_id",
            "memory_bank",
            "memory_agent",
            "memory_layer",
            "agent",
        ] {
            if item["origin"][field].as_str().is_some_and(|s| !bounded(s)) {
                return Err("history origin byte bound".into());
            }
        }
    }
    Ok(())
}

/// The finished failed or cancelled requests a delivery attests: the only
/// rows that can add recovery authority, so [`import`] runs for no other.
pub(crate) fn recoverable(input: &PreGenerateInput) -> Vec<Row> {
    input
        .recall_context
        .as_ref()
        .and_then(|context| context["extraction_history"]["rows"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|row| Row::deserialize(row).ok())
        .filter(|row| {
            row.attestation.terminal.exchange_finished
                && matches!(
                    row.attestation.terminal.status.as_str(),
                    "failed" | "cancelled"
                )
        })
        .collect()
}

/// Entire delivery is validated before any write. Unknown/missing never erases
/// earlier evidence, changes its bank, or advances a cursor.
/// The caller authenticated the delivery first.
pub(crate) async fn import(deps: &Deps, input: &PreGenerateInput) -> Result<(), String> {
    let Some(context) = input.recall_context.as_ref() else {
        return Ok(());
    };
    let Some(value) = context.get("extraction_history") else {
        return Ok(());
    };
    value_bytes(context, HISTORY_BYTES)?;
    if !bounded(&input.session_id)
        || input.turn_id.as_deref().is_none_or(|turn| !bounded(turn))
        || context["version"] != 1
        || context["session_id"] != input.session_id
        || context["turn_id"].as_str() != input.turn_id.as_deref()
    {
        return Err("history context mismatch".into());
    }
    if let Some(rows) = value.get("rows").and_then(Value::as_array) {
        if rows.len() > 1024 {
            return Err("history row bound".into());
        }
        for row in rows {
            value_bytes(row, ROW_BYTES)?;
        }
    }
    let h: History = serde_json::from_value(value.clone()).map_err(|_| "history schema")?;
    if h.version != 1
        || h.missing != "unknown"
        || h.retention.delivery_entries != 128
        || h.retention.turns_per_entry != 8
        || h.rows.len() > 1024
    {
        return Err("history bounds".into());
    }
    rows_bytes(&h.rows)?;
    if h.rows.is_empty() {
        return Ok(());
    }
    // Fetch only a bounded actual active path, not a future-session policy scan.
    let mut path = HashMap::new();
    let mut order = Vec::new();
    let mut cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut complete = false;
    let mut path_bytes = 0;
    for _ in 0..32 {
        let mut payload = json!({"session_id":input.session_id,"limit":100,"roles":["user","assistant"],"include_image_data":false});
        if let Some(c) = &cursor {
            payload["cursor"] = json!(c);
        }
        let reply = deps
            .iii
            .call(
                iii_sdk::CallRequest::new("session::messages", payload)
                    .metadata(json!({}))
                    .timeout(std::time::Duration::from_millis(400)),
            )
            .await
            .map_err(|_| "history path unavailable")?;
        let entries = reply["messages"]
            .as_array()
            .filter(|e| e.len() <= 100)
            .ok_or("history path schema")?;
        source_page(&reply)?;
        for item in entries {
            charge(&mut path_bytes, source_bytes(item)?, PATH_BYTES)?;
            join_text(item.pointer("/message/content"))?;
            let id = item["entry_id"]
                .as_str()
                .filter(|s| bounded(s))
                .ok_or("history path id")?;
            order.push(id.to_owned());
            if path.insert(id.to_owned(), item.clone()).is_some() {
                return Err("history path duplicate".into());
            }
        }
        match reply.get("next_cursor") {
            None | Some(Value::Null) => {
                complete = true;
                break;
            }
            Some(Value::String(c)) if bounded(c) && cursors.insert(c.clone()) => {
                cursor = Some(c.clone())
            }
            _ => return Err("history path cursor".into()),
        }
    }
    if !complete
        || !path.contains_key(
            context["source"]["entry_id"]
                .as_str()
                .ok_or("history source")?,
        )
    {
        return Err("history active path unavailable".into());
    }
    let mut counts = HashMap::new();
    let mut turns = HashSet::new();
    for row in &h.rows {
        let a = &row.attestation;
        let count = counts.entry(&a.entry_id).or_insert(0usize);
        *count += 1;
        let item = path.get(&a.entry_id).ok_or("history source missing")?;
        let text = join_text(item.pointer("/message/content"))?;
        if a.version != 1
            || a.session_id != input.session_id
            || !bounded(&a.turn_id)
            || input.turn_id.as_deref() == Some(a.turn_id.as_str())
            || !bounded(&a.entry_id)
            || *count > 8
            || !turns.insert(&a.turn_id)
            || !matches!(
                row.source_kind.as_str(),
                "direct" | "invoked-agent" | "notification"
            )
            || !matches!(a.policy.as_str(), "on" | "off" | "unknown")
            || (a.policy == "on" && a.bank.is_none())
            || [&a.bank, &a.layer, &a.agent]
                .into_iter()
                .flatten()
                .any(|s| !bounded(s))
            || !matches!(
                a.terminal.status.as_str(),
                "completed" | "failed" | "cancelled"
            )
            || item["message"]["role"] != "user"
            || text.trim().is_empty()
            || a.text_sha256 != sha256(text.as_bytes())
        {
            return Err("history contradictory proof".into());
        }
    }
    if counts.len() > 128 {
        return Err("history entry bounds".into());
    }
    // Rows of entries the cursor already passed are never read again: drop
    // them, stored and delivered, so the ledger holds only pending turns.
    // The delivered ones are remembered, so the next step that delivers them
    // again imports nothing. An unreadable cursor prunes nothing.
    let cursor = crate::extract::cursor_get(&deps.iii, &input.session_id)
        .await
        .ok()
        .flatten();
    let confirmed: HashSet<&str> = match cursor
        .as_deref()
        .and_then(|cursor| order.iter().position(|id| id == cursor))
    {
        Some(at) => order[..=at].iter().map(String::as_str).collect(),
        None => HashSet::new(),
    };
    let (rows, dismissed): (Vec<Row>, Vec<Row>) = h
        .rows
        .into_iter()
        .partition(|row| !confirmed.contains(row.attestation.entry_id.as_str()));
    let store = deps.store().await;
    crate::store::run_blocking(|| {
        store.prune_authenticated_history(&input.session_id, &confirmed)?;
        store.save_authenticated_history(&input.session_id, &rows)?;
        store.save_dismissed_history(&input.session_id, &dismissed)
    })
    .await
    .map_err(|e| format!("history persistence failed: {e}"))?;
    // The session's purge runs once it is gone: one that ran before this save
    // is redone here, so the ledger never outlives the session.
    if crate::scope::session_meta(&deps.iii, input.session_id.clone())
        .await?
        .is_none()
    {
        return crate::store::run_blocking(|| store.purge_session_extraction(&input.session_id))
            .await
            .map_err(|e| format!("history purge failed: {e}"));
    }
    // Queue messages carry correlation only, never history or a trusted flag.
    // Re-delivery is safe: extraction receipts bind effects to the original source.
    let cfg = deps.config().await;
    if cfg.extraction_enabled {
        let stored = crate::store::run_blocking(|| store.authenticated_history(&input.session_id))
            .await
            .map_err(|_| "history read failed")?;
        for row in stored {
            let a = &row.attestation;
            let Some(item) = path.get(&a.entry_id) else {
                continue;
            };
            let followed_by_assistant = order
                .iter()
                .position(|id| id == &a.entry_id)
                .and_then(|at| order.get(at + 1))
                .and_then(|id| path.get(id))
                .is_some_and(|item| item["message"]["role"] == "assistant");
            if followed_by_assistant
                || !row.matches_source(item)
                || !a.terminal.exchange_finished
                || !matches!(a.terminal.status.as_str(), "failed" | "cancelled")
            {
                continue;
            }
            let scope = match row.scope() {
                Ok(Some(scope)) => scope,
                _ => continue,
            };
            if scope.bank != cfg.default_bank && store.is_deleted(&scope.bank) {
                continue;
            }
            let job = crate::hooks::ExtractJobInput {
                session_id: input.session_id.clone(),
                turn_id: Some(a.turn_id.clone()),
                bank: Some(scope.bank),
                derived: scope.derived,
                agent: scope.agent,
                skip: false,
                until: Some(a.entry_id.clone()),
            };
            deps.iii
                .call(
                    iii_sdk::CallRequest::new(
                        "engine::queue::enqueue",
                        json!({
                            "queue":crate::hooks::EXTRACTION_QUEUE,
                            "function_id":crate::hooks::EXTRACT_JOB_FN,"data":job,
                            "messageReceiptId":format!("memx-history-{}",a.turn_id)
                        }),
                    )
                    .metadata(json!({}))
                    .timeout(std::time::Duration::from_millis(400)),
                )
                .await
                .map_err(|_| "history recovery scheduling failed")?;
        }
    }
    Ok(())
}

impl Row {
    pub(crate) fn matches_source(&self, item: &Value) -> bool {
        let a = &self.attestation;
        if source_bytes(item).is_err() {
            return false;
        }
        let Ok(text) = join_text(item.pointer("/message/content")) else {
            return false;
        };
        item["entry_id"] == a.entry_id
            && item["message"]["role"] == "user"
            && item.get("origin").is_none_or(Value::is_null)
            && matches!(self.source_kind.as_str(), "direct" | "invoked-agent")
            && a.text_sha256 == sha256(text.as_bytes())
    }

    pub(crate) fn scope(&self) -> Result<Option<crate::scope::Resolved>, String> {
        let a = &self.attestation;
        match a.policy.as_str() {
            "off" => Ok(None),
            "on" => {
                let bank = a.bank.as_ref().ok_or("history bank missing")?;
                crate::store::validate_name(bank).map_err(|_| "history invalid bank")?;
                if a.layer
                    .as_ref()
                    .is_some_and(|v| crate::store::validate_name(v).is_err())
                {
                    return Err("history invalid layer".into());
                }
                let agent = a
                    .agent
                    .as_deref()
                    .map(|v| {
                        v.strip_prefix("agent-")
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned)
                            .ok_or("history invalid agent")
                    })
                    .transpose()?;
                Ok(Some(crate::scope::Resolved {
                    bank: bank.clone(),
                    derived: a.derived || a.layer.is_some() || agent.is_some(),
                    agent,
                }))
            }
            _ => Err("history unknown policy retained".into()),
        }
    }
}

fn join_text(content: Option<&Value>) -> Result<String, String> {
    let size = content_bytes(content)?;
    let mut text = String::with_capacity(size);
    if let Some(blocks) = content.and_then(Value::as_array) {
        let mut first = true;
        for part in blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
        {
            if !first {
                text.push('\n');
            }
            first = false;
            text.push_str(part);
        }
    } else if let Some(part) = content.and_then(Value::as_str) {
        text.push_str(part);
    }
    Ok(text)
}

// SHA-256 over the full joined text, with no new dependency or model service.
pub(crate) fn sha256(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut bytes = input.to_vec();
    bytes.push(0x80);
    while bytes.len() % 64 != 56 {
        bytes.push(0);
    }
    bytes.extend_from_slice(&(input.len() as u64 * 8).to_be_bytes());
    for chunk in bytes.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(b.try_into().unwrap());
        }
        for i in 16..64 {
            let x = w[i - 15];
            let y = w[i - 2];
            w[i] = w[i - 16]
                .wrapping_add(x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3))
                .wrapping_add(w[i - 7])
                .wrapping_add(y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10));
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let t = hh
                .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let u = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
                .wrapping_add((a & b) ^ (a & c) ^ (b & c));
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t);
            d = c;
            c = b;
            b = a;
            a = t.wrapping_add(u);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn checked_byte_budget_and_full_join_boundaries() {
        let mut total = usize::MAX - 1;
        assert!(super::charge(&mut total, 2, usize::MAX).is_err());
        let mut total = 15;
        super::charge(&mut total, 1, 16).unwrap();
        assert_eq!(total, 16);
        assert!(super::charge(&mut total, 1, 16).is_err());
        let exact = serde_json::json!([{"type":"text","text":"a".repeat(super::TEXT_BYTES)}]);
        let text = super::join_text(Some(&exact)).unwrap();
        assert_eq!(text.len(), super::TEXT_BYTES);
        assert_eq!(
            super::sha256(text.as_bytes()),
            "6e03ec5e819aa1579fd8d3e35ea4a88cddbedc2707f1bae3fd3c08710a515831"
        );
        let overflow = serde_json::json!([
            {"type":"text","text":"a".repeat(super::SOURCE_TEXT_BYTES / 2)},
            {"type":"text","text":"a".repeat(super::SOURCE_TEXT_BYTES / 2)}]);
        assert!(super::join_text(Some(&overflow)).is_err());
        let value = serde_json::json!("a");
        assert_eq!(super::value_bytes(&value, 8).unwrap(), 8);
        assert!(super::value_bytes(&value, 7).is_err());
        let mut nested = serde_json::Value::Null;
        for _ in 0..34 {
            nested = serde_json::json!([nested]);
        }
        assert!(super::value_bytes(&nested, super::HISTORY_BYTES).is_err());
    }

    #[test]
    fn full_text_digest_known_vectors() {
        assert_eq!(
            super::sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            super::sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            super::sha256(&vec![b'a'; 1000000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
}
