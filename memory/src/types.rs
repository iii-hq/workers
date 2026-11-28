//! Core record shapes shared by the store, the functions, and the events.

use std::sync::LazyLock;

use regex::Regex;
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
    /// Saved explicitly through `memory::save`: by a human (console, REST,
    /// CLI), or by an agent from a turn: then `source.turn_id` is set and it
    /// is that agent's note, not a fact the user stated.
    Stated,
}

/// Where a memory came from. Every extracted memory points back into
/// session-manager; nothing in memory duplicates the transcript store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The turn an explicit save was called from (`memory::save` inside a turn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// The last entry of the run the memory was extracted from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// The first entry of that run: with `entry_id` it bounds the span of
    /// transcript the memory came from. Absent on records written before
    /// spans were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Set on a record written by `memory::correct`: why the old text was
    /// replaced, quoting the source conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_reason: Option<String>,
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

    /// A note an agent saved from inside a turn: its own words, not a fact
    /// the user stated or extraction read.
    pub fn is_agent_note(&self) -> bool {
        self.confidence == Confidence::Stated
            && self.source.as_ref().is_some_and(|s| s.turn_id.is_some())
    }

    /// When the fact was last stated, even when its record was written later
    /// (a late extraction pass); a legacy record goes by its last update.
    pub fn last_stated(&self) -> u64 {
        self.stated_at.unwrap_or(self.updated_at)
    }

    /// A live, unpinned memory tagged [`TRANSIENT_TAG`], or naming a work
    /// item ([`names_work_item`]), whose fact was last
    /// stated over [`TRANSIENT_MAX_AGE_DAYS`] before `now`: a turn is no
    /// longer given it. Hidden, not retired: list and get still return it.
    pub fn expired(&self, now: u64) -> bool {
        self.is_live()
            && !self.pinned
            && (self.tags.iter().any(|t| t == TRANSIENT_TAG) || names_work_item(&self.text))
            && now.saturating_sub(self.last_stated()) > TRANSIENT_MAX_AGE_DAYS * 86_400_000
    }
}

/// The tag that marks a fact that stops being true when the current work
/// ends (a time-boxed status); a person or an agent sets it.
pub const TRANSIENT_TAG: &str = "transient";

/// Whether a text names a work item, whose state ends with the work: a ticket
/// key (`KAN-158`), a pull request (`PR 12`, `#1895`) or a commit hash.
// shortcut: a denylist keeps standards and zones (UTF-8, UTC-3) apart from
// ticket keys, and `#` takes 2-5 digits so colours pass; a project key in the
// list is missed, and a 3-digit colour ("#333") reads as a pull request.
pub fn names_work_item(text: &str) -> bool {
    static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([A-Z]{2,10})-\d+\b").unwrap());
    static PR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:pr|pull request)\s*#?\d+\b|#\d{2,5}\b").unwrap());
    static HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[0-9a-f]{7,40}\b").unwrap());
    const STANDARDS: [&str; 34] = [
        "ADR", "AES", "BR", "COVID", "CRC", "CVE", "CWE", "ECMA", "EIP", "ERC", "ES", "FIPS",
        "GHSA", "GLM", "GMT", "GPT", "HTTP", "IE", "IEEE", "IPV", "ISO", "MD", "PEP", "PG", "RFC",
        "RSA", "RUSTSEC", "SHA", "SQL", "SSL", "TLS", "UTC", "UTF", "WCAG",
    ];
    let bytes = text.as_bytes();
    KEY.captures_iter(text).any(|c| !STANDARDS.contains(&&c[1]))
        || PR.is_match(text)
        || HASH.find_iter(text).any(|m| {
            let h = m.as_str();
            // A colour (`#1e1e1e`) or a UUID's group is no commit.
            let before = m.start().checked_sub(1).map(|i| bytes[i]);
            let after = bytes.get(m.end()).copied();
            !matches!(before, Some(b'#' | b'-'))
                && after != Some(b'-')
                && h.bytes().any(|b| b.is_ascii_digit())
                && h.bytes().any(|b| b.is_ascii_alphabetic())
        })
}
// shortcut: a fixed 7 days, make it a config field if users need to tune it.
pub const TRANSIENT_MAX_AGE_DAYS: u64 = 7;

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
    fn a_ticket_pull_request_or_commit_names_a_work_item() {
        for named in [
            "A rodada 14 do KAN-158 teve veredito reprovado.",
            "MOT-5934 is merged.",
            "O PR #1895 foi aberto sem merge.",
            "PR 12 made refunds retry three times.",
            "The fix landed in #1944.",
            "Verified at commit fffd3d0a63482a142a9f3671ac7db12c7aba2e6d.",
            "The stack runs the build of aa67ce3.",
        ] {
            assert!(names_work_item(named), "{named}");
        }
        for durable in [
            "Files are UTF-8 and hashed with SHA-256 (ISO-8601 dates).",
            "The tests need Docker running.",
            "Use pnpm --filter <package> add <dep>.",
            "Port 49334 is the engine; deadbeef is not a hash without a digit.",
            "Colour #fff, step #1, HTTP-2 and CVE-2024-1234 are not work items.",
            "O usuário mora em São Paulo (UTC-3), fuso GMT-3.",
            "Body text is #333333 on #1e1e1eff; keys are RSA-4096, IEEE-754, PEP-8.",
            "The user prefers GLM-4.6, DeepSeek-R1-0528 and Kimi-K2-0905.",
            "Session b036d262-5ee0-424e-a78d-a9647f9e1c4d is the default one.",
        ] {
            assert!(!names_work_item(durable), "{durable}");
        }
    }

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
                ..Provenance::default()
            }),
            created_at: 1,
            updated_at: 2,
            invalid_at: None,
            superseded_by: None,
            revision: 3,
            stated_at: Some(2),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(
            !s.contains("turn_id"),
            "records without a turn stay as they were"
        );
        let back: Memory = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
        let from_turn = Memory {
            source: Some(Provenance {
                session_id: Some("s_1".into()),
                turn_id: Some("t_1".into()),
                ..Provenance::default()
            }),
            ..f
        };
        let s = serde_json::to_string(&from_turn).unwrap();
        let back: Memory = serde_json::from_str(&s).unwrap();
        assert_eq!(from_turn, back);
    }

    #[test]
    fn an_agent_note_is_a_stated_save_with_a_turn() {
        let with = |confidence, turn_id: Option<&str>| Memory {
            id: fingerprint("x"),
            text: "x".into(),
            entities: vec![],
            tags: vec![],
            confidence,
            corroboration: 0,
            pinned: false,
            source: Some(Provenance {
                session_id: Some("s_1".into()),
                turn_id: turn_id.map(str::to_string),
                ..Provenance::default()
            }),
            created_at: 1,
            updated_at: 1,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: Some(1),
        };
        assert!(with(Confidence::Stated, Some("t_1")).is_agent_note());
        assert!(
            !with(Confidence::Stated, None).is_agent_note(),
            "a save on behalf of a session, outside a turn"
        );
        assert!(!with(Confidence::Extracted, Some("t_1")).is_agent_note());
        let human = Memory {
            source: None,
            ..with(Confidence::Stated, None)
        };
        assert!(!human.is_agent_note());
    }
}
