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

const LIST_DESC: &str = "The Claude Code accounts on this computer: each one's name, colour, config folder, email, plan, whether it is signed in, the Claude Code panes using it and its latest usage reading; which one is global (the sign-in of ~/.claude, what every Claude Code that isn't pinned uses, plain `claude` included); a switch waiting for windows to finish their turn (`pending_global`), how long a session may take to notice a switch (`switch_delay_s`), the rules for when the global account runs low, any suggestion and the latest switch. Without `accounts.json` there is one account, `default`.";
const ADD_DESC: &str = "Add a Claude Code account: a config folder of its own (CLAUDE_CONFIG_DIR) under the accounts folder, sharing with ~/.claude what `shared` names (settings, extensions, plugins, conversations; all of them by default) through links. It starts signed out: tmux::accounts::login signs it in.";
const LOGIN_DESC: &str = "Sign a Claude Code account in: opens a window named `login` in `session` running `claude auth login`, on the account's folder, or on ~/.claude for the global account, which prints a sign-in link and waits for the code. The window closes when the command ends; the account then shows signed in. Signing in to another account on ~/.claude makes that one global (added when it is new).";
const UPDATE_DESC: &str = "Rename or recolour a Claude Code account, or change what it shares with ~/.claude (links are made for groups added and the worker's links removed for groups dropped; files of the account's own are never touched).";
const REMOVE_DESC: &str = "Take a Claude Code account off the list. Not the default account, the global one, nor one a switch waits to make global. Claude Code panes on its folder keep running. With delete_files, its folder (sign-in, history of its own) is deleted too, never what its links point at; refused while Claude Code runs on it.";

const GLOBAL_DESC: &str = "Make a Claude Code account the global one (it must be signed in), like /login: its sign-in moves into ~/.claude and the old global account's goes to that account's folder. Every Claude Code on ~/.claude that isn't pinned, started by the worker or by hand, uses the new account with its next message, without a restart (`following`; within `switch_delay_s` seconds on macOS). Windows pinned to the new account are restarted on ~/.claude and windows pinned to the old one on its folder, resuming their conversations (`restarted`; working ones after their turn: `after_turn`). While a window pinned to the new account is in a turn, the switch waits (`state: waiting`, `waiting_for`, the snapshot's `pending_global`) and the worker finishes it once they are idle; another switch replaces it, switching to the current global account cancels it. Errors are one JSON object `{code, message, blocked_by?}` with `code` `not_signed_in`, `busy` (Claude Code is renewing its sign-in), `blocked` (a Claude Code on that account's folder can't be restarted: `blocked_by`), `owner_check_on` or `failed` (anything else).";
const MOVE_IDLE_DESC: &str = "Move the idle Claude Code panes using account `from` (on its folder, or on ~/.claude when it is the global one) to account `to` and pin them there, resuming their conversations: those that follow the global account, are pinned to `from` or were started by hand (never those pinned elsewhere, nor working or waiting ones).";

const RULES_DESC: &str = "Change the rules for when the global Claude Code account runs low (what is left out stays): mode `off`, `suggest` (a suggestion in the feed) or `switch` (the worker makes another account global by itself, even with the app closed, and leaves `last_switch`; never one with Claude Code working on its folder); `at` `week_80`, `week_90` or `limit`; `five_hour` also acts when the 5-hour window is full; `back_after_reset` switches back once the old account's window resets; `order` the accounts to try first, then the one with most room this week.";
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

/// The launch `claude` asks for; never together with `command`. It comes
/// with the swap gate, held until Claude Code has started: a switch would
/// move the sign-in out of the folder it starts on.
pub(super) async fn plan<'a>(
    ctx: &'a Ctx,
    claude: Option<&ClaudeLaunchInput>,
    command: Option<&str>,
) -> Result<Option<(Launch, tokio::sync::MutexGuard<'a, ()>)>, String> {
    let Some(claude) = claude else {
        return Ok(None);
    };
    if command.is_some() {
        return Err("Give either command or claude, not both.".to_string());
    }
    let accounts = ctx.collector.accounts();
    let gate = accounts.swap_gate.lock().await;
    accounts
        .plan_launch(&claude.account, claude.skip_permissions, None)
        .map(|launch| Some((launch, gate)))
}

