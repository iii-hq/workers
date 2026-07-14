//! `memory::bank::*` — banks as first-class named objects.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::types::BankInfo;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BankCreateRequest {
    /// Bank name, `[a-z0-9][a-z0-9_-]{0,63}` (it becomes a folder name).
    pub name: String,
    /// Human description shown in listings and the console.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BankCreateResponse {
    pub bank: BankInfo,
    /// False when the bank already existed (create is idempotent).
    pub created: bool,
}

pub async fn create(
    deps: &Deps,
    req: BankCreateRequest,
) -> Result<BankCreateResponse, MemoryError> {
    let store = deps.store().await;
    let (bank, created) = store
        .ensure_bank(&req.name, req.description.as_deref())
        .await?;
    if created {
        deps.emitter.bank("created", &req.name).await;
    }
    let (facts, pinned) = bank.counts().await;
    let blocks = bank.list_blocks().map(|b| b.len()).unwrap_or(0);
    Ok(BankCreateResponse {
        bank: BankInfo {
            name: bank.name.clone(),
            description: bank.description.clone(),
            facts,
            pinned,
            blocks,
        },
        created,
    })
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BankListRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BankListResponse {
    pub banks: Vec<BankInfo>,
}

pub async fn list(deps: &Deps, _req: BankListRequest) -> Result<BankListResponse, MemoryError> {
    let store = deps.store().await;
    let mut banks = Vec::new();
    for name in store.bank_names().await {
        let Ok(bank) = store.bank(&name).await else {
            continue;
        };
        let (facts, pinned) = bank.counts().await;
        let blocks = bank.list_blocks().map(|b| b.len()).unwrap_or(0);
        banks.push(BankInfo {
            name: bank.name.clone(),
            description: bank.description.clone(),
            facts,
            pinned,
            blocks,
        });
    }
    Ok(BankListResponse { banks })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BankDeleteRequest {
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BankDeleteResponse {
    pub ok: bool,
    /// Where the bank folder was moved (under the store's `.trash/`).
    pub trashed_to: String,
}

pub async fn delete(
    deps: &Deps,
    req: BankDeleteRequest,
) -> Result<BankDeleteResponse, MemoryError> {
    let store = deps.store().await;
    let trashed_to = store.trash_bank(&req.name).await?;
    deps.emitter.bank("trashed", &req.name).await;
    Ok(BankDeleteResponse {
        ok: true,
        trashed_to,
    })
}
