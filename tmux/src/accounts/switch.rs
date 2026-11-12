//! Switching the global account (agent-accounts G3, G4, G6, G12): the
//! panes around the swap of sign-ins `slot` makes.
//!
//! - Panes on the slot that are not pinned (the worker's `global` ones and
//!   any `claude` started by hand there) follow by themselves: Claude Code
//!   reads its sign-in again on its next request. Nothing is done to them.
//! - Panes on the new account's folder: its sign-in can't move while they
//!   run. Idle ones are stopped before the swap and started again on the
//!   slot after it, still pinned to that account. Working or waiting ones
//!   make the switch wait (`pending_global`), and the worker finishes it
//!   once they are idle. One that can't be stopped and started again
//!   refuses the switch (`blocked`).
//! - Panes pinned to the old global account (on the slot): idle ones are
//!   stopped before the swap and started on its folder after; working ones
//!   go there once their turn ends.
//! - A sign-in by hand on the slot (`/login`) makes that account global
//!   (G6), a new one added when nobody knows it.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::moves::{self, Stopped, NOT_IDLE};
use super::panes::PaneState;
use super::registry::{self, PendingSwitch, Registry};
use super::slot::{self, Crash, Journal, Place, SwapError};
use super::{Accounts, RecentSwap};
use crate::model::{now_ms, AccountSwitch};
use crate::tmux_cli::Tmux;

/// Who asks for a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum By {
    /// Someone, through `tmux::accounts::global`: noted in `last_switch`
    /// (`auto: false`), and any switch back the rules had planned is off.
    Person,
    /// The rules (D14). They decided on the registry of `generation`; a
    /// registry written since is left alone. `record` becomes
    /// `last_switch` with the swap. They never wait for windows.
    Rules {
        generation: u64,
        record: AccountSwitch,
    },
    /// The worker, finishing a switch that waited (`person` or `rule`).
    Pending(String),
}

impl By {
    fn word(&self) -> String {
        match self {
            Self::Person => "person".into(),
            Self::Rules { .. } => "rule".into(),
            Self::Pending(by) => by.clone(),
        }
    }
}

/// What a switch did (G12).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The global account now.
    pub global: String,
    /// It waits for `waiting_for` to finish their turn.
    pub waiting: bool,
    /// Panes on the slot that switch with their next message.
    pub following: u32,
    /// Pinned panes started again (the new account's on the slot, the old
    /// one's on its folder).
    pub restarted: Vec<String>,
    /// Panes pinned to the old account that go to its folder after their turn.
    pub after_turn: Vec<String>,
    /// Panes on the new account's folder the switch waits for.
    pub waiting_for: Vec<String>,
}

/// Why a switch did not happen, with the stable code the apps read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchError {
    NotSignedIn(String),
    Busy,
    /// Panes on the new account's folder that can't be stopped and started
    /// again (Claude Code started by hand, a shell that isn't back).
    Blocked(Vec<String>, String),
    OwnerCheckOn,
    /// The registry changed while the rules decided.
    Stale,
    Failed(String),
}

pub const RULES_STALE: &str =
    "The global account changed while the rules decided; nothing was switched.";

impl SwitchError {
    /// `not_signed_in`, `busy`, `blocked`, `owner_check_on`, `stale` or
    /// `failed`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotSignedIn(_) => "not_signed_in",
            Self::Busy => "busy",
            Self::Blocked(..) => "blocked",
            Self::OwnerCheckOn => "owner_check_on",
            Self::Stale => "stale",
            Self::Failed(_) => "failed",
        }
    }

    pub fn sentence(&self) -> String {
        match self {
            Self::NotSignedIn(text) | Self::Failed(text) => text.clone(),
            Self::Busy => slot::BUSY_TEXT.into(),
            Self::Blocked(panes, why) => format!(
                "Claude Code in {} runs on that account's folder and can't be restarted ({why}); close it or move it, then try again.",
                panes.join(", ")
            ),
            Self::OwnerCheckOn => "This Claude Code version ties running sessions to their account, so switching would stop them.".into(),
            Self::Stale => RULES_STALE.into(),
        }
    }

    /// The error as `tmux::accounts::global` gives it: one JSON object,
    /// `{"code", "message", "blocked_by"?}`, in the SDK's handler error.
    pub fn to_wire(&self) -> String {
        let mut body = serde_json::json!({"code": self.code(), "message": self.sentence()});
        if let Self::Blocked(panes, _) = self {
            body["blocked_by"] = serde_json::json!(panes);
        }
        body.to_string()
    }

    fn from_swap(error: SwapError, accounts: &Accounts, to: &str) -> Self {
        match error {
            SwapError::NotSignedIn => Self::NotSignedIn(moves::not_signed_in(accounts, to)),
            SwapError::Busy => Self::Busy,
            SwapError::OwnerCheckOn => Self::OwnerCheckOn,
            other => Self::Failed(other.sentence()),
        }
    }
}

