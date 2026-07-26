//! `session-manager` client: ensure/create a session, append and update
//! messages, set status, and load the active path. Required dependency.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::HarnessError;
use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;

/// One active-path entry as returned by `session::messages`.
#[derive(Debug, Clone, Deserialize)]
pub struct LoadedEntry {
    pub entry_id: String,
    #[serde(default)]
    pub message: Option<AgentMessage>,
    #[serde(default)]
    pub custom: Option<LoadedCustom>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoadedCustom {
    pub custom_type: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone)]
pub struct SessionClient {
    iii: Arc<IIIClient>,
    timeout_ms: u64,
}

impl SessionClient {
    pub fn new(iii: Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { iii, timeout_ms }
    }

    async fn call(&self, function_id: &str, payload: Value) -> Result<Value, HarnessError> {
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(self.timeout_ms),
        };
        let res = match self.iii.namespace() {
            Some(ns) => self.iii.trigger(request.namespace(ns)).await,
            None => self.iii.trigger(request).await,
        };
        res.map_err(|e| HarnessError::Dependency(format!("{function_id}: {e}")))
    }

    /// Best-effort session title from `session::get` — `None` when the
    /// session is unknown, untitled, or the call fails. Trace display
    /// metadata must never fail or delay a step, so errors are swallowed.
    pub async fn title(&self, session_id: &str) -> Option<String> {
        let resp = self
            .call("session::get", json!({ "session_id": session_id }))
            .await
            .ok()?;
        let title = resp.get("meta")?.get("title")?.as_str()?.trim().to_string();
        (!title.is_empty()).then_some(title)
    }

    /// Idempotently ensure a session exists, applying `metadata` on creation.
    /// Returns whether this call CREATED the session — `false` means it already
    /// existed and the supplied metadata (e.g. parent linkage) was NOT applied.
    pub async fn ensure(
        &self,
        session_id: &str,
        title: Option<&str>,
        metadata: Option<&Value>,
    ) -> Result<bool, HarnessError> {
        let mut payload = json!({ "session_id": session_id });
        if let Some(t) = title {
            payload["title"] = json!(t);
        }
        if let Some(m) = metadata {
            payload["metadata"] = m.clone();
        }
        let resp = self.call("session::ensure", payload).await?;
        Ok(resp
            .get("created")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// Create a fresh session, returning its id.
    pub async fn create(
        &self,
        title: Option<&str>,
        metadata: Option<&Value>,
    ) -> Result<String, HarnessError> {
        let mut payload = json!({});
        if let Some(t) = title {
            payload["title"] = json!(t);
        }
        if let Some(m) = metadata {
            payload["metadata"] = m.clone();
        }
        let resp = self.call("session::create", payload).await?;
        resp.get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| HarnessError::Dependency("session::create: no session_id".into()))
    }

    /// Set the coarse session status (idle/working/done/error).
    pub async fn set_status(
        &self,
        session_id: &str,
        status: &str,
        reason: Option<&str>,
    ) -> Result<(), HarnessError> {
        let mut payload = json!({ "session_id": session_id, "status": status });
        if let Some(r) = reason {
            payload["reason"] = json!(r);
        }
        self.call("session::set-status", payload).await.map(|_| ())
    }

    /// Append a message entry (idempotent on `entry_id`). Returns the entry id.
    pub async fn append(
        &self,
        session_id: &str,
        message: &AgentMessage,
        entry_id: Option<&str>,
        parent_id: Option<&str>,
        origin: Option<&Value>,
    ) -> Result<String, HarnessError> {
        let mut payload = json!({
            "session_id": session_id,
            "message": message,
        });
        if let Some(id) = entry_id {
            payload["entry_id"] = json!(id);
        }
        if let Some(p) = parent_id {
            payload["parent_id"] = json!(p);
        }
        if let Some(o) = origin {
            payload["origin"] = o.clone();
        }
        let resp = self.call("session::append", payload).await?;
        resp.get("entry_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| HarnessError::Dependency("session::append: no entry_id".into()))
    }

    /// Append a `custom` (kind: custom) bookkeeping entry — used for the
    /// compaction record. Idempotent on `entry_id`.
    pub async fn append_custom(
        &self,
        session_id: &str,
        custom_type: &str,
        data: Value,
        entry_id: &str,
        origin: Option<&Value>,
    ) -> Result<(), HarnessError> {
        let payload = custom_append_payload(session_id, custom_type, data, entry_id, origin);
        self.call("session::append", payload).await.map(|_| ())
    }

    /// Replace a message entry's content (streaming + final write). Returns
    /// the new revision.
    ///
    /// Fires once per stream batch — dozens of calls per turn — so the call
    /// site is tagged `iii.tag.hidden`: its spans (and the session-manager
    /// handler span behind them) stack into the trace UI's internal section,
    /// hidden by default.
    pub async fn update_message(
        &self,
        session_id: &str,
        entry_id: &str,
        content: &[ContentBlock],
        details: Option<&Value>,
        origin: Option<&Value>,
    ) -> Result<u64, HarnessError> {
        let mut payload = json!({
            "session_id": session_id,
            "entry_id": entry_id,
            "content": content,
        });
        if let Some(d) = details {
            payload["details"] = d.clone();
        }
        if let Some(o) = origin {
            payload["origin"] = o.clone();
        }
        let resp = crate::trace_tags::run_hidden(
            "session updates",
            self.call("session::update-message", payload),
        )
        .await?;
        Ok(resp.get("revision").and_then(Value::as_u64).unwrap_or(0))
    }

    /// Load the **entire** active path, oldest first. `include_custom`
    /// interleaves `kind: custom` entries (where the compaction record lives).
    ///
    /// `session::messages` is paginated (session-manager caps a single page at
    /// `max_list_limit`, default 500, and serves `default_list_limit`, default
    /// 50, when no `limit` is given). The turn loop needs the full transcript —
    /// otherwise it assembles context from a stale oldest-N window and never
    /// sees recent messages or the latest compaction marker (which lives at the
    /// tail). So we request the max page size and follow `next_cursor` to the
    /// end, mirroring the console transcript reader.
    pub async fn messages(
        &self,
        session_id: &str,
        include_custom: bool,
    ) -> Result<Vec<LoadedEntry>, HarnessError> {
        const PAGE_LIMIT: u64 = 500;
        let mut out: Vec<LoadedEntry> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut payload = json!({
                "session_id": session_id,
                "include_custom": include_custom,
                "limit": PAGE_LIMIT,
            });
            if let Some(c) = &cursor {
                payload["cursor"] = json!(c);
            }
            let resp = self.call("session::messages", payload).await?;
            let arr = resp
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in arr {
                match serde_json::from_value::<LoadedEntry>(item) {
                    Ok(entry) => out.push(entry),
                    Err(e) => {
                        tracing::warn!(session_id, error = %e, "skipping unparseable session entry")
                    }
                }
            }
            match resp.get("next_cursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => cursor = Some(next.to_string()),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Load only the entries strictly after `after_entry_id` on the active
    /// path — the incremental form of [`Self::messages`] for callers holding
    /// a watermark. `Ok(None)` means the watermark left the active path
    /// (fork / `set_active_leaf`); the caller must fall back to a full load.
    pub async fn messages_after(
        &self,
        session_id: &str,
        after_entry_id: &str,
        include_custom: bool,
    ) -> Result<Option<Vec<LoadedEntry>>, HarnessError> {
        const PAGE_LIMIT: u64 = 500;
        let mut out: Vec<LoadedEntry> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut payload = json!({
                "session_id": session_id,
                "include_custom": include_custom,
                "limit": PAGE_LIMIT,
            });
            match &cursor {
                // Later pages resume from the server cursor; only the first
                // page anchors on the caller's watermark.
                Some(c) => payload["cursor"] = json!(c),
                None => payload["after_entry_id"] = json!(after_entry_id),
            }
            let resp = match self.call("session::messages", payload).await {
                Ok(resp) => resp,
                Err(HarnessError::Dependency(msg)) if msg.contains("session/invalid_cursor") => {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            };
            let arr = resp
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in arr {
                match serde_json::from_value::<LoadedEntry>(item) {
                    Ok(entry) => out.push(entry),
                    Err(e) => {
                        tracing::warn!(session_id, error = %e, "skipping unparseable session entry")
                    }
                }
            }
            match resp.get("next_cursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => cursor = Some(next.to_string()),
                _ => break,
            }
        }
        Ok(Some(out))
    }
}

/// The `session::append` request for a bookkeeping entry. The record MUST ride
/// the dedicated `custom` field: `session::append` only creates a
/// `kind: "custom"` entry from it — a `role: "custom"` message wrapper is
/// stored as a plain `kind: "message"` entry, which `session::messages`
/// returns with `custom: None`, so the record would never be found on
/// read-back (the compaction round-trip depends on this).
fn custom_append_payload(
    session_id: &str,
    custom_type: &str,
    data: Value,
    entry_id: &str,
    origin: Option<&Value>,
) -> Value {
    let mut payload = json!({
        "session_id": session_id,
        "entry_id": entry_id,
        "custom": {
            "custom_type": custom_type,
            "data": data,
        },
    });
    if let Some(o) = origin {
        payload["origin"] = o.clone();
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    // Prevents: the compaction record silently persisting as a plain message
    // entry — `session::append` only creates a `kind: "custom"` entry from
    // the `custom` field, and the turn loop only reads the record back from
    // `entry.custom` (a message-wrapped record makes every step reassemble
    // the full transcript and recompact from scratch).
    #[test]
    fn custom_append_rides_the_custom_field() {
        let origin = json!({ "turn_id": "t_1" });
        let payload = custom_append_payload(
            "s_1",
            "compaction",
            json!({ "summary": "s", "tail_start_entry_id": "e_9" }),
            "e_compaction_1",
            Some(&origin),
        );
        assert_eq!(payload["custom"]["custom_type"], "compaction");
        assert_eq!(payload["custom"]["data"]["summary"], "s");
        assert_eq!(payload["entry_id"], "e_compaction_1");
        assert_eq!(payload["origin"], origin);
        assert!(
            payload.get("message").is_none(),
            "custom records must not be message-wrapped (stored as kind: message, \
             returned with custom: None, invisible to the read-back)"
        );
    }
}
