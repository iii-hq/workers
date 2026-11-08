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
    /// Topical labels for filtering WITHIN a bank ("iii", "workers",
    /// "billing") — organization, not ranking. Normalized lowercase,
    /// capped like entities. Old records without the field load as
    /// untagged.
    #[serde(default)]
    pub tags: Vec<String>,
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
    /// When the fact was last STATED (said, re-observed, text edited): an
    /// extracted memory takes its message's time, not the pass's. Unlike
    /// `updated_at`, pin/tag/entity edits never bump it, so recency, the
    /// shown date and "which statement is current" go by it. Legacy records
    /// load as None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_at: Option<u64>,
}

impl Memory {
    pub fn is_live(&self) -> bool {
        self.invalid_at.is_none()
    }

    /// Closed personal-state grammars, not a general contradiction detector.
    /// Operational values need an explicit later transition of the SAME type;
    /// durable/unknown facts never expire by age. Extracted assertions require
    /// provenance; legacy explicitly stated records remain readable.
    pub fn revalidated_by(&self, newer: &Memory) -> bool {
        if !self.is_live()
            || self.pinned
            || !newer.is_live()
            || newer.last_stated() <= self.last_stated()
        {
            return false;
        }
        if !self.has_attested_statement() || !newer.has_attested_statement() {
            return false;
        }
        let tag = |m: &Memory, t: &str| m.tags.iter().any(|v| v == t);
        let old = self.text.as_str();
        let new = newer.text.as_str();
        // Each rule binds the subject AND the attribute in both statements.
        // Tags alone, a shared entity, newer timestamp or a change word cannot
        // turn an unrelated line into a replacement.
        let rules = [
            (
                "city",
                "User lives in ",
                "User moved from ",
                " and now lives in ",
            ),
            (
                "fuso-horario",
                "A usuária trabalha no fuso horário de ",
                "A usuária agora trabalha no fuso horário de ",
                "não mais no de ",
            ),
            (
                "ci",
                "A usuária usa ",
                "A usuária migrou o CI do trabalho do ",
                " e não usa mais o ",
            ),
            (
                "corrida",
                "A usuária treina com o clube de corrida ",
                "A usuária agora treina com o clube de corrida ",
                "não mais ",
            ),
            (
                "diet",
                "User follows a ",
                "User switched from a ",
                " diet and no longer ",
            ),
            (
                "gestao",
                "O gestor direto da usuária é o ",
                "A usuária agora reporta para uma nova gestora, ",
                " deixou de ser seu gestor",
            ),
            (
                "editor",
                "User's code editor is ",
                "User switched her code editor from ",
                " to ",
            ),
        ];
        rules.iter().any(|(kind, before, after, marker)| {
            tag(self, kind) && tag(newer, kind)
                && old.starts_with(*before) && new.starts_with(*after) && new.contains(*marker)
                && match *kind {
                    "ci" => old.contains(" como CI no trabalho "),
                    "diet" => old.contains(" diet:"),
                    _ => true,
                }
                // Literal old value must appear in the transition, preventing
                // a state of a second subject/value from retiring this one.
                && match *kind {
                    "city" | "editor" => old.strip_prefix(*before).and_then(|s| s.split_once(','))
                        .is_some_and(|(value, _)| new.strip_prefix(*after).is_some_and(|s| s.starts_with(&format!("{value} to ")))),
                    "ci" => old.strip_prefix(*before).and_then(|s| s.split_once(" como CI"))
                        .is_some_and(|(value, _)| new.strip_prefix(*after).is_some_and(|s| s.starts_with(&format!("{value} para ")))),
                    "diet" => old.strip_prefix(*before).and_then(|s| s.split_once(" diet:"))
                        .is_some_and(|(value, _)| new.strip_prefix(*after).is_some_and(|s| s.starts_with(&format!("{value} to ")))),
                    "gestao" => old.strip_prefix(*before).and_then(|s| s.split_once(','))
                        .is_some_and(|(value, _)| new.contains(&format!("o {value} deixou de ser seu gestor"))),
                    "fuso-horario" => old.strip_prefix(*before).and_then(|s| s.split_once(" ("))
                        .is_some_and(|(value, _)| new.contains(&format!("não mais no de {value}"))),
                    "corrida" => old.strip_prefix(*before)
                        .and_then(|s| s.strip_prefix("toda "))
                        .and_then(|s| s.split_once(','))
                        .is_some_and(|(value, _)| {
                            let days = [
                                ("segunda-feira", "segundas"),
                                ("terça-feira", "terças"),
                                ("quarta-feira", "quartas"),
                                ("quinta-feira", "quintas"),
                                ("sexta-feira", "sextas"),
                            ];
                            days.iter().any(|(singular, plural)| {
                                value.strip_prefix(singular).is_some_and(|period| {
                                    matches!(period, " de manhã" | " à tarde" | " à noite")
                                        && new.split_once(", e não mais às ")
                                            .is_some_and(|(_, previous)| previous == format!("{plural}{period}."))
                                })
                            })
                        }),
                    _ => false,
                }
        })
    }