/// The panes a switch from `from` to `to` deals with (G4).
struct Sorted<'a> {
    /// On the slot, not pinned.
    following: u32,
    /// On `to`'s folder.
    on_to: Vec<&'a PaneState>,
    /// On the slot, pinned to `from`.
    pinned_from: Vec<&'a PaneState>,
}

fn sort_panes<'a>(panes: &'a HashMap<String, PaneState>, from: &str, to: &str) -> Sorted<'a> {
    let to_folder = Place::Folder(to.to_string());
    let mut out = Sorted {
        following: 0,
        on_to: Vec::new(),
        pinned_from: Vec::new(),
    };
    for pane in panes.values().filter(|pane| pane.running()) {
        match &pane.place {
            Some(Place::Slot) => match pane.marks.pinned() {
                None => out.following += 1,
                Some(mark) if mark == from => out.pinned_from.push(pane),
                Some(_) => {}
            },
            Some(place) if *place == to_folder => out.on_to.push(pane),
            _ => {}
        }
    }
    out.on_to.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    out.pinned_from.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    out
}

/// The accounts the rules may not switch to now: some Claude Code on their
/// folder is working, waiting or can't be restarted (D14, G12).
pub fn unready_targets(panes: &HashMap<String, PaneState>) -> std::collections::HashSet<String> {
    panes
        .values()
        .filter(|pane| pane.running())
        .filter_map(|pane| match &pane.place {
            Some(Place::Folder(id)) if !pane.idle() || !pane.can_move() => Some(id.clone()),
            _ => None,
        })
        .collect()
}

impl Accounts {
    /// Note a swap just made, for a session that writes the old account's
    /// profile back (G3 step 8).
    fn note_swap(&self, from: &str, to: &str) {
        *self.recent_swap.lock().unwrap_or_else(|p| p.into_inner()) = Some(RecentSwap {
            from: from.to_string(),
            to: to.to_string(),
            at: std::time::Instant::now(),
        });
    }

    /// The registry as a swap leaves it (its step 7).
    fn commit_swap(&self, journal: &Journal) -> Result<(), String> {
        self.update_registry(|registry| {
            registry.global = journal.to.clone();
            if let Some(record) = &journal.record {
                registry.last_switch = Some(record.clone());
            }
            registry.pending_global = None;
            Ok(())
        })
    }

    /// Finish a swap a stopped worker left (G7). Blocking.
    pub fn recover_blocking(&self) -> Result<Option<Journal>, SwapError> {
        let Some(stores) = self.stores() else {
            return Ok(None);
        };
        self.swapping.store(true, Ordering::SeqCst);
        let done = slot::recover(&stores, &|journal| self.commit_swap(journal));
        self.swapping.store(false, Ordering::SeqCst);
        if let Ok(Some(journal)) = &done {
            self.note_swap(&journal.from, &journal.to);
            self.reread_credentials(&journal.from);
            self.reread_credentials(&journal.to);
        }
        done
    }

