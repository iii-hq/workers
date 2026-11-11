//! `tmux::accounts::*`: Claude Code accounts (agent-accounts §3.4).

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, EmptyInput, EmptyOutput, FunctionSpec};
use crate::accounts::launch::Launch;
use crate::accounts::{tmux_env, GLOBAL};
use crate::model::{ClaudeAccount, ClaudeSnapshot, RulesMode, SwitchRules, Threshold};
use crate::tmux_cli::{format_literal, resolve_cwd, Tmux, HOLD, SEP};

pub const LIST: &str = "tmux::accounts::list";
pub const ADD: &str = "tmux::accounts::add";
pub const LOGIN: &str = "tmux::accounts::login";
pub const UPDATE: &str = "tmux::accounts::update";
pub const REMOVE: &str = "tmux::accounts::remove";
pub const GLOBAL_FN: &str = "tmux::accounts::global";
pub const MOVE_IDLE: &str = "tmux::accounts::move_idle";
pub const RULES: &str = "tmux::accounts::rules";
pub const DISMISS: &str = "tmux::accounts::dismiss";

const LIST_DESC: &str = "The Claude Code accounts on this computer: each one's name, colour, config folder, email, plan, whether it is signed in, the Claude Code panes running on it and its latest usage reading; which one is global (what plain `claude` gets in new tmux windows); the rules for when the global account runs low, any suggestion and the latest switch. Without `accounts.json` there is one account, `default` (~/.claude).";
const ADD_DESC: &str = "Add a Claude Code account: a config folder of its own (CLAUDE_CONFIG_DIR) under the accounts folder, sharing with ~/.claude what `shared` names (settings, extensions, plugins, conversations; all of them by default) through links. It starts signed out: tmux::accounts::login signs it in.";
const LOGIN_DESC: &str = "Sign a Claude Code account in: opens a window named `login` in `session` running `claude auth login` for that account's folder, which prints a sign-in link and waits for the code. The window closes when the command ends; the account then shows signed in.";
const UPDATE_DESC: &str = "Rename or recolour a Claude Code account, or change what it shares with ~/.claude (links are made for groups added and the worker's links removed for groups dropped; files of the account's own are never touched).";
const REMOVE_DESC: &str = "Take a Claude Code account off the list. Not the default account (~/.claude) nor the global one. Claude Code panes on it keep running. With delete_files, its folder (sign-in, history of its own) is deleted too, never what its links point at; refused while Claude Code runs on it.";

const GLOBAL_DESC: &str = "Make a Claude Code account the global one (it must be signed in): what plain `claude` gets in new tmux windows (tmux's global CLAUDE_CONFIG_DIR). Panes that follow the global account go along as `move` says: `idle_now_working_after` (default: idle ones move now, resuming their conversation; working or waiting ones after their turn) or `new_only` (they stay where they run, pinned there). Shells already open keep the environment they started with.";
const MOVE_IDLE_DESC: &str = "Move the idle Claude Code panes running on account `from` to account `to` and pin them there, resuming their conversations: those that follow the global account, are pinned to `from` or were started by hand (never those pinned elsewhere, nor working or waiting ones).";

