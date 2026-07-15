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
    let (memory, event, created) = match bank.get(&id).await {
        Some(existing) => {
            // Same fingerprint: reinforce a live record, RESURRECT a
            // tombstoned/superseded one. Building a fresh revision-0 record
            // here would roll the revision counter back, and last-wins
            // replay would silently restore the tombstone on the next boot.
            let live = existing.is_live();
            let mut f = existing;
            if !live {
                f.invalid_at = None;
                f.superseded_by = None;
            }
            f.corroboration = f.corroboration.saturating_add(1);
            f.pinned = f.pinned || req.pinned;
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
                entities: req
                    .entities
                    .into_iter()
                    .map(|e| e.trim().to_lowercase())
                    .filter(|e| !e.is_empty())
                    .take(8)
                    .collect(),
                confidence: Confidence::Stated,
                corroboration: 0,
                pinned: req.pinned,
                source: req.session_id.map(|session_id| Provenance {
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
    };
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
        memory.entities = entities
            .into_iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .take(8)
            .collect();
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
            pinned: Some(req.pinned),
        },
    )
    .await
}
