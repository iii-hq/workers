//! `tmux::accounts::*`: Claude Code accounts (agent-accounts §3.4).

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, spec, Ctx, EmptyInput, EmptyOutput, FunctionSpec};
use crate::accounts::launch::Launch;
use crate::accounts::{tmux_env, GLOBAL};
use crate::model::{
    ClaudeAccount, ClaudeSnapshot, LoginBrowser, RulesMode, SwitchRules, Threshold,
};
use crate::tmux_cli::Tmux;

pub const LIST: &str = "tmux::accounts::list";
pub const ADD: &str = "tmux::accounts::add";
pub const LOGIN_START: &str = "tmux::accounts::login_start";
pub const LOGIN_CODE: &str = "tmux::accounts::login_code";
pub const LOGIN_CANCEL: &str = "tmux::accounts::login_cancel";
pub const UPDATE: &str = "tmux::accounts::update";
pub const REMOVE: &str = "tmux::accounts::remove";
pub const GLOBAL_FN: &str = "tmux::accounts::global";
pub const MOVE_IDLE: &str = "tmux::accounts::move_idle";
pub const RULES: &str = "tmux::accounts::rules";
pub const DISMISS: &str = "tmux::accounts::dismiss";

const LIST_DESC: &str = "The Claude Code accounts on this computer: each one's name, colour, config folder, email, plan, whether it is signed in, the Claude Code panes using it and its latest usage reading; which one is global (the sign-in of ~/.claude, what every Claude Code that isn't pinned uses, plain `claude` included); a switch waiting for windows to finish their turn (`pending_global`), how long a session may take to notice a switch (`switch_delay_s`), the rules for when the global account runs low, any suggestion and the latest switch. Without `accounts.json` there is one account, `default`.";
const ADD_DESC: &str = "Add a Claude Code account: a config folder of its own (CLAUDE_CONFIG_DIR) under the accounts folder, sharing with ~/.claude what `shared` names (settings, extensions, plugins, conversations; all of them by default) through links. It starts signed out: tmux::accounts::login_start signs it in.";
const LOGIN_START_DESC: &str = "Start signing a Claude Code account in: runs `claude auth login` (with `--email` when the account has one) on the account's folder, or on ~/.claude for the global account, in a hidden terminal (no tmux window), and answers with the sign-in link (`url`) once Claude Code printed it. The page it opens shows a code to pass to tmux::accounts::login_code. `browser`: `none` (the default) keeps Claude Code from opening a browser on this computer; `host` lets it, for an app running on this computer: coming back from that page finishes the sign-in with no code (`browser_url` is the link it opened; only this answer carries it, never the feed or tmux::accounts::list). `host` fails when nothing here can open a browser (no BROWSER, and no xdg-open with a display). The link works for 10 minutes (`expires_at_ms`); then Claude Code is stopped (a code being checked, or with `host` a return from the page, gets 15 seconds more to finish). A new start for the same account cancels the one under way, and one still starting answers `gone`. While a sign-in runs (or its Claude Code is still there), switching the global account away from or to the accounts it involves is refused (`signing_in`); while a switch left half-way is being settled, no sign-in starts on ~/.claude or on its two accounts. The feed's `logins` shows every sign-in under way (`waiting_code`, `checking`) and those that ended in the last minute (`done`, `failed`, `expired`). Signing in to another account on ~/.claude makes that one global (added when it is new). On macOS, Claude Code keeps the sign-in in the login Keychain, which only a worker running in the user's GUI session (a LaunchAgent, not an SSH or background shell) can write. Errors are one JSON object `{code, message}`: `failed`, or `gone` when a later start for the account replaced this one (or the account was removed, or the worker is stopping).";
const LOGIN_CODE_DESC: &str = "Pass the code from the sign-in page to a sign-in under way (tmux::accounts::login_start): its whitespace is taken out, and a code without Claude Code's shape (`<code>#<state>`) is refused (`invalid_code`) without reaching Claude Code. Then waits up to a minute for Claude Code to check it with Anthropic and answers `{state: \"done\", account}` (the account signed in; on ~/.claude the global one, which may be another account than asked for when the person signed in to it), or an error, one JSON object `{code, message}`: `invalid_code` (the sign-in still waits), `rejected` (Anthropic didn't accept the code; Claude Code ended, so a new link is needed: a code only works with the link it came from), `expired` (the 10 minutes ran out, also for a code typed after them), `gone` (no such sign-in, cancelled, replaced, its account removed, or over) or `failed` (anything else). A second call while a code is checked waits for the same outcome; a call after the sign-in ended answers how it ended while it is listed, then `gone`. `done` means a new sign-in is where it belongs (the account's folder, or ~/.claude taken in as the global account); Claude Code saying it signed in while the one there is still the one from before is `failed`. The code is never logged or kept.";
const LOGIN_CANCEL_DESC: &str = "Stop a sign-in under way (tmux::accounts::login_start): Claude Code is stopped and the sign-in leaves the feed's `logins`; a tmux::accounts::login_code waiting on it answers `gone`. Nothing happens for one that is over or unknown.";
const UPDATE_DESC: &str = "Rename or recolour a Claude Code account, or change what it shares with ~/.claude (links are made for groups added and the worker's links removed for groups dropped; files of the account's own are never touched).";
const REMOVE_DESC: &str = "Take a Claude Code account off the list. Not the default account, the global one, nor one a switch waits to make global. Claude Code panes on its folder keep running. A sign-in of it from the app stops. With delete_files, its folder (sign-in, history of its own) is deleted too, never what its links point at; refused while Claude Code runs on it (a pane, or a sign-in until its Claude Code is gone).";