    /// [`Accounts::recover_blocking`] off the async threads.
    pub async fn recover(self: &Arc<Self>) -> Result<Option<Journal>, SwapError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.recover_blocking())
            .await
            .map_err(|error| SwapError::Failed(format!("The worker failed ({error}).")))?
    }

    /// Whether `id`'s folder holds a sign-in Claude Code can use, read now.
    fn folder_signed_in(&self, id: &str) -> bool {
        self.stores()
            .and_then(|stores| stores.read(&Place::Folder(id.to_string())).ok())
            .flatten()
            .as_ref()
            .map(super::credentials::credentials_of)
            .is_some_and(|credentials| credentials.signed_in())
    }

    /// G6: someone signed in by hand on the slot. Looks at `~/.claude.json`
    /// when it changed: a known account signed in there becomes global
    /// (`login`), an unknown one is added and made global (`login_new`); a
    /// session that wrote the old account's profile back right after a
    /// swap gets the new one's put back (G3 step 8). Only with
    /// `accounts.json`, never during a swap. Returns whether the registry
    /// changed. Blocking.
    pub fn watch_slot_blocking(&self) -> bool {
        let Some(stores) = self.stores() else {
            return false;
        };
        if !self.managed()
            || self.swapping.load(Ordering::SeqCst)
            || stores.read_journal().is_some()
        {
            return false;
        }
        let now_stamp = super::stamp(&stores.paths.default_json);
        {
            let mut seen = self.slot_stamp.lock().unwrap_or_else(|p| p.into_inner());
            if seen.as_ref() == Some(&now_stamp) {
                return false;
            }
            *seen = Some(now_stamp);
        }
        // Signed out by hand: the global account stays, and reads signed
        // out from its store.
        let Some(slot_uuid) = stores.uuid(&Place::Slot) else {
            return false;
        };
        let registry = self.registry();
        let global_uuid = stores.uuid(&Place::Folder(registry.global.clone()));
        if global_uuid.as_deref() == Some(slot_uuid.as_str()) {
            return false;
        }
        let recent = self
            .recent_swap
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(recent) = recent.filter(|recent| {
            recent.at.elapsed() < slot::WRITE_BACK_WINDOW && recent.to == registry.global
        }) {
            if stores.uuid(&Place::Folder(recent.from.clone())).as_deref() == Some(&slot_uuid) {
                tracing::info!(from = %recent.from, to = %recent.to, "a session wrote the old account's profile back; the new one is put back");
                if let Err(error) = slot::swap_records(&stores, &recent.from, &recent.to, false) {
                    tracing::warn!(error = %error.sentence(), "the new account's profile was not put back");
                }
                return false;
            }
        }
        let known = registry
            .accounts
            .iter()
            .find(|entry| {
                stores.uuid(&Place::Folder(entry.id.clone())).as_deref() == Some(&slot_uuid)
            })
            .map(|entry| entry.id.clone());
        if known.is_none() && global_uuid.is_none() {
            // The global account's folder has no record yet (it was signed
            // out when it was made): the slot's is its own.
            let _ = slot::update_config(
                &stores.json(&Place::Folder(registry.global.clone())),
                |config| {
                    let slot = slot::read_config(&stores.paths.default_json).ok().flatten();
                    match slot.and_then(|slot| slot.get(slot::IDENTITY_KEY).cloned()) {
                        Some(record) => {
                            config.insert(slot::IDENTITY_KEY.into(), record);
                            true
                        }
                        None => false,
                    }
                },
            );
            return false;
        }
        let at_ms = now_ms();
        let from = registry.global.clone();
        let changed = match known {
            Some(id) => self.update_registry(|registry| {
                tracing::info!(account = %id, "signed in by hand on the slot: it is global now");
                registry.last_switch = Some(login_record(&registry.global, &id, "login", at_ms));
                registry.global = id.clone();
                registry.pending_global = None;
                Ok(())
            }),
            None => self.add_from_slot(&stores, at_ms),
        };
        match changed {
            Ok(()) => {
                self.reread_credentials(&from);
                let global = self.registry().global;
                self.reread_credentials(&global);
                true
            }
            Err(error) => {
                tracing::warn!(%error, "a sign-in by hand on the slot was not taken in");
                // Looked at again on the next change or start.
                *self.slot_stamp.lock().unwrap_or_else(|p| p.into_inner()) = None;
                false
            }
        }
    }

    /// G6 for an account nobody knows: added (name from the profile, the
    /// next free colour, everything shared, its folder seeded with the
    /// slot's record) and made global.
    fn add_from_slot(&self, stores: &slot::Stores<'_>, at_ms: u64) -> Result<(), String> {
        let config = slot::read_config(&stores.paths.default_json)?.unwrap_or_default();
        let record = config.get(slot::IDENTITY_KEY);
        let text = |key: &str| {
            record
                .and_then(|record| record.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let base = text("displayName")
            .or_else(|| {
                text("emailAddress").and_then(|email| email.split('@').next().map(str::to_string))
            })
            .unwrap_or_else(|| "Account".to_string());
        let paths = stores.paths.clone();
        self.update_registry(|registry| {
            let mut name: String = base.chars().take(40).collect();
            let mut n = 2;
            while registry
                .accounts
                .iter()
                .any(|entry| entry.name.to_lowercase() == name.to_lowercase())
            {
                let suffix = format!(" {n}");
                name = format!(
                    "{}{suffix}",
                    base.chars().take(40 - suffix.len()).collect::<String>()
                );
                n += 1;
            }
            let id = registry.new_id(&name, |id| {
                std::fs::symlink_metadata(paths.account_dir(id)).is_ok()
            });
            let used: Vec<&str> = registry.accounts.iter().map(|e| e.color.as_str()).collect();
            let color = registry::COLORS
                .iter()
                .find(|color| !used.contains(color))
                .unwrap_or(&registry::COLORS[registry.accounts.len() % registry::COLORS.len()])
                .to_string();
            let groups: Vec<String> = super::shared::GROUPS.iter().map(|g| g.to_string()).collect();
            slot::seed_folder(&paths, &id, &groups, true)?;
            tracing::info!(account = %id, "signed in by hand on the slot to a new account: added, global");
            registry.accounts.push(registry::Entry {
                id: id.clone(),
                name,
                color,
                shared: groups,
                created_at_ms: at_ms,
            });
            registry.last_switch = Some(login_record(&registry.global, &id, "login_new", at_ms));
            registry.global = id;
            registry.pending_global = None;
            Ok(())
        })
    }

    /// [`Accounts::watch_slot_blocking`] off the async threads.
    pub async fn watch_slot(self: &Arc<Self>) -> bool {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.watch_slot_blocking())
            .await
            .unwrap_or(false)
    }
}

