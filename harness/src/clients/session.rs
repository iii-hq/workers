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
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| HarnessError::Dependency(format!("{function_id}: {e}")))
    }

    /// Idempotently ensure a session exists, applying `metadata` on creation.
    pub async fn ensure(
        &self,
        session_id: &str,
        title: Option<&str>,
        metadata: Option<&Value>,
    ) -> Result<(), HarnessError> {
        let mut payload = json!({ "session_id": session_id });
        if let Some(t) = title {
            payload["title"] = json!(t);
        }
        if let Some(m) = metadata {
            payload["metadata"] = m.clone();
        }
        self.call("session::ensure", payload).await.map(|_| ())
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
        // session-manager stores custom entries via a `custom` message wrapper
        // with no model wire mapping; the harness reads them back with
        // include_custom.
        let mut payload = json!({
            "session_id": session_id,
            "entry_id": entry_id,
            "message": {
                "role": "custom",
                "custom_type": custom_type,
                "content": [],
                "details": data,
                "timestamp": AgentMessage::now_ms(),
            },
        });
        if let Some(o) = origin {
            payload["origin"] = o.clone();
        }
        self.call("session::append", payload).await.map(|_| ())
    }

    /// Replace a message entry's content (streaming + final write). Returns
    /// the new revision.
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
        let resp = self.call("session::update-message", payload).await?;
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
}