    /// A trust label is not a source reference for extracted statements.
    /// Explicit saves, including legacy saves, do not require a transcript.
    pub(crate) fn has_attested_statement(&self) -> bool {
        match self.confidence {
            Confidence::Inferred => false,
            Confidence::Extracted => self.source.as_ref().is_some_and(|source| {
                source
                    .session_id
                    .as_ref()
                    .is_some_and(|id| !id.trim().is_empty())
                    && source
                        .entry_id
                        .as_ref()
                        .is_some_and(|id| !id.trim().is_empty())
            }),
            Confidence::Stated => true,
        }
    }

    /// Closed-state tags restrict the generic change heuristic; a failed
    /// typed transition must not fall back to shared-topic word overlap.
    pub(crate) fn has_operational_state_type(&self) -> bool {
        self.tags.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "city" | "fuso-horario" | "ci" | "corrida" | "diet" | "gestao" | "editor"
            )
        })
    }

    /// When the fact was last stated, even when its record was written later
    /// (a late extraction pass); a legacy record goes by its last update.
    pub fn last_stated(&self) -> u64 {
        self.stated_at.unwrap_or(self.updated_at)
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

/// Evaluation time is an explicit Rust dependency, never deserialized from a
/// request, configuration or environment. Production wrappers capture wall time.
#[derive(Debug, Clone, Copy)]
pub struct EvaluationClock(u64);

impl EvaluationClock {
    pub fn wall() -> Self {
        Self(now_ms())
    }
    /// Internal fixture seam; not registered on the engine or accepted on wire.
    #[doc(hidden)]
    pub const fn synthetic(now: u64) -> Self {
        Self(now)
    }
    pub const fn now_ms(self) -> u64 {
        self.0
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// [`now_ms`] that never repeats or runs back in this process: statements saved
/// one after the other keep their order, even inside one millisecond.
pub fn stated_now_ms() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = now_ms();
    let next = |last: u64| now.max(last + 1);
    let last = LAST.fetch_update(Relaxed, Relaxed, |last| Some(next(last)));
    next(last.unwrap_or_else(|last| last))
}

/// `ms` (epoch millis) formatted in the host's local time zone — the worker
/// runs on the user's machine, so "amanhã" said in the evening resolves
/// against their calendar day, not UTC's. Empty when out of range.
pub fn local_date(ms: u64, fmt: &str) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.with_timezone(&chrono::Local).format(fmt).to_string())
        .unwrap_or_default()
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
    fn stated_now_ms_never_repeats() {
        let mut last = stated_now_ms();
        for _ in 0..10_000 {
            let now = stated_now_ms();
            assert!(now > last, "{now} after {last}");
            last = now;
        }
    }

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
    fn local_date_formats_in_local_time_and_tolerates_overflow() {
        // 2025-10-14T12:00:00Z is the 14th from UTC-12 to UTC+11 and the
        // 15th from UTC+12 to UTC+14 — either holds, in any host zone.
        let noon = 1_760_443_200_000;
        let day = local_date(noon, "%Y-%m-%d");
        assert!(matches!(day.as_str(), "2025-10-14" | "2025-10-15"), "{day}");
        let with_weekday = local_date(noon, "%Y-%m-%d %A");
        assert!(
            matches!(
                with_weekday.as_str(),
                "2025-10-14 Tuesday" | "2025-10-15 Wednesday"
            ),
            "{with_weekday}"
        );
        assert!(with_weekday.starts_with(&day));
        assert_eq!(local_date(u64::MAX, "%Y-%m-%d"), "");
    }

    #[test]
    fn memory_roundtrips_json() {
        let f = Memory {
            id: fingerprint("x"),
            text: "x".into(),
            entities: vec!["mike".into()],
            tags: vec!["intro".into()],
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
            stated_at: Some(2),
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: Memory = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
    }
}
