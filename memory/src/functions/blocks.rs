//! `memory::block::*` — the always-injected markdown documents. Plain
//! files under `<bank>/blocks/`; editing them by hand is equivalent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::types::Block;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockListRequest {
    /// Bank whose blocks to list; the configured default when omitted.
    #[serde(default)]
    pub bank: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockListResponse {
    pub bank: String,
    pub blocks: Vec<Block>,
}

pub async fn list(deps: &Deps, req: BlockListRequest) -> Result<BlockListResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    Ok(BlockListResponse {
        bank: bank_name,
        blocks: bank.list_blocks()?,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockSetRequest {
    /// Bank to write into; the configured default when omitted. Created
    /// on first use.
    #[serde(default)]
    pub bank: Option<String>,
    /// Block name, `[a-z0-9][a-z0-9_-]{0,63}` (it becomes `<name>.md`).
    pub name: String,
    /// Markdown content. Empty removes the block.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockSetResponse {
    pub ok: bool,
    /// False when the empty content removed the block.
    pub exists: bool,
}

pub async fn set(deps: &Deps, req: BlockSetRequest) -> Result<BlockSetResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let (bank, created) = store.ensure_bank(&bank_name, None).await?;
    if created {
        deps.emitter.bank("created", &bank_name).await;
    }
    let exists = !req.content.trim().is_empty();
    bank.set_block(&req.name, &req.content)?;
    deps.emitter.bank("blocks-changed", &bank_name).await;
    Ok(BlockSetResponse { ok: true, exists })
}