/// Start `launch` in new pane `pane` (running [`HOLD`]): its marks and
/// remain-on-exit, then Claude Code. A start on the slot unsets
/// `CLAUDE_CONFIG_DIR` outright when the pane's session or tmux's global
/// environment sets it (G5). Returns the command it runs (`runs`). On
/// failure the pane is gone.
pub(super) async fn start_claude(
    tmux: &Tmux,
    pane: &str,
    launch: &Launch,
    remain_on_exit: bool,
) -> Result<String, String> {
    let mut command = launch.command.clone();
    if launch.on_slot {
        match tmux_env::pane_sets_var(tmux, pane).await {
            Ok(false) => {}
            Ok(true) => command = launch.explicit(),
            Err(error) => {
                // Not known: the slot named outright is right either way.
                tracing::debug!(%error, "tmux environment not looked at");
                command = launch.explicit();
            }
        }
    }
    let mut options = launch.marks();
    // Off unless asked for, whatever tmux's own default says.
    options.push(("remain-on-exit", if remain_on_exit { "on" } else { "off" }));
    tmux.start_with_options(pane, &options, Some(&command))
        .await?;
    Ok(command)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GlobalInput {
    /// Account id.
    pub account: String,
}

/// `switched` (the account is global now) or `waiting` (for `waiting_for`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GlobalState {
    Switched,
    Waiting,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GlobalOutput {
    /// The global account now (the old one while the switch waits).
    pub global: String,
    pub state: GlobalState,
    /// Claude Code panes on ~/.claude that aren't pinned: they use the new account with their next message.
    pub following: u32,
    /// Pinned panes restarted now (the new account's on ~/.claude, the old one's on its folder).
    pub restarted: Vec<String>,
    /// Panes pinned to the old account that restart on its folder once their turn ends.
    pub after_turn: Vec<String>,
    /// Panes on the new account's folder the switch waits for.
    pub waiting_for: Vec<String>,
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
    use crate::accounts::switch::{switch_locked, By};
    let tmux = ctx.tmux().await?;
    // One switch or move at a time, and the panes as they are once it is
    // this one's turn.
    let gate = ctx.collector.accounts().swap_gate.lock().await;
    let panes = ctx.collector.claude_panes(true).await;
    let switched = switch_locked(
        &tmux,
        ctx.collector.accounts(),
        &panes,
        &input.account,
        By::Person,
    )
    .await;
    drop(gate);
    ctx.monitor.poke();
    let outcome = switched.map_err(|error| error.to_wire())?;
    Ok(GlobalOutput {
        global: outcome.global,
        state: if outcome.waiting {
            GlobalState::Waiting
        } else {
            GlobalState::Switched
        },
        following: outcome.following,
        restarted: outcome.restarted,
        after_turn: outcome.after_turn,
        waiting_for: outcome.waiting_for,
    })
}

pub async fn move_idle(ctx: &Ctx, input: MoveIdleInput) -> Result<MoveIdleOutput, String> {
    let tmux = ctx.tmux().await?;
    // Never during a switch: it moves sign-ins between the folders.
    let gate = ctx.collector.accounts().swap_gate.lock().await;
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
    drop(gate);
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
    // Not while a switch runs: it could be making this account global.
    let gate = ctx.collector.accounts().swap_gate.lock().await;
    let running = if input.delete_files {
        // Its folder goes only on a look that saw every pane: a failed one
        // is no proof that nothing runs on it.
        let panes = ctx
            .collector
            .claude_panes_checked()
            .await
            .map_err(|error| {
                format!("The panes could not be looked at ({error}); the folder was kept.")
            })?;
        // A move stops Claude Code before it copies the conversation: for
        // that moment its pane runs nothing, yet the folder is needed.
        if ctx.collector.accounts().moving.any() {
            return Err(
                "A Claude Code pane is being moved; delete the folder once that is over."
                    .to_string(),
            );
        }
        let folder = crate::accounts::slot::Place::Folder(input.account.trim().to_string());
        panes
            .values()
            .filter(|pane| pane.running() && pane.place.as_ref() == Some(&folder))
            .count() as u32
    } else {
        0
    };
    let accounts = ctx.collector.accounts().clone();
    let removed =
        blocking(move || accounts.remove(&input.account, input.delete_files, running)).await;
    drop(gate);
    removed?;
    ctx.monitor.poke();
    Ok(EmptyOutput {})
}

pub async fn login(ctx: &Ctx, input: LoginInput) -> Result<LoginWindow, String> {
    // Where it signs in (`~/.claude` or the folder) is decided and started
    // with no switch half-way.
    let _gate = ctx.collector.accounts().swap_gate.lock().await;
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