fn login_record(from: &str, to: &str, reason: &str, at_ms: u64) -> AccountSwitch {
    AccountSwitch {
        from: from.to_string(),
        to: to.to_string(),
        reason: Some(reason.to_string()),
        auto: false,
        following: 0,
        restarted: Vec::new(),
        after_turn: Vec::new(),
        at_ms,
        back_at_ms: None,
    }
}

/// Put the stopped panes back where they were.
async fn put_back(tmux: &Tmux, accounts: &Accounts, stopped: Vec<Stopped>) {
    for pane in stopped {
        let id = pane.pane_id.clone();
        if let Err(error) = pane.start_where_it_was(tmux).await {
            tracing::warn!(pane = %id, %error, "Claude Code not started again where it was");
            accounts.note_move_error(&id, &error);
        }
    }
}

/// Make `to` the global account (G3, G4). `panes` is a fresh collection.
/// Waits for no window but by `pending_global`; one switch at a time.
pub async fn make_global(
    tmux: &Tmux,
    accounts: &Arc<Accounts>,
    panes: &HashMap<String, PaneState>,
    to: &str,
    by: By,
) -> Result<Outcome, SwitchError> {
    let to = to.trim().to_string();
    let _gate = accounts.swap_gate.lock().await;
    match accounts.recover().await {
        Ok(_) => {}
        Err(SwapError::Busy) => return Err(SwitchError::Busy),
        Err(error) => tracing::warn!(error = %error.sentence(), "a stopped swap was not finished"),
    }
    // Someone may have signed in by hand: the global account is whoever
    // the slot holds (G6), before anything is decided.
    accounts.watch_slot().await;
    let (registry, generation) = accounts.registry_at();
    if !registry.has(&to) {
        return Err(SwitchError::Failed(super::manage::no_account(&to)));
    }
    if let By::Rules {
        generation: decided_on,
        ..
    } = &by
    {
        if generation != *decided_on {
            return Err(SwitchError::Stale);
        }
    }
    let from = registry.global.clone();
    let sorted = sort_panes(panes, &from, &to);
    if to == from {
        // The same account again: nothing moves, a waiting switch is off,
        // and so is a switch back the rules planned.
        let person = by == By::Person;
        accounts
            .update_registry_at_async(|registry, _| {
                registry.pending_global = None;
                if person {
                    if let Some(last) = registry.last_switch.as_mut() {
                        last.back_at_ms = None;
                    }
                }
                Ok(())
            })
            .await
            .map_err(SwitchError::Failed)?;
        return Ok(Outcome {
            global: from,
            following: sorted.following,
            ..Outcome::default()
        });
    }
    {
        let this = accounts.clone();
        let id = to.clone();
        let signed_in = tokio::task::spawn_blocking(move || this.folder_signed_in(&id))
            .await
            .unwrap_or(false);
        if !signed_in {
            return Err(SwitchError::NotSignedIn(moves::not_signed_in(
                accounts, &to,
            )));
        }
    }
    if let Some(stores) = accounts.stores() {
        let config = slot::read_config(&stores.paths.default_json).ok().flatten();
        if slot::owner_gate_on(config.as_ref()) {
            return Err(SwitchError::OwnerCheckOn);
        }
    }
    // G4: the new account's folder must be free.
    let blocked: Vec<&PaneState> = sorted
        .on_to
        .iter()
        .copied()
        .filter(|p| !p.can_move())
        .collect();
    if let Some(first) = blocked.first() {
        return Err(SwitchError::Blocked(
            blocked.iter().map(|p| p.pane_id.clone()).collect(),
            first.why_not_movable(),
        ));
    }
    let busy: Vec<&PaneState> = sorted
        .on_to
        .iter()
        .copied()
        .filter(|pane| !pane.idle())
        .collect();
    if !busy.is_empty() {
        return wait(accounts, &registry, &to, &by, busy, sorted.following).await;
    }
    // Stop what must move, then swap, then start them again.
    let mut stopped_to = Vec::new();
    for pane in &sorted.on_to {
        match moves::stop(tmux, accounts, pane).await {
            Ok(stopped) => stopped_to.push(stopped),
            Err(error) => {
                put_back(tmux, accounts, stopped_to).await;
                if error == NOT_IDLE {
                    return wait(accounts, &registry, &to, &by, vec![*pane], sorted.following)
                        .await;
                }
                return Err(SwitchError::Blocked(vec![pane.pane_id.clone()], error));
            }
        }
    }
    let mut stopped_from = Vec::new();
    let mut after_turn = Vec::new();
    for pane in &sorted.pinned_from {
        if !pane.can_move() {
            // It goes on with the new account on the slot.
            accounts.note_move_error(&pane.pane_id, &pane.why_not_movable());
            continue;
        }
        if !pane.idle() {
            after_turn.push(*pane);
            continue;
        }
        match moves::stop(tmux, accounts, pane).await {
            Ok(stopped) => stopped_from.push(stopped),
            Err(error) if error == NOT_IDLE => after_turn.push(*pane),
            Err(error) => accounts.note_move_error(&pane.pane_id, &error),
        }
    }
    let restarted: Vec<String> = stopped_to
        .iter()
        .chain(stopped_from.iter())
        .map(|stopped| stopped.pane_id.clone())
        .collect();
    let at_ms = now_ms();
    let record = match &by {
        By::Rules { record, .. } => AccountSwitch {
            following: sorted.following,
            restarted: restarted.clone(),
            after_turn: after_turn.iter().map(|p| p.pane_id.clone()).collect(),
            ..record.clone()
        },
        By::Person | By::Pending(_) => AccountSwitch {
            from: from.clone(),
            to: to.clone(),
            reason: None,
            auto: by.word() == "rule",
            following: sorted.following,
            restarted: restarted.clone(),
            after_turn: after_turn.iter().map(|p| p.pane_id.clone()).collect(),
            at_ms,
            back_at_ms: None,
        },
    };
    let swapped = {
        let this = accounts.clone();
        let (from, to) = (from.clone(), to.clone());
        tokio::task::spawn_blocking(move || {
            let Some(stores) = this.stores() else {
                return Err(SwapError::Failed(
                    "No home folder for Claude Code accounts.".into(),
                ));
            };
            this.swapping.store(true, Ordering::SeqCst);
            let done = slot::swap(
                &stores,
                &from,
                &to,
                Some(record),
                &|journal| this.commit_swap(journal),
                Crash::None,
            );
            this.swapping.store(false, Ordering::SeqCst);
            done
        })
        .await
        .map_err(|error| SwapError::Failed(format!("The worker failed ({error}).")))
        .and_then(|done| done)
    };
    if let Err(error) = swapped {
        put_back(tmux, accounts, stopped_to).await;
        put_back(tmux, accounts, stopped_from).await;
        if let SwapError::Identity(_) = error {
            accounts.watch_slot().await;
        }
        return Err(SwitchError::from_swap(error, accounts, &to));
    }
    tracing::info!(%from, %to, by = %by.word(), "the global account switched");
    accounts.note_swap(&from, &to);
    accounts.reread_credentials(&from);
    accounts.reread_credentials(&to);
    // The new account's panes, on the slot now, still pinned to it.
    for stopped in stopped_to {
        restart(tmux, accounts, stopped, &Place::Slot, &to).await;
    }
    // The old account's pinned panes, on its folder now.
    let from_folder = Place::Folder(from.clone());
    for stopped in stopped_from {
        restart(tmux, accounts, stopped, &from_folder, &from).await;
    }
    let mut waiting = Vec::new();
    for pane in after_turn {
        let id = pane.pane_id.clone();
        match moves::set_marks(tmux, &id, &from, pane.info.skip_permissions, Some(&from)).await {
            Ok(()) => {
                accounts.note_waiting(&id, pane.claude_pid);
                waiting.push(id);
            }
            Err(error) => accounts.note_move_error(&id, &error),
        }
    }
    Ok(Outcome {
        global: to,
        waiting: false,
        following: sorted.following,
        restarted,
        after_turn: waiting,
        waiting_for: Vec::new(),
    })
}

