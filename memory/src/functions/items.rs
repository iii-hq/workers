//! `memory::save/get/list/update/delete/pin` — memory CRUD. Every mutation
//! goes through the bank's commit choke point and emits an item event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::events::ItemEvent;
use crate::types::{fingerprint, now_ms, Confidence, Memory, Provenance};

/// Normalized entity handles: trimmed, lowercased, empty dropped, capped.
fn normalize_entities(entities: Vec<String>) -> Vec<String> {
    entities
        .into_iter()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .take(8)
        .collect()
}

/// Inputs for one save transition (grouped: clippy arity cap, and the
/// call sites read better named).
struct SaveInput {
    id: String,
    text: String,
    entities: Vec<String>,
    tags: Vec<String>,
    pinned: bool,
    session_id: Option<String>,
    now: u64,
}

/// The save state transition, pure so its invariants are testable:
/// - same fingerprint + live: reinforce (corroboration + 1, revision + 1);
/// - same fingerprint + retired: RESURRECT — clear the tombstone and
///   CONTINUE the revision counter (a fresh revision-0 record would lose
///   to the tombstone under last-wins replay on the next boot);
/// - unknown fingerprint: a fresh record.
fn save_transition(existing: Option<Memory>, input: SaveInput) -> (Memory, ItemEvent, bool) {
    let SaveInput {
        id,
        text,
        entities,
        tags,
        pinned,
        session_id,
        now,
    } = input;
    match existing {
        Some(existing) => {
            let live = existing.is_live();
            let mut f = existing;
            if !live {
                f.invalid_at = None;
                f.superseded_by = None;
            }
            // A re-observation can add labels; it never removes them.
            for tag in normalize_entities(tags) {
                if !f.tags.contains(&tag) && f.tags.len() < 8 {
                    f.tags.push(tag);
                }
            }
            f.corroboration = f.corroboration.saturating_add(1);
            f.pinned = f.pinned || pinned;
            f.updated_at = now;
            f.stated_at = Some(now);
            f.revision += 1;
            if live {
                (f, ItemEvent::Updated, false)
            } else {
                (f, ItemEvent::Created, true)
            }
        }
        None => (
            Memory {
                id,
                text,
                entities: normalize_entities(entities),
                tags: normalize_entities(tags),
                confidence: Confidence::Stated,
                corroboration: 0,
                pinned,
                source: session_id.map(|session_id| Provenance {
                    session_id: Some(session_id),
                    entry_id: None,
                    agent: None,
                }),
                created_at: now,
                updated_at: now,
                invalid_at: None,
                superseded_by: None,
                revision: 0,
                stated_at: Some(now),
            },
            ItemEvent::Created,
            true,
        ),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveRequest {
    /// Target bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// The memory, one self-contained sentence.
    pub text: String,
    /// Entity handles (people/projects/tools) used as a retrieval signal.
    #[serde(default)]
    pub entities: Vec<String>,
    /// Topical labels for filtering within the bank (organization, not
    /// ranking).
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Provenance session, when saved on behalf of a conversation.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveResponse {
    pub memory: Memory,
    /// False when the text fingerprint matched an existing memory (it was
    /// reinforced instead).
    pub created: bool,
}

pub async fn save(deps: &Deps, req: SaveRequest) -> Result<SaveResponse, MemoryError> {
    let text = req.text.trim().to_string();
    if text.len() < 3 || text.len() > 2_000 {
        return Err(MemoryError::InvalidInput(
            "text must be 3..2000 characters".into(),
        ));
    }
    // Redact before the fingerprint: dedup and the stored text agree, and
    // the caller sees what was kept in the returned record.
    let redaction = crate::redact::redact(&text);
    let text = redaction.text;
    // Short fields too, before normalization lowercases them (a lowercased
    // key still leaks its material, and the key shapes are case-sensitive).
    let (mut entities, mut tags) = (req.entities, req.tags);
    let mut labels = redaction.labels;
    labels.extend(crate::redact::drop_secrets(&mut entities));
    labels.extend(crate::redact::drop_secrets(&mut tags));
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let (bank, bank_created) = store.ensure_bank(&bank_name, None).await?;
    if bank_created {
        deps.emitter.bank("created", &bank_name).await;
    }

    let id = fingerprint(&text);
    let now = now_ms();
    let existing = bank.get(&id).await;
    let (memory, event, created) = save_transition(
        existing,
        SaveInput {
            id,
            text,
            entities,
            tags,
            pinned: req.pinned,
            session_id: req.session_id,
            now,
        },
    );
    bank.commit(memory.clone()).await?;
    let reason = crate::redact::reason(&labels);
    deps.emitter
        .item_with(event, &bank_name, &memory, reason.as_deref())
        .await;
    crate::embed_client::spawn_backfill(deps, &bank_name);
    Ok(SaveResponse { memory, created })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetResponse {
    pub memory: Memory,
}

pub async fn get(deps: &Deps, req: GetRequest) -> Result<GetResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    Ok(GetResponse { memory })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// Page size, default 50, max 500.
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    /// Include superseded/tombstoned records (the history view).
    #[serde(default)]
    pub include_superseded: Option<bool>,
    /// Only memories carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub memories: Vec<Memory>,
    pub total: usize,
}

pub async fn list(deps: &Deps, req: ListRequest) -> Result<ListResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let limit = req.limit.unwrap_or(50).min(500);
    let (memories, total) = bank
        .list(
            limit,
            req.offset.unwrap_or(0),
            req.include_superseded.unwrap_or(false),
            req.tag.as_deref(),
        )
        .await;
    Ok(ListResponse { memories, total })
}

/// Shared response for update/delete/pin.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MemoryResponse {
    pub memory: Memory,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub entities: Option<Vec<String>>,
    /// Replace the tag set (empty list clears).
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub pinned: Option<bool>,
}

pub async fn update(deps: &Deps, req: UpdateRequest) -> Result<MemoryResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let mut memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    let mut text_changed = false;
    let mut labels: Vec<&'static str> = Vec::new();
    if let Some(text) = req.text {
        let text = text.trim().to_string();
        if text.len() < 3 || text.len() > 2_000 {
            return Err(MemoryError::InvalidInput(
                "text must be 3..2000 characters".into(),
            ));
        }
        let redaction = crate::redact::redact(&text);
        labels.extend(redaction.labels);
        let text = redaction.text;
        // A text edit restates the fact; pin/tag/entity edits do not.
        text_changed = memory.text != text;
        if text_changed {
            memory.stated_at = Some(now_ms());
        }
        memory.text = text;
    }
    // Redacted before normalization lowercases them (see `save`).
    if let Some(mut entities) = req.entities {
        labels.extend(crate::redact::drop_secrets(&mut entities));
        memory.entities = normalize_entities(entities);
    }
    if let Some(mut tags) = req.tags {
        labels.extend(crate::redact::drop_secrets(&mut tags));
        memory.tags = normalize_entities(tags);
    }
    if let Some(pinned) = req.pinned {
        memory.pinned = pinned;
    }
    memory.updated_at = now_ms();
    memory.revision += 1;
    bank.commit(memory.clone()).await?;
    let reason = crate::redact::reason(&labels);
    deps.emitter
        .item_with(ItemEvent::Updated, &bank_name, &memory, reason.as_deref())
        .await;
    // The commit dropped the vector of the old text; embed the new one.
    if text_changed {
        crate::embed_client::spawn_backfill(deps, &bank_name);
    }
    Ok(MemoryResponse { memory })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

pub async fn delete(deps: &Deps, req: DeleteRequest) -> Result<MemoryResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let mut memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    if memory.invalid_at.is_none() {
        memory.invalid_at = Some(now_ms());
        memory.updated_at = now_ms();
        memory.revision += 1;
        bank.commit(memory.clone()).await?;
        deps.emitter
            .item(ItemEvent::Deleted, &bank_name, &memory)
            .await;
    }
    Ok(MemoryResponse { memory })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PinRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
    pub pinned: bool,
}

