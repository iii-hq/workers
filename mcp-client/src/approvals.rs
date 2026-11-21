//! Which entry an operator approved, by fingerprint: the vault secret
//! `MCP_CLIENT_APPROVALS`, written only by compare-and-set, and the memory of
//! the exact text this worker last read or wrote, so a write by anyone else
//! is seen as a tamper.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::host::{Host, SecretError};
use crate::refs::ref_error;

pub const APPROVALS_SECRET: &str = "MCP_CLIENT_APPROVALS";
pub const APPROVALS_REF: &str = "secret://MCP_CLIENT_APPROVALS";

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u8,
    approved: BTreeMap<String, String>,
}

fn encode(approved: &BTreeMap<String, String>) -> String {
    let envelope = Envelope {
        version: 1,
        approved: approved.clone(),
    };
    serde_json::to_string(&envelope).unwrap_or_default()
}

/// The empty map for anything that is not a version 1 envelope.
fn decode(text: &str) -> BTreeMap<String, String> {
    serde_json::from_str::<Envelope>(text)
        .ok()
        .filter(|e| e.version == 1)
        .map(|e| e.approved)
        .unwrap_or_default()
}

/// 64 characters of `[0-9a-f]`, as `config::fingerprint` writes them.
pub fn is_fingerprint(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The stored text; `None` when the secret is absent.
async fn read(host: &dyn Host) -> Result<Option<String>, SecretError> {
    match host.resolve(APPROVALS_REF).await {
        Ok(text) => Ok(Some(text)),
        Err(SecretError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

fn error_text(e: SecretError) -> String {
    ref_error(APPROVALS_REF, e).message
}

#[derive(Default)]
struct Memory {
    loaded: bool,
    approved: BTreeMap<String, String>,
    /// The exact text last read or written; `None`: absent.
    text: Option<String>,
    /// An edit's fingerprint, approved before its entry was written.
    pending: HashMap<String, String>,
}

/// Once loaded, this memory is the source of truth: the map changes only
/// through `approve`, `forget` and `on_change`.
#[derive(Default)]
pub struct Approvals {
    /// Held across each whole load, approve, forget and on_change, so the
    /// change event of this worker's own write is judged after it was recorded.
    op: tokio::sync::Mutex<()>,
    memory: Mutex<Memory>,
}

impl Approvals {
    pub fn new() -> Approvals {
        Self::default()
    }

    fn memory(&self) -> MutexGuard<'_, Memory> {
        self.memory.lock().unwrap()
    }

    /// Reads the stored approvals once: a boot is the only moment they are
    /// taken on trust.
    pub async fn load(&self, host: &dyn Host) -> Result<(), SecretError> {
        let _op = self.op.lock().await;
        self.load_held(host).await
    }

    async fn load_held(&self, host: &dyn Host) -> Result<(), SecretError> {
        if self.memory().loaded {
            return Ok(());
        }
        let text = read(host).await?;
        let mut memory = self.memory();
        memory.approved = text.as_deref().map(decode).unwrap_or_default();
        memory.text = text;
        memory.loaded = true;
        Ok(())
    }

    /// False until loaded.
    pub fn is_approved(&self, server: &str, fingerprint: &str) -> bool {
        self.memory()
            .approved
            .get(server)
            .is_some_and(|f| f == fingerprint)
    }

    pub async fn approve(
        &self,
        host: &dyn Host,
        server: &str,
        fingerprint: &str,
    ) -> Result<(), String> {
        let _op = self.op.lock().await;
        // Before the first load, a write would drop the stored approvals.
        self.load_held(host).await.map_err(error_text)?;
        let mut next = self.memory().approved.clone();
        if next.insert(server.into(), fingerprint.into()).as_deref() == Some(fingerprint) {
            // Already the approved one: nothing to write.
            return Ok(());
        }
        self.save(host, next).await
    }

    pub async fn forget(&self, host: &dyn Host, server: &str) -> Result<(), String> {
        let _op = self.op.lock().await;
        self.load_held(host).await.map_err(error_text)?;
        let mut next = self.memory().approved.clone();
        if next.remove(server).is_none() {
            return Ok(());
        }
        self.save(host, next).await
    }

    /// Writes `next` over whatever is stored; memory changes only after a
    /// swap succeeded. The caller holds `op`.
    async fn save(&self, host: &dyn Host, next: BTreeMap<String, String>) -> Result<(), String> {
        let text = encode(&next);
        for _ in 0..8 {
            // Read only to pass as `expected`: the next text comes from memory.
            let expected = read(host).await.map_err(error_text)?;
            let swapped = host
                .compare_and_set(APPROVALS_SECRET, expected.as_deref(), &text)
                .await
                .map_err(|e| format!("saving the approval failed: {}", error_text(e)))?;
            if swapped {
                let mut memory = self.memory();
                memory.approved = next;
                memory.text = Some(text);
                return Ok(());
            }
        }
        Err("approvals changed while saving, try again".into())
    }

    /// Re-reads after a change event. A text other than the one this worker
    /// last read or wrote (or a failed read) unapproves every entry until a
    /// new approve call; `true` then.
    pub async fn on_change(&self, host: &dyn Host) -> bool {
        let _op = self.op.lock().await;
        if !self.memory().loaded {
            // The boot load has yet to happen and will read this text.
            return false;
        }
        let read = read(host).await;
        {
            let mut memory = self.memory();
            match read {
                Ok(text) if text == memory.text => return false,
                // Remembered, so the same foreign text is reported once.
                Ok(text) => memory.text = text,
                Err(_) => {}
            }
            memory.approved.clear();
        }
        tracing::warn!(
            "`{APPROVALS_SECRET}` was changed by someone other than mcp-client: every MCP server is unapproved until an operator approves it again"
        );
        true
    }

    pub fn set_pending(&self, server: &str, fingerprint: &str) {
        self.memory()
            .pending
            .insert(server.into(), fingerprint.into());
    }

    pub fn pending(&self, server: &str) -> Option<String> {
        self.memory().pending.get(server).cloned()
    }

    pub fn clear_pending(&self, server: &str) {
        self.memory().pending.remove(server);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips_and_an_unknown_one_approves_nothing() {
        let fp = "a".repeat(64);
        let approved = BTreeMap::from([("linear".to_string(), fp.clone())]);
        assert_eq!(
            encode(&approved),
            format!(r#"{{"version":1,"approved":{{"linear":"{fp}"}}}}"#)
        );
        assert_eq!(decode(&encode(&approved)), approved);
        assert!(decode(r#"{"version":2,"approved":{"linear":"x"}}"#).is_empty());
        assert!(decode(r#"{"version":1}"#).is_empty());
        assert!(decode("not json").is_empty());
    }

    #[test]
    fn a_fingerprint_is_64_lower_case_hex_characters() {
        assert!(is_fingerprint(&"0123456789abcdef".repeat(4)));
        assert!(
            !is_fingerprint("abc")
                && !is_fingerprint(&"A".repeat(64))
                && !is_fingerprint(&"g".repeat(64))
        );
    }
}