/// Start a stopped pane again on `place`, pinned to `account`, its
/// conversation made visible there; where it was when that fails.
async fn restart(tmux: &Tmux, accounts: &Accounts, stopped: Stopped, place: &Place, account: &str) {
    let id = stopped.pane_id.clone();
    let command = match accounts.restart_command(place, stopped.skip, Some(&stopped.session)) {
        Ok(command) => command,
        Err(error) => {
            accounts.note_move_error(&id, &error);
            let _ = stopped.start_where_it_was(tmux).await;
            return;
        }
    };
    let started = match stopped.share_with(accounts, place).await {
        Ok(_) => stopped.start(tmux, &command, Some(account)).await,
        Err(error) => {
            let _ = stopped.start_where_it_was(tmux).await;
            Err(error)
        }
    };
    match started {
        Ok(()) => accounts.clear_move_error(&id),
        Err(error) => {
            tracing::warn!(pane = %id, %error, "pane not started again after the switch");
            accounts.note_move_error(&id, &error);
        }
    }
}

/// The switch waits for `busy` (G4): `pending_global` says so, replacing
/// any other. The rules never wait.
async fn wait(
    accounts: &Arc<Accounts>,
    registry: &Registry,
    to: &str,
    by: &By,
    busy: Vec<&PaneState>,
    following: u32,
) -> Result<Outcome, SwitchError> {
    if matches!(by, By::Rules { .. }) {
        return Err(SwitchError::Failed(
            "Claude Code is working on that account's folder; the rules try another.".into(),
        ));
    }
    let pending = PendingSwitch {
        account: to.to_string(),
        by: by.word(),
        at_ms: match by {
            By::Pending(_) => registry
                .pending_global
                .as_ref()
                .map(|pending| pending.at_ms)
                .unwrap_or_else(now_ms),
            _ => now_ms(),
        },
    };
    accounts
        .update_registry_at_async(move |registry, _| {
            registry.pending_global = Some(pending);
            Ok(())
        })
        .await
        .map_err(SwitchError::Failed)?;
    accounts.note_global_waiting(Some(busy.iter().map(|pane| pane.claude_pid).collect()));
    Ok(Outcome {
        global: registry.global.clone(),
        waiting: true,
        following,
        restarted: Vec::new(),
        after_turn: Vec::new(),
        waiting_for: busy.iter().map(|pane| pane.pane_id.clone()).collect(),
    })
}