pub async fn pin(deps: &Deps, req: PinRequest) -> Result<MemoryResponse, MemoryError> {
    update(
        deps,
        UpdateRequest {
            bank: req.bank,
            id: req.id,
            text: None,
            entities: None,
            tags: None,
            pinned: Some(req.pinned),
        },
    )
    .await
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SupersedeRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
    /// Memory to retire.
    pub id: String,
    /// Live memory that replaces it.
    pub superseded_by: String,
    /// Merge semantics: the survivor absorbs the retired record's
    /// observations (corroboration += retired + 1) in the same call. A
    /// pinned survivor is never written. Default false (a state change:
    /// the new record replaces the old, nothing to absorb).
    #[serde(default)]
    pub reinforce: bool,
    /// Why — carried on the `memory::item-changed` events this call emits
    /// (not persisted on the record). Trimmed, capped at 500 chars.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SupersedeResponse {
    /// The retired record.
    pub memory: Memory,
    /// True when the survivor was reinforced (`reinforce` asked and the
    /// survivor is not pinned).
    pub reinforced: bool,
}

const MAX_REASON_CHARS: usize = 500;

fn clamp_reason(reason: Option<String>) -> Option<String> {
    let reason: String = reason?.trim().chars().take(MAX_REASON_CHARS).collect();
    (!reason.is_empty()).then_some(reason)
}