const RULES_DESC: &str = "Change the rules for when the global Claude Code account runs low (what is left out stays): mode `off`, `suggest` (a suggestion in the feed) or `switch` (the worker makes another account global by itself, even with the app closed, and leaves `last_switch`); `at` `week_80`, `week_90` or `limit`; `five_hour` also acts when the 5-hour window is full; `move_idle` moves the idle windows that follow the global account on a switch (else they stay pinned where they run); `back_after_reset` switches back once the old account's window resets; `order` the accounts to try first, then the one with most room this week.";
const DISMISS_DESC: &str = "\"Not now\" on a suggestion to switch the global account: no suggestion from that account for that reason until its window resets or it runs lower (week 80 % → 90 % → 5-hour window full → week limit).";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddAccountInput {
    /// What people call it, e.g. `Personal`; its id is made from it.
    pub name: String,
    /// `blue`, `green`, `violet`, `orange` or `pink`.
    pub color: String,
    /// What it shares with ~/.claude: `settings` (settings.json, CLAUDE.md, keybindings.json), `extensions` (skills, agents, commands, output styles), `plugins` (plugins, and a copy of the MCP servers), `conversations` (projects, history, todos, plans). Default: all four.
    #[serde(default)]
    pub shared: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccountOutput {
    pub account: ClaudeAccount,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoginInput {
    /// Account id (`default` for ~/.claude).
    pub account: String,
    /// Session id (`$5`) or exact name the `login` window opens in.
    pub session: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LoginWindow {
    /// Session id.
    pub session_id: String,
    /// The `login` window's id.
    pub window_id: String,
    /// Its pane, running `claude auth login`.
    pub pane_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateAccountInput {
    /// Account id.
    pub account: String,
    /// New name.
    #[serde(default)]
    pub name: Option<String>,
    /// New colour: `blue`, `green`, `violet`, `orange` or `pink`.
    #[serde(default)]
    pub color: Option<String>,
    /// What it shares from now on (see tmux::accounts::add). Not for `default`.
    #[serde(default)]
    pub shared: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveAccountInput {
    /// Account id.
    pub account: String,
    /// Delete its folder too (default false: the folder stays where it was).
    #[serde(default)]
    pub delete_files: bool,
}

/// `claude` on tmux::windows::create and tmux::sessions::create.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ClaudeLaunchInput {
    /// `global` (the default: it runs on the global account and moves when that changes) or an account id (pinned to it).
    #[serde(default = "global_account")]
    pub account: String,
    /// Start it with --dangerously-skip-permissions (default false). Refused when the host runs as root.
    #[serde(default)]
    pub skip_permissions: bool,
}

fn global_account() -> String {
    GLOBAL.to_string()
}

/// The launch `claude` asks for; never together with `command`.
pub(super) fn plan(
    ctx: &Ctx,
    claude: Option<&ClaudeLaunchInput>,
    command: Option<&str>,
) -> Result<Option<Launch>, String> {
    let Some(claude) = claude else {
        return Ok(None);
    };
    if command.is_some() {
        return Err("Give either command or claude, not both.".to_string());
    }
    ctx.collector
        .accounts()
        .plan_launch(&claude.account, claude.skip_permissions, None)
        .map(Some)
}

/// Start `launch` in new pane `pane` (running [`HOLD`]): tmux's global
/// environment first for a pane that follows the global account (D3), then
/// its marks and remain-on-exit, then Claude Code.
pub(super) async fn start_claude(
    ctx: &Ctx,
    tmux: &Tmux,
    pane: &str,
    launch: &Launch,
    remain_on_exit: bool,
) -> Result<(), String> {
    if launch.account == GLOBAL {
        if let Err(error) = tmux_env::reconcile(tmux, ctx.collector.accounts()).await {
            // The pane's account is read from its process (D5), so it never
            // shows the wrong one.
            tracing::warn!(%error, "tmux global CLAUDE_CONFIG_DIR not set");
        }
    }
    let mut options = launch.marks();
    if remain_on_exit {
        options.push(("remain-on-exit", "on"));
    }
    tmux.start_with_options(pane, &options, Some(&launch.command))
        .await
}

/// How panes that follow the global account go along (D8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GlobalMove {
    /// Idle ones move now, working or waiting ones after their turn.
    #[default]
    IdleNowWorkingAfter,
    /// They stay on the account they run on, pinned there.
    NewOnly,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GlobalInput {
    /// Account id.
    pub account: String,
    /// What panes following the global account do (default `idle_now_working_after`).
    #[serde(default, rename = "move")]
    pub move_mode: GlobalMove,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GlobalOutput {
    /// The global account now.
    pub global: String,
    /// Panes moved to it now.
    pub moved: Vec<String>,
    /// Panes that move once their turn ends.
    pub after_turn: Vec<String>,
    /// Panes that stay where they run (pinned there with `new_only`, or that can't move).
    pub stayed: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveIdleInput {
    /// Account id they run on.
    pub from: String,
    /// Account id to move them to.
    pub to: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MoveIdleOutput {
    /// Panes moved.
    pub moved: Vec<String>,
}

pub async fn global(ctx: &Ctx, input: GlobalInput) -> Result<GlobalOutput, String> {
    use crate::accounts::moves::{switch_global, SwitchBy, SwitchMode};
    let tmux = ctx.tmux().await?;
    let panes = ctx.collector.claude_panes(true).await;
    // Signed in as of now (a sign-in may have just finished): that account
    // alone is read.
    ctx.collector
        .accounts()
        .load_accounts_now(vec![input.account.trim().to_string()])
        .await;
    let mode = match input.move_mode {
        GlobalMove::IdleNowWorkingAfter => SwitchMode::IdleNowWorkingAfter,
        GlobalMove::NewOnly => SwitchMode::NewOnly,
    };
    let switched = switch_global(
        &tmux,
        ctx.collector.accounts(),
        &panes,
        &input.account,
        mode,
        SwitchBy::Person,
    )
    .await;
    ctx.monitor.poke();
    let switched = switched?;
    Ok(GlobalOutput {
        global: input.account.trim().to_string(),
        moved: switched.moved,
        after_turn: switched.after_turn,
        stayed: switched.stayed,
    })
}

pub async fn move_idle(ctx: &Ctx, input: MoveIdleInput) -> Result<MoveIdleOutput, String> {
    let tmux = ctx.tmux().await?;
    let panes = ctx.collector.claude_panes(true).await;
    ctx.collector
        .accounts()
        .load_accounts_now(vec![input.to.trim().to_string()])
        .await;
    let moved = crate::accounts::moves::move_idle(
        &tmux,
        ctx.collector.accounts(),
        &panes,
        input.from.trim(),
        input.to.trim(),
    )
    .await;
    ctx.monitor.poke();
    Ok(MoveIdleOutput { moved: moved? })
}

/// The rules to change; what is left out stays.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RulesInput {
    /// `off`, `suggest` or `switch`.
    #[serde(default)]
    pub mode: Option<RulesMode>,
    /// `week_80`, `week_90` or `limit`.
    #[serde(default)]
    pub at: Option<Threshold>,
    /// Also act when the 5-hour window is full.
    #[serde(default)]
    pub five_hour: Option<bool>,
    /// A switch moves the idle windows that follow the global account.
    #[serde(default)]
    pub move_idle: Option<bool>,
    /// Switch back once the old account's window resets.
    #[serde(default)]
    pub back_after_reset: Option<bool>,
    /// Account ids to try first, in order.
    #[serde(default)]
    pub order: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RulesOutput {
    pub rules: SwitchRules,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DismissInput {
    /// The suggestion's `id`.
    pub suggestion: String,
}

pub async fn rules(ctx: &Ctx, input: RulesInput) -> Result<RulesOutput, String> {
    let accounts = ctx.collector.accounts().clone();
    let rules = blocking(move || {
        accounts.set_rules(|rules| {
            if let Some(mode) = input.mode {
                rules.mode = mode;
            }
            if let Some(at) = input.at {
                rules.at = at;
            }
            if let Some(five_hour) = input.five_hour {
                rules.five_hour = five_hour;
            }
            if let Some(move_idle) = input.move_idle {
                rules.move_idle = move_idle;
            }
            if let Some(back) = input.back_after_reset {
                rules.back_after_reset = back;
            }
            if let Some(order) = input.order {
                rules.order = order.iter().map(|id| id.trim().to_string()).collect();
            }
        })
    })
    .await?;
    ctx.monitor.poke();
    Ok(RulesOutput { rules })
}

pub async fn dismiss(ctx: &Ctx, input: DismissInput) -> Result<EmptyOutput, String> {
    let accounts = ctx.collector.accounts().clone();
    blocking(move || accounts.dismiss(input.suggestion.trim())).await?;
    ctx.monitor.poke();
    Ok(EmptyOutput {})
}

pub async fn list(ctx: &Ctx, _input: EmptyInput) -> Result<ClaudeSnapshot, String> {
    ctx.collector.accounts().mark_watched();
    ctx.collector.accounts().load_now().await;
    let overview = ctx.collector.overview().await;
    overview
        .usage
        .map(|usage| usage.claude)
        .ok_or_else(|| "Claude Code accounts are not available.".to_string())
}

/// Account `id` as the feed shows it now.
async fn account(ctx: &Ctx, id: &str) -> Result<ClaudeAccount, String> {
    ctx.collector.accounts().load_now().await;
    ctx.monitor.poke();
    ctx.collector
        .overview()
        .await
        .usage
        .and_then(|usage| usage.claude.accounts.into_iter().find(|a| a.id == id))
        .ok_or_else(|| crate::accounts::manage::no_account(id))
}

/// Run blocking file work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("The worker failed to finish ({error})."))?
}

pub async fn add(ctx: &Ctx, input: AddAccountInput) -> Result<AccountOutput, String> {
    let accounts = ctx.collector.accounts().clone();
    let id = blocking(move || accounts.add(&input.name, &input.color, input.shared)).await?;
    Ok(AccountOutput {
        account: account(ctx, &id).await?,
    })
}

pub async fn update(ctx: &Ctx, input: UpdateAccountInput) -> Result<AccountOutput, String> {
    let accounts = ctx.collector.accounts().clone();
    let id = input.account.clone();
    blocking(move || {
        accounts.update(
            &input.account,
            input.name.as_deref(),
            input.color.as_deref(),
            input.shared,
        )
    })
    .await?;
    Ok(AccountOutput {
        account: account(ctx, &id).await?,
    })
}

pub async fn remove(ctx: &Ctx, input: RemoveAccountInput) -> Result<EmptyOutput, String> {
    let running = ctx
        .collector
        .claude_panes(true)
        .await
        .values()
        .filter(|pane| pane.claude_pid.is_some())
        .filter(|pane| pane.info.account.as_deref() == Some(input.account.as_str()))
        .count() as u32;
    let accounts = ctx.collector.accounts().clone();
    blocking(move || accounts.remove(&input.account, input.delete_files, running)).await?;
    ctx.monitor.poke();
    Ok(EmptyOutput {})
}

pub async fn login(ctx: &Ctx, input: LoginInput) -> Result<LoginWindow, String> {
    let accounts = ctx.collector.accounts().clone();
    let id = input.account.clone();
    let command = blocking(move || accounts.login_command(&id)).await?;
    let tmux = ctx.tmux().await?;
    let session_id = tmux.resolve_session(&input.session).await?;
    let format = [
        "#{session_id}",
        "#{window_id}",
        "#{window_index}",
        "#{pane_id}",
    ]
    .join(&SEP.to_string());
    let home = resolve_cwd("~").unwrap_or_else(|_| "/".to_string());
    let args: Vec<String> = vec![
        "new-window".into(),
        "-P".into(),
        "-F".into(),
        format,
        "-t".into(),
        format!("{session_id}:"),
        "-n".into(),
        tmux.new_window_name_arg("login"),
        "-c".into(),
        format_literal(&home),
        "--".into(),
        HOLD.into(),
    ];
    let output = tmux.run(&args).await.map_err(|error| error.sentence())?;
    let created = super::windows::parse_created(&output)?;
    // The window closes when the sign-in ends, whatever the user's
    // remain-on-exit says.
    tmux.start_held(&created.pane_id, false, Some(&command))
        .await?;
    ctx.monitor.poke();
    Ok(LoginWindow {
        session_id: created.session_id,
        window_id: created.window_id,
        pane_id: created.pane_id,
    })
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, ClaudeSnapshot>(LIST, LIST_DESC),
        spec::<AddAccountInput, AccountOutput>(ADD, ADD_DESC),
        spec::<LoginInput, LoginWindow>(LOGIN, LOGIN_DESC),
        spec::<UpdateAccountInput, AccountOutput>(UPDATE, UPDATE_DESC),
        spec::<RemoveAccountInput, EmptyOutput>(REMOVE, REMOVE_DESC),
        spec::<GlobalInput, GlobalOutput>(GLOBAL_FN, GLOBAL_DESC),
        spec::<MoveIdleInput, MoveIdleOutput>(MOVE_IDLE, MOVE_IDLE_DESC),
        spec::<RulesInput, RulesOutput>(RULES, RULES_DESC),
        spec::<DismissInput, EmptyOutput>(DISMISS, DISMISS_DESC),
    ]
}

pub fn register(iii: &IIIClient, ctx: &Arc<Ctx>) {
    register_fn!(iii, ctx, LIST, LIST_DESC, EmptyInput, list);
    register_fn!(iii, ctx, ADD, ADD_DESC, AddAccountInput, add);
    register_fn!(iii, ctx, LOGIN, LOGIN_DESC, LoginInput, login);
    register_fn!(iii, ctx, UPDATE, UPDATE_DESC, UpdateAccountInput, update);
    register_fn!(iii, ctx, REMOVE, REMOVE_DESC, RemoveAccountInput, remove);
    register_fn!(iii, ctx, GLOBAL_FN, GLOBAL_DESC, GlobalInput, global);
    register_fn!(
        iii,
        ctx,
        MOVE_IDLE,
        MOVE_IDLE_DESC,
        MoveIdleInput,
        move_idle
    );
    register_fn!(iii, ctx, RULES, RULES_DESC, RulesInput, rules);
    register_fn!(iii, ctx, DISMISS, DISMISS_DESC, DismissInput, dismiss);
}
