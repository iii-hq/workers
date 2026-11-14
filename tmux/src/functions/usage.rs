use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{Ctx, FunctionSpec, register_fn, spec};
use crate::model::UsageSnapshot;

pub const READ: &str = "tmux::usage::read";

const READ_DESC: &str = "The weekly usage of Claude Code (per account: 5-hour window, week, per-model weeks, each with when it resets) and of Codex (plan, 5-hour window, week, credits), with the Claude Code accounts, the global one, the rules and any suggestion. The same object the live feed carries as `usage`. Readings are at most five minutes old while the app watches; refresh asks again now (at most once a minute per account). Never returns a token.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct UsageReadInput {
    /// Read every signed-in account again now instead of returning the
    /// latest readings (each account at most once a minute).
    #[serde(default)]
    pub refresh: Option<bool>,
}

pub async fn read(ctx: &Ctx, input: UsageReadInput) -> Result<UsageSnapshot, String> {
    let accounts = ctx.collector.accounts().clone();
    accounts.mark_watched();
    if input.refresh == Some(true) {
        accounts.refresh(None, true).await;
    } else {
        // Readings that are due are taken in the background; this answers now.
        let monitor = ctx.monitor.clone();
        tokio::spawn(async move {
            if accounts.refresh(None, false).await {
                monitor.poke();
            }
        });
    }
    let overview = ctx.collector.overview().await;
    ctx.monitor.poke();
    overview
        .usage
        .ok_or_else(|| "Usage is not available.".to_string())
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![spec::<UsageReadInput, UsageSnapshot>(READ, READ_DESC)]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, READ, READ_DESC, UsageReadInput, read);
}
