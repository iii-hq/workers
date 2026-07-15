//! Core record shapes shared by the store, the functions, and the events.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How a memory entered the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Stated directly in the source conversation.
    Extracted,
    /// Derived by the extraction model rather than stated verbatim.
    Inferred,
    /// Saved explicitly by an agent or a human (highest trust).
    Stated,
}

/// Where a memory came from. Every extracted memory points back into
/// session-manager; nothing in memory duplicates the transcript store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// One memory item. Serialized whole as a `memories.jsonl` line; replay is
/// last-wins by `id`, so an update appends a full record with a bumped
/// `revision` and a delete appends a tombstoned record (`invalid_at` set) —
/// history is never destroyed, only superseded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Memory {
    /// Content fingerprint (`fp` + FNV-1a of the normalized text), so
    /// re-ingesting the same source reinforces instead of duplicating.
    pub id: String,
    pub text: String,
    /// Entity handles used as an exact-match retrieval signal.
    #[serde(default)]
    pub entities: Vec<String>,
    pub confidence: Confidence,
    /// Times this memory was independently re-observed. Boosts recall rank.
    #[serde(default)]
    pub corroboration: u32,
    /// Pinned memories are never touched by any automatic path (consolidation,
    /// decay); only an explicit `memory::pin`/`memory::update` changes them.
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Provenance>,
    /// Milliseconds since epoch.
    pub created_at: u64,
    pub updated_at: u64,
    /// Set when the memory stopped being true (superseded or deleted). The
    /// record stays on disk and remains queryable with
    /// `include_superseded: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<u64>,
    /// Memory id that replaced this one, when superseded by content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Bumped on every appended revision of this id; replay keeps the
    /// highest.
    #[serde(default)]
    pub revision: u64,
}

impl Memory {
    pub fn is_live(&self) -> bool {
        self.invalid_at.is_none()
    }
}

/// One markdown rule: always injected whole into the system prompt for
/// sessions using its bank. Stored as `rules/<name>.md` — open it in any
/// editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub name: String,
    pub content: String,
    /// Milliseconds since epoch (file mtime).
    pub updated_at: u64,
}

/// Summary row for `memory::bank::list` and the console page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BankInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Live memory (memory) count.
    pub memories: usize,
    pub pinned: usize,
    /// Rule (always-injected markdown) count.
    pub rules: usize,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Stable 64-bit FNV-1a over the normalized text (lowercased, whitespace
/// collapsed). Deliberately hand-rolled: the fingerprint is persisted, so it
/// must never change across toolchains or dependency bumps.
pub fn fingerprint(text: &str) -> String {
    let normalized: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in normalized.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fp{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_normalizing() {
        let a = fingerprint("User prefers  short   names");
        let b = fingerprint("user prefers short names");
        assert_eq!(a, b);
        assert_eq!(a, fingerprint("User prefers  short   names"));
        assert_ne!(a, fingerprint("user prefers long names"));
        assert!(a.starts_with("fp"));
        assert_eq!(a.len(), 18);
    }

    #[test]
    fn memory_roundtrips_json() {
        let f = Memory {
            id: fingerprint("x"),
            text: "x".into(),
            entities: vec!["mike".into()],
            confidence: Confidence::Extracted,
            corroboration: 2,
            pinned: true,
            source: Some(Provenance {
                session_id: Some("s_1".into()),
                entry_id: None,
                agent: None,
            }),
            created_at: 1,
            updated_at: 2,
            invalid_at: None,
            superseded_by: None,
            revision: 3,
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: Memory = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
    }
}