/// The worker's side of a switch that waits (G4): once every Claude Code
/// on the new account's folder is idle, the switch happens. One that can't
/// be restarted, or an account no longer signed in, calls it off (each pane
/// in the way says why). Returns whether anything changed.
pub async fn finish_pending(
    tmux: &Tmux,
    accounts: &Arc<Accounts>,
    panes: &HashMap<String, PaneState>,
) -> bool {
    let registry = accounts.registry();
    let Some(pending) = registry.pending_global.clone() else {
        accounts.note_global_waiting(None);
        return false;
    };
    let sorted = sort_panes(panes, &registry.global, &pending.account);
    let busy: Vec<Option<u32>> = sorted
        .on_to
        .iter()
        .filter(|pane| !pane.idle())
        .map(|pane| pane.claude_pid)
        .collect();
    if !busy.is_empty() {
        accounts.note_global_waiting(Some(busy));
        return false;
    }
    match make_global(
        tmux,
        accounts,
        panes,
        &pending.account,
        By::Pending(pending.by.clone()),
    )
    .await
    {
        Ok(outcome) => {
            if !outcome.waiting {
                accounts.note_global_waiting(None);
            }
            !outcome.waiting
        }
        Err(error) => {
            tracing::warn!(account = %pending.account, error = %error.sentence(), "the switch that waited was called off");
            if let SwitchError::Blocked(panes, _) = &error {
                for pane in panes {
                    accounts.note_move_error(pane, &error.sentence());
                }
            }
            let calls_off = !matches!(error, SwitchError::Busy);
            if calls_off {
                accounts.note_global_waiting(None);
                let _ = accounts
                    .update_registry_at_async(|registry, _| {
                        if registry
                            .pending_global
                            .as_ref()
                            .is_some_and(|p| p.account == pending.account)
                        {
                            registry.pending_global = None;
                        }
                        Ok(())
                    })
                    .await;
            }
            calls_off
        }
    }
}