const GLOBAL_DESC: &str = "Make a Claude Code account the global one (it must be signed in), like /login: its sign-in moves into ~/.claude and the old global account's goes to that account's folder. Every Claude Code on ~/.claude that isn't pinned, started by the worker or by hand, uses the new account with its next message, without a restart (`following`; within `switch_delay_s` seconds on macOS). Windows pinned to the new account are restarted on ~/.claude and windows pinned to the old one on its folder, resuming their conversations (`restarted`; working ones after their turn: `after_turn`). While a window pinned to the new account is in a turn, the switch waits (`state: waiting`, `waiting_for`, the snapshot's `pending_global`) and the worker finishes it once they are idle (a failure that may pass is tried again; one that can't calls it off, and the panes it waited for say why in `move_error`); another switch replaces it, switching to the current global account cancels it. A window pinned to the old account that can't be moved stays on ~/.claude and follows the new account from then on (counted in `following`, with a `move_error`). Errors are one JSON object `{code, message, blocked_by?}` with `code` `not_signed_in`, `busy` (Claude Code is renewing its sign-in), `blocked` (a Claude Code on that account's folder can't be restarted: `blocked_by`), `owner_check_on`, `signing_in` (a sign-in from tmux::accounts::login_start runs on ~/.claude or on either account's folder: finish or cancel it first; the rules and a waiting switch go on after it) or `failed` (anything else).";
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
pub struct LoginStartInput {
    /// Account id (`default` for the one `~/.claude` had at first).
    pub account: String,
    /// `none` (the default): no browser opens on this computer; `host`: Claude Code opens it here.
    #[serde(default)]
    pub browser: Option<LoginBrowser>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LoginStartOutput {
    /// Pass it to tmux::accounts::login_code and login_cancel.
    pub login_id: String,
    /// The account signing in.
    pub account: String,
    /// The sign-in page, for any device: it shows the code to paste.
    pub url: String,
    /// With `browser: host`, the link Claude Code opened in this computer's browser, whose return finishes the sign-in with no code (it works on this computer only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_url: Option<String>,
    /// When the link stops working and Claude Code is stopped (10 minutes).
    pub expires_at_ms: u64,
    /// The email the page is filled with (the account's), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// `none` or `host`.
    pub browser: LoginBrowser,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoginCodeInput {
    /// From tmux::accounts::login_start.
    pub login_id: String,
    /// The code the sign-in page shows (`<code>#<state>`), as copied: whitespace is taken out.
    pub code: String,
}

/// Always `done`: anything else is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginDone {
    Done,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LoginCodeOutput {
    /// `done`.
    pub state: LoginDone,
    /// The account signed in, as tmux::accounts::list shows it.
    pub account: ClaudeAccount,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoginCancelInput {
    /// From tmux::accounts::login_start.
    pub login_id: String,
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
/// with the swap gate, held until Claude Code has started
/// ([`create_with_claude`]): a switch would move the sign-in out of the
/// folder it starts on.
pub(super) async fn plan(
    ctx: &Ctx,
    claude: Option<&ClaudeLaunchInput>,
    command: Option<&str>,
) -> Result<Option<(Launch, crate::accounts::Gate)>, String> {
    let Some(claude) = claude else {
        return Ok(None);
    };
    if command.is_some() {
        return Err("Give either command or claude, not both.".to_string());
    }
    let accounts = ctx.collector.accounts();
    let gate = accounts.gate().await;
    let unsettled = accounts.unsettled_swap().await;
    let launch = accounts.plan_launch(&claude.account, claude.skip_permissions, None)?;
    if unsettled.is_some_and(|journal| {
        launch.on_slot || journal.from == launch.account || journal.to == launch.account
    }) {
        return Err(crate::accounts::switch::SWAP_UNSETTLED.to_string());
    }
    Ok(Some((launch, gate)))
}

/// Make a pane with `args` (tmux's command that starts `HOLD` in it and
/// prints its line, the pane's id read by `pane_of`), then start `launch`
/// in it, in a task of its own that holds `gate`: a call that goes away
/// meanwhile (its future dropped) leaves neither a pane holding nor the
/// gate free before Claude Code has started. Returns what tmux printed and
/// the command Claude Code runs.
pub(super) async fn create_with_claude(
    tmux: &Tmux,
    args: Vec<String>,
    launch: Launch,
    gate: crate::accounts::Gate,
    remain_on_exit: bool,
    pane_of: fn(&str) -> Result<String, String>,
) -> Result<(String, String), String> {
    let tmux = tmux.clone();
    tokio::spawn(async move {
        let _gate = gate;
        let output = tmux.run(&args).await.map_err(|error| error.sentence())?;
        let pane = pane_of(&output)?;
        let runs = start_claude(&tmux, &pane, &launch, remain_on_exit).await?;
        Ok((output, runs))
    })
    .await
    .map_err(|error| format!("The worker failed to finish ({error})."))?
}

/// Start `launch` in new pane `pane` (running `HOLD`): its marks and
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

/// Run `work` holding the swap gate, in a task of its own: once it has the
/// gate, it runs to its end even when the call goes away (the SDK drops a
/// handler on a timeout or a disconnect), and only then frees the gate. A
/// dropped call cancels only its wait for the gate. One switch, recovery
/// or move at a time, each on the panes as they are once it is its turn.
pub(super) async fn gated<T, Fut>(ctx: &Ctx, work: impl FnOnce() -> Fut) -> Result<T, String>
where
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let gate = ctx.collector.accounts().gate().await;
    let work = work();
    tokio::spawn(async move {
        let _gate = gate;
        work.await
    })
    .await
    .map_err(|error| format!("The worker failed to finish ({error})."))
}

/// What a move or switch says when tmux did not list its panes: a failed
/// look is no proof that nothing runs on an account's folder.
pub(crate) fn not_listed(error: &str, what: &str) -> String {
    format!("tmux did not list its panes ({error}); {what}")
}

pub async fn global(ctx: &Ctx, input: GlobalInput) -> Result<GlobalOutput, String> {
    use crate::accounts::switch::{switch_locked, By, SwitchError};
    ctx.tmux().await?;
    let collector = ctx.collector.clone();
    let switched = gated(ctx, move || async move {
        // The server as configured once it is this call's turn, and its
        // panes as they are then: the new account's folder must be free
        // (G4), which only a look that listed every pane can tell.
        let tmux = collector.tmux().await.map_err(SwitchError::Failed)?;
        let panes = collector
            .claude_panes_checked()
            .await
            .map_err(|error| SwitchError::Failed(not_listed(&error, "nothing was switched.")))?;
        switch_locked(
            &tmux,
            collector.accounts(),
            &panes,
            &input.account,
            By::Person,
        )
        .await
    })
    .await
    .unwrap_or_else(|error| Err(SwitchError::Failed(error)));
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
    ctx.tmux().await?;
    // Never during a switch: it moves sign-ins between the folders.
    let collector = ctx.collector.clone();
    let moved = gated(ctx, move || async move {
        let tmux = collector.tmux().await?;
        let panes = collector
            .claude_panes_checked()
            .await
            .map_err(|error| not_listed(&error, "nothing was moved."))?;
        collector
            .accounts()
            .load_accounts_now(vec![input.to.trim().to_string()])
            .await;
        crate::accounts::moves::move_idle(
            &tmux,
            collector.accounts(),
            &panes,
            input.from.trim(),
            input.to.trim(),
        )
        .await
    })
    .await
    .and_then(|moved| moved);
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
    let collector = ctx.collector.clone();
    gated(ctx, move || async move {
        let running = if input.delete_files {
            // Its folder goes only on a look that saw every pane, and every
            // process as it is now: a failed or reused one is no proof that
            // nothing runs on it.
            let panes = collector.claude_panes_rescanned().await.map_err(|error| {
                format!("The panes could not be looked at ({error}); the folder was kept.")
            })?;
            // A move stops Claude Code before it copies the conversation:
            // for that moment its pane runs nothing, yet the folder is
            // needed.
            if collector.accounts().moving.any() {
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
        let accounts = collector.accounts().clone();
        blocking(move || accounts.remove(&input.account, input.delete_files, running)).await
    })
    .await??;
    ctx.monitor.poke();
    Ok(EmptyOutput {})
}

/// What tells the feed a sign-in changed.
fn poke_of(ctx: &Ctx) -> crate::accounts::login::Poke {
    let monitor = ctx.monitor.clone();
    Arc::new(move || monitor.poke())
}

pub async fn login_start(ctx: &Ctx, input: LoginStartInput) -> Result<LoginStartOutput, String> {
    use crate::accounts::login::to_wire;
    use crate::model::LoginError;
    let failed = |message: String| {
        to_wire(&LoginError {
            code: "failed".into(),
            message,
        })
    };
    let accounts = ctx.collector.accounts().clone();
    let browser = input.browser.unwrap_or_default();
    let account = input.account.trim().to_string();
    // Where it signs in (`~/.claude` or the folder) is decided and Claude
    // Code started with no switch half-way.
    let gate = accounts.gate().await;
    // A switch left half-way moves sign-ins in and out of the slot and its
    // two accounts' folders until it is settled: no sign-in there.
    let unsettled = accounts.unsettled_swap().await;
    let plan = {
        let accounts = accounts.clone();
        blocking(move || accounts.login_plan(&account, browser))
            .await
            .map_err(failed)?
    };
    if unsettled.is_some_and(|journal| {
        plan.config_dir.is_none() || journal.from == plan.account || journal.to == plan.account
    }) {
        return Err(failed(crate::accounts::switch::SWAP_UNSETTLED.to_string()));
    }
    // To its end even when the call goes away: a Claude Code started is
    // either in the feed (and stopped after 10 minutes) or stopped now.
    let poke = poke_of(ctx);
    let started = tokio::spawn(async move {
        accounts
            .logins
            .start(accounts.clone(), plan, poke, gate)
            .await
    })
    .await
    .map_err(|error| failed(format!("The worker failed to finish ({error}).")))?
    .map_err(|error| to_wire(&error))?;
    Ok(LoginStartOutput {
        login_id: started.login.login_id,
        account: started.login.account,
        url: started.login.url,
        browser_url: started.browser_url,
        expires_at_ms: started.login.expires_at_ms,
        email: started.login.email,
        browser: started.login.browser,
    })
}

pub async fn login_code(ctx: &Ctx, input: LoginCodeInput) -> Result<LoginCodeOutput, String> {
    use crate::accounts::login::to_wire;
    use crate::model::LoginError;
    let id = ctx
        .collector
        .accounts()
        .logins
        .code(input.login_id.trim(), &input.code, poke_of(ctx))
        .await
        .map_err(|error| to_wire(&error))?;
    let account = account(ctx, &id).await.map_err(|message| {
        to_wire(&LoginError {
            code: "failed".into(),
            message,
        })
    })?;
    Ok(LoginCodeOutput {
        state: LoginDone::Done,
        account,
    })
}

pub async fn login_cancel(ctx: &Ctx, input: LoginCancelInput) -> Result<EmptyOutput, String> {
    if ctx
        .collector
        .accounts()
        .logins
        .cancel(input.login_id.trim())
    {
        ctx.monitor.poke();
    }
    Ok(EmptyOutput {})
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, ClaudeSnapshot>(LIST, LIST_DESC),
        spec::<AddAccountInput, AccountOutput>(ADD, ADD_DESC),
        spec::<LoginStartInput, LoginStartOutput>(LOGIN_START, LOGIN_START_DESC),
        spec::<LoginCodeInput, LoginCodeOutput>(LOGIN_CODE, LOGIN_CODE_DESC),
        spec::<LoginCancelInput, EmptyOutput>(LOGIN_CANCEL, LOGIN_CANCEL_DESC),
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
    register_fn!(
        iii,
        ctx,
        LOGIN_START,
        LOGIN_START_DESC,
        LoginStartInput,
        login_start
    );
    register_fn!(
        iii,
        ctx,
        LOGIN_CODE,
        LOGIN_CODE_DESC,
        LoginCodeInput,
        login_code
    );
    register_fn!(
        iii,
        ctx,
        LOGIN_CANCEL,
        LOGIN_CANCEL_DESC,
        LoginCancelInput,
        login_cancel
    );
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
