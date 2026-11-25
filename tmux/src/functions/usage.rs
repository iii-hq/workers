use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{Ctx, FunctionSpec, register_fn, spec};
use crate::model::{OutsideSpend, UsageSnapshot};

pub const READ: &str = "tmux::usage::read";
pub const CLAUDE_OUTSIDE: &str = "tmux::usage::claude_outside";

const READ_DESC: &str = "The weekly usage of Claude Code (per account: 5-hour window, week, per-model weeks, each with when it resets) and of Codex (plan, 5-hour window, week, credits), with the Claude Code accounts, the global one, the rules and any suggestion. The same object the live feed carries as `usage`. Readings are at most five minutes old while the app watches; refresh asks again now (at most once a minute per account). Never returns a token: only the internal tmux::accounts::credential does.";

const CLAUDE_OUTSIDE_DESC: &str = "The Claude Code spend on account `account` outside the Chat since `since_ms` (tmux windows and any other Claude Code on this computer), as tokens by model (`input`, `output`, `cache_read`, `cache_write`, `messages`), from Claude Code's own transcripts. A session tmux ran in a pane counts for that pane's account at each reply's time (or, on ~/.claude, the account global then); any other session counts for the account that was global when it replied (`global_history`), and `attributed` says how many replies were counted that way. Each reply counts once, by its message id (else its request id), wherever its transcript was copied. Read when asked, only what Claude Code added since the last read, counted by the minute, and kept five minutes (`as_of_ms`). `global` names the account global now. Token counts only; never a token. Errors are one JSON object `{code, message}`: `unknown_account`.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaudeOutsideInput {
    /// An account id, or `global` for the account global now.
    pub account: String,
    /// Count replies from this time on (milliseconds since the epoch), e.g. the account's week start: its `week.resets_at_ms` minus seven days.
    pub since_ms: u64,
}

pub async fn claude_outside(ctx: &Ctx, input: ClaudeOutsideInput) -> Result<OutsideSpend, String> {
    ctx.collector
        .accounts()
        .spend_outside(&input.account, input.since_ms)
        .await
        .ok_or_else(|| {
            serde_json::json!({
                "code": "unknown_account",
                "message": crate::accounts::manage::no_account(input.account.trim()),
            })
            .to_string()
        })
}

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
    vec![
        spec::<UsageReadInput, UsageSnapshot>(READ, READ_DESC),
        spec::<ClaudeOutsideInput, OutsideSpend>(CLAUDE_OUTSIDE, CLAUDE_OUTSIDE_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, READ, READ_DESC, UsageReadInput, read);
    register_fn!(
        iii,
        ctx,
        CLAUDE_OUTSIDE,
        CLAUDE_OUTSIDE_DESC,
        ClaudeOutsideInput,
        claude_outside
    );
}