/// G11 on tmux, once the registry was brought to the slot: the folder
/// 0.3.0 put in tmux's global environment goes, and panes marked `global`
/// that run on a folder (they followed it there) are pinned to that
/// folder's account. Safe to run again: it only undoes what 0.3.0 left.
pub async fn migrate_tmux(tmux: &Tmux, accounts: &Accounts, panes: &HashMap<String, PaneState>) {
    let Some(root) = accounts.paths().map(|paths| paths.root.clone()) else {
        return;
    };
    if let Err(error) = super::tmux_env::unset_old_global(tmux, &root).await {
        tracing::debug!(%error, "tmux global environment not looked at");
    }
    for pane in panes.values().filter(|pane| pane.running()) {
        let (Some(super::GLOBAL), None, Some(Place::Folder(id))) = (
            pane.marks.account.as_deref(),
            pane.marks.pending.as_deref(),
            &pane.place,
        ) else {
            continue;
        };
        if pane.marks_shared {
            continue;
        }
        match moves::set_marks(tmux, &pane.pane_id, id, pane.info.skip_permissions, None).await {
            Ok(()) => {
                tracing::info!(pane = %pane.pane_id, account = %id, "pinned where 0.3.0 left it")
            }
            Err(error) => tracing::debug!(pane = %pane.pane_id, %error, "pane not pinned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_a_stable_code() {
        let blocked = SwitchError::Blocked(vec!["%4".into(), "%9".into()], "nested".into());
        let wire: serde_json::Value = serde_json::from_str(&blocked.to_wire()).unwrap();
        assert_eq!(wire["code"], "blocked");
        assert_eq!(wire["blocked_by"], serde_json::json!(["%4", "%9"]));
        assert!(wire["message"].as_str().unwrap().contains("%4, %9"));
        for (error, code) in [
            (SwitchError::Busy, "busy"),
            (SwitchError::OwnerCheckOn, "owner_check_on"),
            (SwitchError::NotSignedIn("x".into()), "not_signed_in"),
        ] {
            let wire: serde_json::Value = serde_json::from_str(&error.to_wire()).unwrap();
            assert_eq!(wire["code"], code);
            assert!(wire.get("blocked_by").is_none());
        }
    }
}
