//! `tmux::accounts::*`: Claude Code accounts (agent-accounts §3.4).

use std::sync::Arc;

use iii_sdk::IIIClient;

use super::{register_fn, spec, Ctx, EmptyInput, FunctionSpec};
use crate::model::ClaudeSnapshot;

pub const LIST: &str = "tmux::accounts::list";

const LIST_DESC: &str = "The Claude Code accounts on this computer: each one's name, colour, config folder, email, plan, whether it is signed in, the Claude Code panes running on it and its latest usage reading; which one is global (what plain `claude` gets in new tmux windows); the rules for when the global account runs low, any suggestion and the latest switch. Without `accounts.json` there is one account, `default` (~/.claude).";

pub async fn list(ctx: &Ctx, _input: EmptyInput) -> Result<ClaudeSnapshot, String> {
    ctx.collector.accounts().mark_watched();
    let overview = ctx.collector.overview().await;
    overview
        .usage
        .map(|usage| usage.claude)
        .ok_or_else(|| "Claude Code accounts are not available.".to_string())
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![spec::<EmptyInput, ClaudeSnapshot>(LIST, LIST_DESC)]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, LIST, LIST_DESC, EmptyInput, list);
}