/// The two records a supersede writes: the retired loser and, when
/// reinforcing a non-pinned survivor, the survivor absorbing the loser's
/// observations — `(s+1) + (r+1)` total, so corroboration `s + r + 1`,
/// stamped with the LATEST of the two `updated_at` (a merge is not a fresh
/// restatement; Hindsight keeps the latest source `mentioned_at`).
fn supersede_transition(
    mut loser: Memory,
    mut winner: Memory,
    reinforce: bool,
    now: u64,
) -> (Memory, Option<Memory>) {
    let loser_updated_at = loser.updated_at;
    let loser_corroboration = loser.corroboration;
    let loser_stated_at = loser.stated_at;
    loser.invalid_at = Some(now);
    loser.superseded_by = Some(winner.id.clone());
    loser.updated_at = now;
    loser.revision += 1;
    if !reinforce || winner.pinned {
        return (loser, None);
    }
    winner.corroboration = winner
        .corroboration
        .saturating_add(loser_corroboration)
        .saturating_add(1);
    winner.updated_at = winner.updated_at.max(loser_updated_at);
    winner.stated_at = winner.stated_at.max(loser_stated_at);
    winner.revision += 1;
    (loser, Some(winner))
}

/// Retire one memory in favor of another: tombstone with a pointer, never a
/// plain delete. The consolidation seam — siblings merge duplicates through
/// this instead of touching files. Pinned memories cannot be superseded.
pub async fn supersede(
    deps: &Deps,
    req: SupersedeRequest,
) -> Result<SupersedeResponse, MemoryError> {
    if req.id == req.superseded_by {
        return Err(MemoryError::InvalidInput(
            "a memory cannot supersede itself".into(),
        ));
    }
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let winner = bank
        .get(&req.superseded_by)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.superseded_by.clone()))?;
    if !winner.is_live() {
        return Err(MemoryError::InvalidInput(format!(
            "superseded_by `{}` is not a live memory",
            req.superseded_by
        )));
    }
    let memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    if memory.pinned {
        return Err(MemoryError::InvalidInput(format!(
            "memory `{}` is pinned; pinned memories are untouchable by automatic paths",
            req.id
        )));
    }
    // An already-retired target is an error, not a silent success:
    // concurrent consolidation passes would otherwise count the same
    // retirement twice and double-reinforce the survivor.
    if !memory.is_live() {
        return Err(MemoryError::InvalidInput(format!(
            "memory `{}` is already retired (superseded_by: {:?})",
            req.id, memory.superseded_by
        )));
    }
    let reason = clamp_reason(req.reason);
    let (retired, survivor) = supersede_transition(memory, winner, req.reinforce, now_ms());
    // Loser first: a retry after a failed survivor write then fails as
    // "already retired" instead of double-reinforcing.
    bank.commit(retired.clone()).await?;
    deps.emitter
        .item_with(
            ItemEvent::Superseded,
            &bank_name,
            &retired,
            reason.as_deref(),
        )
        .await;
    let reinforced = survivor.is_some();
    if let Some(survivor) = survivor {
        bank.commit(survivor.clone()).await?;
        deps.emitter
            .item_with(ItemEvent::Updated, &bank_name, &survivor, reason.as_deref())
            .await;
    }
    Ok(SupersedeResponse {
        memory: retired,
        reinforced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, revision: u64, live: bool, pinned: bool) -> Memory {
        Memory {
            id: id.into(),
            text: "some text".into(),
            entities: vec![],
            tags: vec![],
            confidence: Confidence::Stated,
            corroboration: 2,
            pinned,
            source: None,
            created_at: 10,
            updated_at: 10,
            invalid_at: if live { None } else { Some(20) },
            superseded_by: if live { None } else { Some("fpw".into()) },
            revision,
            stated_at: None,
        }
    }

    #[test]
    fn fresh_save_starts_at_revision_zero_with_normalized_entities() {
        let (m, event, created) = save_transition(
            None,
            SaveInput {
                id: "fp1".into(),
                text: "text".into(),
                entities: vec![" Mike ".into(), "".into(), "BLOG".into()],
                tags: vec![" Intro ".into()],
                pinned: true,
                session_id: Some("s_1".into()),
                now: 99,
            },
        );
        assert!(created);
        assert!(matches!(event, ItemEvent::Created));
        assert_eq!(m.revision, 0);
        assert_eq!(m.stated_at, Some(99), "a new record is stated now");
        assert_eq!(m.entities, vec!["mike", "blog"]);
        assert_eq!(m.tags, vec!["intro"]);
        assert!(m.pinned);
        assert_eq!(m.source.unwrap().session_id.as_deref(), Some("s_1"));
    }

    #[test]
    fn entity_cap_holds_at_eight() {
        let many: Vec<String> = (0..12).map(|i| format!("e{i}")).collect();
        let (m, _, _) = save_transition(
            None,
            SaveInput {
                id: "fp".into(),
                text: "t".into(),
                entities: many,
                tags: vec![],
                pinned: false,
                session_id: None,
                now: 1,
            },
        );
        assert_eq!(m.entities.len(), 8);
    }

    #[test]
    fn live_resave_reinforces_and_never_unpins() {
        let (m, event, created) = save_transition(
            Some(record("fp1", 4, true, true)),
            SaveInput {
                id: "fp1".into(),
                text: "some text".into(),
                entities: vec![],
                tags: vec!["fresh-tag".into()],
                pinned: false,
                session_id: None,
                now: 99,
            },
        );
        assert!(!created);
        assert!(matches!(event, ItemEvent::Updated));
        assert_eq!(m.revision, 5);
        assert_eq!(m.corroboration, 3);
        assert!(m.pinned, "a resave without pinned must not unpin");
        assert_eq!(m.tags, vec!["fresh-tag"], "re-observation adds labels");
        assert_eq!(m.updated_at, 99);
    }

    #[test]
    fn retired_resave_resurrects_and_continues_the_revision_counter() {
        let (m, event, created) = save_transition(
            Some(record("fp1", 7, false, false)),
            SaveInput {
                id: "fp1".into(),
                text: "some text".into(),
                entities: vec![],
                tags: vec![],
                pinned: false,
                session_id: None,
                now: 99,
            },
        );
        assert!(created, "a resurrection re-enters live sets");
        assert_eq!(m.stated_at, Some(99), "re-saving restates the fact");
        assert!(matches!(event, ItemEvent::Created));
        assert_eq!(
            m.revision, 8,
            "revision continues — rollback would lose to the tombstone on replay"
        );
        assert!(m.invalid_at.is_none());
        assert!(m.superseded_by.is_none());
    }

    #[test]
    fn supersede_retires_and_reinforce_carries_the_retired_count() {
        let mut loser = record("fpl", 3, true, false);
        loser.corroboration = 6;
        loser.updated_at = 50;
        loser.stated_at = Some(45);
        let mut winner = record("fpw", 1, true, false);
        winner.corroboration = 0;
        winner.updated_at = 40;
        winner.stated_at = Some(30);
        let (retired, survivor) = supersede_transition(loser, winner.clone(), true, 99);
        assert_eq!(retired.invalid_at, Some(99));
        assert_eq!(retired.superseded_by.as_deref(), Some("fpw"));
        assert_eq!(retired.revision, 4);
        let survivor = survivor.expect("reinforced");
        // (0+1) + (6+1) observations = corroboration 7: the survivor never
        // ends below the record it absorbed.
        assert_eq!(survivor.corroboration, 7);
        // Latest of the two, NOT now: a merge is not a fresh restatement.
        assert_eq!(survivor.updated_at, 50);
        assert_eq!(survivor.stated_at, Some(45), "latest statement of the two");
        assert_eq!(survivor.revision, 2);

        let (_, none) = supersede_transition(record("fpl", 0, true, false), winner, false, 99);
        assert!(none.is_none(), "reinforce off leaves the survivor alone");
    }

    #[test]
    fn supersede_never_writes_a_pinned_survivor() {
        let pinned = record("fpw", 1, true, true);
        let (retired, survivor) =
            supersede_transition(record("fpl", 0, true, false), pinned, true, 99);
        assert!(retired.invalid_at.is_some(), "the loser still retires");
        assert!(
            survivor.is_none(),
            "pinned is untouchable by automatic paths"
        );
    }

    #[test]
    fn reason_is_trimmed_capped_in_chars_and_empty_is_none() {
        assert_eq!(
            clamp_reason(Some("  same fact  ".into())).as_deref(),
            Some("same fact")
        );
        assert_eq!(clamp_reason(Some("   ".into())), None);
        assert_eq!(clamp_reason(None), None);
        let long = "ção".repeat(300); // 900 chars, multibyte
        let clamped = clamp_reason(Some(long)).unwrap();
        assert_eq!(clamped.chars().count(), 500);
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TagsRequest {
    /// Bank; omitted: the calling session's bank, else the configured default.
    #[serde(default)]
    pub bank: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TagsResponse {
    pub bank: String,
    pub tags: Vec<TagCount>,
}

pub async fn tags(deps: &Deps, req: TagsRequest) -> Result<TagsResponse, MemoryError> {
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    Ok(TagsResponse {
        bank: bank_name,
        tags: bank
            .tags()
            .await
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect(),
    })
}
