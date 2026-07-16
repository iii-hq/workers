//! `memory::save/get/list/update/delete/pin` — memory CRUD. Every mutation
//! goes through the bank's commit choke point and emits an item event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::events::ItemEvent;
use crate::types::{fingerprint, now_ms, Confidence, Memory, Provenance};

fn default_bank_name(req_bank: &Option<String>, cfg_default: &str) -> String {
    req_bank.clone().unwrap_or_else(|| cfg_default.to_string())
}

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
            },
            ItemEvent::Created,
            true,
        ),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveRequest {
    /// Target bank; the configured default when omitted.
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
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
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
            entities: req.entities,
            tags: req.tags,
            pinned: req.pinned,
            session_id: req.session_id,
            now,
        },
    );
    bank.commit(memory.clone()).await?;
    deps.emitter.item(event, &bank_name, &memory).await;
    Ok(SaveResponse { memory, created })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetResponse {
    pub memory: Memory,
}

pub async fn get(deps: &Deps, req: GetRequest) -> Result<GetResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
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
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
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
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let mut memory = bank
        .get(&req.id)
        .await
        .ok_or_else(|| MemoryError::MemoryNotFound(req.id.clone()))?;
    if let Some(text) = req.text {
        let text = text.trim().to_string();
        if text.len() < 3 || text.len() > 2_000 {
            return Err(MemoryError::InvalidInput(
                "text must be 3..2000 characters".into(),
            ));
        }
        memory.text = text;
    }
    if let Some(entities) = req.entities {
        memory.entities = normalize_entities(entities);
    }
    if let Some(tags) = req.tags {
        memory.tags = normalize_entities(tags);
    }
    if let Some(pinned) = req.pinned {
        memory.pinned = pinned;
    }
    memory.updated_at = now_ms();
    memory.revision += 1;
    bank.commit(memory.clone()).await?;
    deps.emitter
        .item(ItemEvent::Updated, &bank_name, &memory)
        .await;
    Ok(MemoryResponse { memory })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    #[serde(default)]
    pub bank: Option<String>,
    pub id: String,
}

pub async fn delete(deps: &Deps, req: DeleteRequest) -> Result<MemoryResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
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
    #[serde(default)]
    pub bank: Option<String>,
    /// Memory to retire.
    pub id: String,
    /// Live memory that replaces it.
    pub superseded_by: String,
}

/// Retire one memory in favor of another: tombstone with a pointer, never a
/// plain delete. The consolidation seam — siblings merge duplicates through
/// this instead of touching files. Pinned memories cannot be superseded.
pub async fn supersede(deps: &Deps, req: SupersedeRequest) -> Result<MemoryResponse, MemoryError> {
    if req.id == req.superseded_by {
        return Err(MemoryError::InvalidInput(
            "a memory cannot supersede itself".into(),
        ));
    }
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
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
    let mut memory = bank
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
    memory.invalid_at = Some(now_ms());
    memory.superseded_by = Some(req.superseded_by);
    memory.updated_at = now_ms();
    memory.revision += 1;
    bank.commit(memory.clone()).await?;
    deps.emitter
        .item(ItemEvent::Superseded, &bank_name, &memory)
        .await;
    Ok(MemoryResponse { memory })
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
        assert!(matches!(event, ItemEvent::Created));
        assert_eq!(
            m.revision, 8,
            "revision continues — rollback would lose to the tombstone on replay"
        );
        assert!(m.invalid_at.is_none());
        assert!(m.superseded_by.is_none());
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TagsRequest {
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
    let cfg = deps.config().await;
    let bank_name = default_bank_name(&req.bank, &cfg.default_bank);
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
