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
#[cfg(test)]
use super::{shared, DEFAULT as DEFAULT_ID};
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
    /// The worker, finishing the switch that waited (`by`: `person` or
    /// `rule`) since `at_ms`: it goes on only while `pending_global` is
    /// still that very one.
    Pending { by: String, at_ms: u64 },
}

impl By {
    fn word(&self) -> String {
        match self {
            Self::Person => "person".into(),
            Self::Rules { .. } => "rule".into(),
            Self::Pending { by, .. } => by.clone(),
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
    /// A sign-in from the app runs where the switch would move sign-ins
    /// (the slot, or the two accounts' folders): the account's name.
    SigningIn(String),
    Failed(String),
}

/// What waits for a swap left half-way (G7) that could not be settled yet.
pub const SWAP_UNSETTLED: &str =
    "A switch of the global account left half-way is being settled; try again in a moment.";

pub const RULES_STALE: &str =
    "The global account changed while the rules decided; nothing was switched.";

impl SwitchError {
    /// `not_signed_in`, `busy`, `blocked`, `owner_check_on`, `stale`,
    /// `signing_in` or `failed`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotSignedIn(_) => "not_signed_in",
            Self::Busy => "busy",
            Self::Blocked(..) => "blocked",
            Self::OwnerCheckOn => "owner_check_on",
            Self::Stale => "stale",
            Self::SigningIn(_) => "signing_in",
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
            Self::SigningIn(name) => format!(
                "{name} is signing in; finish or cancel that sign-in, then switch."
            ),
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
    /// Note a swap just made, with the chain it wrote into the slot, for a
    /// session that writes the old account's profile back (G3 step 8).
    fn note_swap(&self, journal: &Journal) {
        *self.recent_swap.lock().unwrap_or_else(|p| p.into_inner()) = Some(RecentSwap {
            from: journal.from.clone(),
            to: journal.to.clone(),
            at: std::time::Instant::now(),
            to_chains: journal.x_rt.iter().cloned().collect(),
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
        })?;
        self.note_swap(journal);
        Ok(())
    }

    /// The swap made last, while a session may still write the old
    /// profile back over it.
    fn recent_swap(&self) -> Option<RecentSwap> {
        self.recent_swap
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .filter(|recent| recent.at.elapsed() < slot::WRITE_BACK_WINDOW)
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
            self.reread_credentials(&journal.from);
            self.reread_credentials(&journal.to);
        }
        done
    }

    /// The account a sign-in by hand signed out, once (see `after_login`).
    fn take_login_release(&self) -> Option<String> {
        self.login_release
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    /// Whether a sign-in by hand left panes to follow the new global
    /// account (`after_login`).
    pub fn login_release_due(&self) -> bool {
        self.login_release
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    /// Whether a swap left its journal (one that could not be settled
    /// yet, or one running now).
    pub fn has_journal(&self) -> bool {
        self.stores()
            .is_some_and(|stores| stores.read_journal().is_some())
    }

    /// A swap left half-way that one more try could not settle (G7).
    /// Until it is, the registry may not name the account the slot holds,
    /// and the folders of the two accounts it moves may hold neither
    /// sign-in: nothing starts or moves on them.
    pub async fn unsettled_swap(self: &Arc<Self>) -> Option<Journal> {
        if !self.has_journal() {
            return None;
        }
        if let Err(error) = self.recover().await {
            tracing::debug!(error = %error.sentence(), "a switch left half-way is not settled yet");
        }
        self.stores().and_then(|stores| stores.read_journal())
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
        self.place_signed_in(&Place::Folder(id.to_string()))
    }

    /// Whether `place` holds a sign-in Claude Code can use, read now.
    pub(super) fn place_signed_in(&self, place: &Place) -> bool {
        self.stores()
            .and_then(|stores| stores.read(place).ok())
            .flatten()
            .as_ref()
            .map(super::credentials::credentials_of)
            .is_some_and(|credentials| credentials.signed_in())
    }

    /// What tells the sign-in `place` holds now from another one
    /// ([`super::credentials::sign_in_mark`]); `None` without one.
    pub(super) fn place_mark(&self, place: &Place) -> Option<String> {
        self.stores()
            .and_then(|stores| stores.read(place).ok())
            .flatten()
            .as_ref()
            .and_then(super::credentials::sign_in_mark)
    }

    /// Whether the registry names whoever is signed in to the slot as the
    /// global account (always, without `accounts.json`): what a sign-in on
    /// the slot waits for once [`Accounts::watch_slot`] looked.
    pub(super) fn slot_is_global(&self) -> bool {
        if !self.managed() {
            return true;
        }
        let Some(stores) = self.stores() else {
            return false;
        };
        let global = self.registry().global;
        let slot = stores.uuid(&Place::Slot);
        slot.is_some() && slot == stores.uuid(&Place::Folder(global))
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
        // Both files: a sign-in by hand rewrites them, and a renewal the
        // credentials alone (noted below for a swap just made).
        let now_stamp = (
            super::stamp(&stores.paths.default_json),
            super::stamp(
                &stores
                    .paths
                    .claude_home
                    .join(super::credentials::CREDENTIALS_FILE),
            ),
        );
        {
            let mut seen = self.slot_stamp.lock().unwrap_or_else(|p| p.into_inner());
            if seen.as_ref() == Some(&now_stamp) {
                drop(seen);
                // On macOS the slot's sign-in is in the Keychain, whose
                // renewals change no file: right after a swap its chain is
                // looked at anyway.
                self.note_renewal(&stores);
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
            self.note_renewal(&stores);
            return false;
        }
        if let Some(recent) = self
            .recent_swap()
            .filter(|recent| recent.to == registry.global)
        {
            if stores.uuid(&Place::Folder(recent.from.clone())).as_deref() == Some(&slot_uuid) {
                // The old account's profile over the chain the swap wrote (or
                // one it was renewed to): a session that fetched the profile
                // just before wrote it back. Over another chain: someone
                // signed in to the old account by hand (G6, P23). `/login`
                // writes the profile before the sign-in, so the chain is
                // read again once that had time to land.
                let stale = self.slot_chain_is(&stores, &recent.to_chains) && {
                    std::thread::sleep(super::LOGIN_SETTLE);
                    self.slot_chain_is(&stores, &recent.to_chains)
                };
                if stale {
                    tracing::info!(from = %recent.from, to = %recent.to, "a session wrote the old account's profile back; the new one is put back");
                    if let Err(error) = slot::swap_records(&stores, &recent.from, &recent.to, false)
                    {
                        tracing::warn!(error = %error.sentence(), "the new account's profile was not put back");
                    }
                    return false;
                }
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
        // A sign-in by hand ends the swap's watch for a write-back.
        *self.recent_swap.lock().unwrap_or_else(|p| p.into_inner()) = None;
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
                if global != from {
                    // Its pinned panes on the slot follow the new account
                    // (`after_login`): it is signed out now.
                    *self.login_release.lock().unwrap_or_else(|p| p.into_inner()) = Some(from);
                }
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
    /// Whether the slot's sign-in is one of `chains`; unreadable counts as
    /// yes (the write-back changes no sign-in), signed out as no.
    fn slot_chain_is(&self, stores: &slot::Stores<'_>, chains: &[String]) -> bool {
        match stores.read(&Place::Slot) {
            Ok(raw) => super::credentials::chain_id(raw.as_ref())
                .is_some_and(|chain| chains.contains(&chain)),
            Err(_) => true,
        }
    }

    /// Right after a swap, a chain Claude Code renewed in the slot is still
    /// the new account's: noted while the slot has its profile, so the old
    /// profile written back over it is told from a sign-in by hand (P23).
    fn note_renewal(&self, stores: &slot::Stores<'_>) {
        let Some(recent) = self.recent_swap() else {
            return;
        };
        let Some(slot_uuid) = stores.uuid(&Place::Slot) else {
            return;
        };
        if stores.uuid(&Place::Folder(recent.to.clone())).as_deref() != Some(slot_uuid.as_str()) {
            return;
        }
        let chain_now = || {
            stores
                .read(&Place::Slot)
                .ok()
                .and_then(|raw| super::credentials::chain_id(raw.as_ref()))
        };
        let Some(chain) = chain_now() else {
            return;
        };
        if recent.to_chains.contains(&chain) {
            return;
        }
        // A sign-in by hand may write its chain before its profile: a new
        // chain is the new account's only once the profile is still its
        // own a moment later.
        std::thread::sleep(super::LOGIN_SETTLE);
        if stores.uuid(&Place::Slot).as_deref() != Some(slot_uuid.as_str())
            || chain_now().as_ref() != Some(&chain)
        {
            return;
        }
        let mut kept = self.recent_swap.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(kept) = kept
            .as_mut()
            .filter(|kept| kept.to == recent.to && kept.at == recent.at)
        {
            if !kept.to_chains.contains(&chain) {
                tracing::info!(account = %kept.to, "the new global account's sign-in was renewed");
                kept.to_chains.push(chain);
            }
        }
    }

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

/// Put the stopped panes back where they were: the swap did not happen,
/// so each one's account is still where it ran.
async fn put_back(tmux: &Tmux, accounts: &Accounts, stopped: Vec<Stopped>) {
    for pane in stopped {
        let id = pane.pane_id.clone();
        if let Err(error) = pane.start_where_it_was(tmux).await {
            tracing::warn!(pane = %id, %error, "Claude Code not started again where it was");
            accounts.note_move_error(&id, &error);
        }
    }
}

/// A name for account `id` in a sentence.
fn name_of(accounts: &Accounts, id: &str) -> String {
    accounts
        .registry()
        .get(id)
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// After a sign-in by hand on the slot (G6), the panes there pinned to the
/// account that was global: it is signed out now (its only sign-in was the
/// slot's, G2), so they can't go to its folder. They follow the global
/// account instead, and `last_switch.following` counts them with the
/// others. Returns the panes changed.
pub async fn after_login(
    tmux: &Tmux,
    accounts: &Accounts,
    panes: &HashMap<String, PaneState>,
) -> Vec<String> {
    let Some(old) = accounts.take_login_release() else {
        return Vec::new();
    };
    let mut pinned: Vec<&PaneState> = panes
        .values()
        .filter(|pane| pane.running() && pane.place == Some(Place::Slot))
        .filter(|pane| pane.marks.pinned() == Some(old.as_str()))
        .collect();
    pinned.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    let mut released = Vec::new();
    for pane in pinned {
        if accounts.moving.is_busy(&pane.pane_id) {
            continue;
        }
        match moves::set_marks(
            tmux,
            &pane.pane_id,
            super::GLOBAL,
            pane.info.skip_permissions,
            None,
        )
        .await
        {
            Ok(()) => {
                accounts.forget_waiting(&pane.pane_id);
                released.push(pane.pane_id.clone());
            }
            Err(error) => accounts.note_move_error(&pane.pane_id, &error),
        }
    }
    let following = panes
        .values()
        .filter(|pane| pane.running() && pane.place == Some(Place::Slot))
        .filter(|pane| pane.marks.pinned().is_none() || released.contains(&pane.pane_id))
        .count() as u32;
    let _ = accounts
        .update_registry_at_async(move |registry, _| {
            if let Some(last) = registry.last_switch.as_mut().filter(|last| {
                last.from == old && matches!(last.reason.as_deref(), Some("login" | "login_new"))
            }) {
                last.following = following;
            }
            Ok(())
        })
        .await;
    if !released.is_empty() {
        tracing::info!(panes = ?released, "pinned to the account signed out by a sign-in by hand: they follow now");
    }
    released
}

/// `panes` with `released` following the global account (their marks as
/// [`after_login`] left them).
pub fn as_followers(
    panes: &HashMap<String, PaneState>,
    released: &[String],
) -> HashMap<String, PaneState> {
    let mut panes = panes.clone();
    for id in released {
        if let Some(pane) = panes.get_mut(id) {
            pane.marks.account = Some(super::GLOBAL.to_string());
            pane.marks.pending = None;
        }
    }
    panes
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
    let gate = accounts.gate().await;
    // The work owns the gate: a caller that goes away mid-switch (its
    // future dropped) neither stops the switch half-way nor frees the gate
    // while the swap runs.
    let (tmux, accounts, panes, to) = (
        tmux.clone(),
        accounts.clone(),
        panes.clone(),
        to.to_string(),
    );
    tokio::spawn(async move {
        let _gate = gate;
        switch_locked(&tmux, &accounts, &panes, &to, by).await
    })
    .await
    .unwrap_or_else(|error| Err(SwitchError::Failed(format!("The worker failed ({error})."))))
}

/// [`make_global`] for a caller that holds `swap_gate` already, with
/// `panes` collected under it.
pub async fn switch_locked(
    tmux: &Tmux,
    accounts: &Arc<Accounts>,
    panes: &HashMap<String, PaneState>,
    to: &str,
    by: By,
) -> Result<Outcome, SwitchError> {
    let to = to.trim().to_string();
    match accounts.recover().await {
        Ok(_) => {}
        Err(SwapError::Busy) => return Err(SwitchError::Busy),
        Err(error) => tracing::warn!(error = %error.sentence(), "a stopped swap was not finished"),
    }
    // Someone may have signed in by hand: the global account is whoever
    // the slot holds (G6), before anything is decided.
    accounts.watch_slot().await;
    let released = after_login(tmux, accounts, panes).await;
    let owned;
    let panes = if released.is_empty() {
        panes
    } else {
        owned = as_followers(panes, &released);
        &owned
    };
    if accounts.has_journal() {
        // A swap left half-way that could not be settled yet.
        return Err(SwitchError::Busy);
    }
    let (registry, generation) = accounts.registry_at();
    if !registry.has(&to) {
        return Err(SwitchError::Failed(super::manage::no_account(&to)));
    }
    match &by {
        By::Rules {
            generation: decided_on,
            ..
        } if generation != *decided_on => return Err(SwitchError::Stale),
        // The switch that waited goes on only as it was asked: someone may
        // have picked another account or called it off meanwhile.
        By::Pending { at_ms, .. }
            if !registry
                .pending_global
                .as_ref()
                .is_some_and(|pending| pending.account == to && pending.at_ms == *at_ms) =>
        {
            return Err(SwitchError::Stale)
        }
        _ => {}
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
    // A sign-in from the app on the slot or on either folder: Claude Code
    // is about to save one where the swap moves them.
    if let Some(account) = accounts.logins.under_way_on(&[from.as_str(), to.as_str()]) {
        return Err(SwitchError::SigningIn(name_of(accounts, &account)));
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
                if passes(&error) {
                    return Err(SwitchError::Failed(error));
                }
                return Err(SwitchError::Blocked(vec![pane.pane_id.clone()], error));
            }
        }
    }
    let mut stopped_from = Vec::new();
    let mut after_turn = Vec::new();
    // Pinned to the old account but can't be moved to its folder: they go
    // on with the new account on the slot, so they follow it from now on.
    let mut stay_on_slot: Vec<(&PaneState, String)> = Vec::new();
    for pane in &sorted.pinned_from {
        if !pane.can_move() {
            stay_on_slot.push((pane, pane.why_not_movable()));
            continue;
        }
        if !pane.idle() {
            after_turn.push(*pane);
            continue;
        }
        match moves::stop(tmux, accounts, pane).await {
            Ok(stopped) => stopped_from.push(stopped),
            Err(error) if error == NOT_IDLE => after_turn.push(*pane),
            // Started again once it has ended, or once its other move is
            // over, where its account is by then.
            Err(error) if error == moves::DID_NOT_STOP || error == moves::BUSY => {
                accounts.note_move_error(&pane.pane_id, &error)
            }
            Err(error) => stay_on_slot.push((pane, error)),
        }
    }
    let following = sorted.following + stay_on_slot.len() as u32;
    let planned: Vec<String> = stopped_to
        .iter()
        .chain(stopped_from.iter())
        .map(|stopped| stopped.pane_id.clone())
        .collect();
    let at_ms = now_ms();
    let record = match &by {
        By::Rules { record, .. } => AccountSwitch {
            following,
            restarted: planned.clone(),
            after_turn: after_turn.iter().map(|p| p.pane_id.clone()).collect(),
            ..record.clone()
        },
        By::Person | By::Pending { .. } => AccountSwitch {
            from: from.clone(),
            to: to.clone(),
            reason: None,
            auto: by.word() == "rule",
            following,
            restarted: planned.clone(),
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
    let failed = match swapped {
        Ok(()) => None,
        Err(error) => {
            // The new account's chain in the slot already (a later step
            // failed): finished now when it can be, else on the next look.
            // Its panes belong on the slot either way, never on its folder,
            // whose chain is in the slot too.
            let this = accounts.clone();
            let x_in_slot = tokio::task::spawn_blocking(move || {
                this.stores()
                    .is_some_and(|stores| slot::holds_new_chain(&stores))
            })
            .await
            .unwrap_or(false);
            if !x_in_slot {
                put_back(tmux, accounts, stopped_to).await;
                put_back(tmux, accounts, stopped_from).await;
                if let SwapError::Identity(_) = error {
                    accounts.watch_slot().await;
                }
                return Err(SwitchError::from_swap(error, accounts, &to));
            }
            match accounts.recover().await {
                Ok(Some(_)) => None,
                _ => Some(SwitchError::from_swap(error, accounts, &to)),
            }
        }
    };
    if failed.is_none() {
        tracing::info!(%from, %to, by = %by.word(), "the global account switched");
    }
    accounts.reread_credentials(&from);
    accounts.reread_credentials(&to);
    // The new account's panes, on the slot now, still pinned to it.
    let mut restarted = Vec::new();
    for stopped in stopped_to {
        let id = stopped.pane_id.clone();
        if restart(tmux, accounts, stopped, &Place::Slot, &to).await {
            restarted.push(id);
        }
    }
    // The old account's pinned panes, on its folder now.
    let from_folder = Place::Folder(from.clone());
    for stopped in stopped_from {
        let id = stopped.pane_id.clone();
        if restart(tmux, accounts, stopped, &from_folder, &from).await {
            restarted.push(id);
        }
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
    let (from_name, to_name) = (name_of(accounts, &from), name_of(accounts, &to));
    for (pane, why) in stay_on_slot {
        let id = pane.pane_id.clone();
        let note = format!(
            "{from_name} is no longer global and this Claude Code could not be moved to its folder ({why}); it follows the global account ({to_name}) now."
        );
        match moves::set_marks(tmux, &id, super::GLOBAL, pane.info.skip_permissions, None).await {
            Ok(()) => {
                accounts.forget_waiting(&id);
                accounts.note_move_error(&id, &note);
            }
            Err(error) => accounts.note_move_error(&id, &format!("{note} {error}")),
        }
    }
    if let Some(error) = failed {
        return Err(error);
    }
    if restarted.len() != planned.len() {
        let started = restarted.clone();
        let _ = accounts
            .update_registry_at_async(move |registry, _| {
                if let Some(last) = registry.last_switch.as_mut() {
                    last.restarted = started;
                }
                Ok(())
            })
            .await;
    }
    Ok(Outcome {
        global: to,
        waiting: false,
        following,
        restarted,
        after_turn: waiting,
        waiting_for: Vec::new(),
    })
}

/// Start a stopped pane again on `place`, where its account is after the
/// swap, pinned to `account`: never where it ran, whose sign-in moved. Its
/// conversation is resumed when `place` can see it; otherwise a new one
/// starts there and its `move_error` says so. Returns whether it started.
async fn restart(
    tmux: &Tmux,
    accounts: &Accounts,
    stopped: Stopped,
    place: &Place,
    account: &str,
) -> bool {
    let id = stopped.pane_id.clone();
    let shared = stopped.share_with(accounts, place).await;
    let resume = shared.is_ok().then(|| stopped.session.clone());
    let command = match accounts.restart_command(place, stopped.skip, resume.as_deref()) {
        Ok(command) => command,
        Err(error) => {
            let error =
                format!("Claude Code was stopped for the switch and not started again: {error}");
            tracing::warn!(pane = %id, %error, "pane not started again after the switch");
            accounts.note_move_error(&id, &error);
            stopped.leave(tmux).await;
            return false;
        }
    };
    match stopped.start(tmux, &command, Some(account)).await {
        Ok(()) => {
            match shared {
                Ok(_) => accounts.clear_move_error(&id),
                Err(why) => accounts.note_move_error(
                    &id,
                    &format!(
                        "{why} Claude Code started again on its account with a new conversation."
                    ),
                ),
            }
            true
        }
        Err(error) => {
            tracing::warn!(pane = %id, %error, "pane not started again after the switch");
            accounts.note_move_error(&id, &error);
            false
        }
    }
}

/// Why a pane on the new account's folder did not stop, when it passes:
/// a switch that waits tries again. A Claude Code that got SIGTERM but
/// had not ended yet (in its pane or under its shell) is ending; the
/// others are a move under way or a pane in copy mode.
fn passes(error: &str) -> bool {
    [
        moves::DID_NOT_STOP,
        moves::DID_NOT_STOP_SHELL,
        moves::BUSY,
        moves::IN_MODE,
    ]
    .contains(&error)
}

/// The switch waits for `busy` (G4): `pending_global` says so, replacing
/// any other, and a switch back the rules planned is off (as with a
/// person's switch made at once). The rules never wait.
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
            By::Pending { at_ms, .. } => *at_ms,
            _ => now_ms(),
        },
    };
    accounts
        .update_registry_at_async(move |registry, _| {
            registry.pending_global = Some(pending);
            if let Some(last) = registry.last_switch.as_mut() {
                last.back_at_ms = None;
            }
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

/// How many times a switch that waited is tried again after a failure that
/// may pass (a lock Claude Code holds, a write that failed) before it is
/// called off, and how long apart (times the tries so far).
const PENDING_TRIES: u32 = 5;
#[cfg(not(test))]
const PENDING_RETRY_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const PENDING_RETRY_EVERY: std::time::Duration = std::time::Duration::ZERO;

impl Accounts {
    /// Whether the switch that waits may be tried now: never tried, or its
    /// next try is due.
    pub(crate) fn pending_try_due(&self, pending: &PendingSwitch) -> bool {
        let retry = self.pending_retry.lock().unwrap_or_else(|p| p.into_inner());
        match retry.as_ref() {
            Some(retry) if retry.account == pending.account && retry.at_ms == pending.at_ms => {
                std::time::Instant::now() >= retry.next
            }
            _ => true,
        }
    }

    /// A try of the switch that waits failed in a way that may pass:
    /// whether it is tried again (later), or has had its tries.
    fn pending_try_failed(&self, pending: &PendingSwitch) -> bool {
        let mut retry = self.pending_retry.lock().unwrap_or_else(|p| p.into_inner());
        let tries = match retry.as_ref() {
            Some(retry) if retry.account == pending.account && retry.at_ms == pending.at_ms => {
                retry.tries + 1
            }
            _ => 1,
        };
        *retry = Some(super::PendingRetry {
            account: pending.account.clone(),
            at_ms: pending.at_ms,
            tries,
            next: std::time::Instant::now() + PENDING_RETRY_EVERY * tries,
        });
        tries < PENDING_TRIES
    }

    fn forget_pending_tries(&self) {
        *self.pending_retry.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// The worker's side of a switch that waits (G4): once every Claude Code
/// on the new account's folder is idle, the switch happens. Called with
/// `swap_gate` held. A failure that may pass (a lock Claude Code holds, a
/// write that failed) is tried again a few times; one that can't (the
/// account signed out or removed, a pane that can't be restarted, Claude
/// Code's owner gate) calls the switch off, and the panes it waited for
/// (and any in the way) say why in `move_error`. Returns whether anything
/// changed.
pub async fn finish_pending(
    tmux: &Tmux,
    accounts: &Arc<Accounts>,
    panes: &HashMap<String, PaneState>,
) -> bool {
    let registry = accounts.registry();
    let Some(pending) = registry.pending_global.clone() else {
        accounts.note_global_waiting(None);
        accounts.forget_pending_tries();
        return false;
    };
    if !accounts.pending_try_due(&pending) {
        return false;
    }
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
    let waited_for: Vec<String> = sorted.on_to.iter().map(|p| p.pane_id.clone()).collect();
    match switch_locked(
        tmux,
        accounts,
        panes,
        &pending.account,
        By::Pending {
            by: pending.by.clone(),
            at_ms: pending.at_ms,
        },
    )
    .await
    {
        Ok(outcome) => {
            if !outcome.waiting {
                accounts.note_global_waiting(None);
                accounts.forget_pending_tries();
            }
            !outcome.waiting
        }
        // Changed or called off meanwhile (or finished by the recovery of
        // a swap left half-way): the next look sees it as it is.
        Err(SwitchError::Stale) => {
            let gone = accounts.registry().pending_global.is_none();
            if gone {
                accounts.note_global_waiting(None);
                accounts.forget_pending_tries();
            }
            gone
        }
        Err(error) => {
            let calls_off = match &error {
                SwitchError::NotSignedIn(_)
                | SwitchError::Blocked(..)
                | SwitchError::OwnerCheckOn => true,
                SwitchError::Failed(_) if !accounts.registry().has(&pending.account) => true,
                SwitchError::Busy | SwitchError::Failed(_) => {
                    !accounts.pending_try_failed(&pending)
                }
                // Once that sign-in is over.
                SwitchError::Stale | SwitchError::SigningIn(_) => false,
            };
            if !calls_off {
                if matches!(error, SwitchError::SigningIn(_)) {
                    tracing::debug!(account = %pending.account, error = %error.sentence(), "the switch that waits is tried again");
                } else {
                    tracing::info!(account = %pending.account, error = %error.sentence(), "the switch that waits is tried again");
                }
                return false;
            }
            tracing::warn!(account = %pending.account, error = %error.sentence(), "the switch that waited was called off");
            let why = format!(
                "The switch to {} was called off: {}",
                name_of(accounts, &pending.account),
                error.sentence()
            );
            let mut on = waited_for;
            if let SwitchError::Blocked(panes, _) = &error {
                on.extend(panes.iter().cloned());
            }
            on.sort();
            on.dedup();
            for pane in &on {
                accounts.note_move_error(pane, &why);
            }
            accounts.note_global_waiting(None);
            accounts.forget_pending_tries();
            let _ =
                accounts
                    .update_registry_at_async(|registry, _| {
                        if registry.pending_global.as_ref().is_some_and(|p| {
                            p.account == pending.account && p.at_ms == pending.at_ms
                        }) {
                            registry.pending_global = None;
                        }
                        Ok(())
                    })
                    .await;
            true
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
    use crate::accounts::test_support::Home;
    use serde_json::{json, Value};

    fn chain(token: &str) -> Value {
        json!({"claudeAiOauth": {"accessToken": format!("at-{token}"), "refreshToken": format!("rt-{token}"),
            "expiresAt": 4_102_444_800_000u64, "subscriptionType": "max"}})
    }

    fn record(uuid: &str, name: &str, email: &str) -> Value {
        json!({"accountUuid": uuid, "displayName": name, "emailAddress": email})
    }

    fn write(path: &std::path::Path, value: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    /// `~/.claude.json` as Claude Code leaves it after a sign-in.
    fn sign_in_on_slot(home: &Home, token: &str, record: Value) {
        write(
            &home.paths.claude_home.join(".credentials.json"),
            &chain(token),
        );
        write(
            &home.paths.default_json,
            &json!({"oauthAccount": record, "numStartups": 3}),
        );
    }

    /// The slot signed in to `default` (Work), `personal` in its folder,
    /// `accounts.json` made by an add.
    fn managed(name: &str) -> (Home, Arc<Accounts>) {
        let home = Home::new(name);
        sign_in_on_slot(&home, "work", record("uuid-work", "Work", "work@x.dev"));
        let accounts = home.accounts();
        accounts
            .update_registry(|registry| {
                registry.accounts.push(registry::Entry {
                    id: "personal".into(),
                    name: "Personal".into(),
                    color: "green".into(),
                    shared: vec![],
                    created_at_ms: 1,
                });
                Ok(())
            })
            .unwrap();
        let folder = home.paths.account_dir("personal");
        write(&folder.join(".credentials.json"), &chain("personal"));
        write(
            &home.paths.account_json("personal"),
            &json!({"oauthAccount": record("uuid-personal", "Personal", "me@x.dev")}),
        );
        (home, accounts)
    }

    fn look_again(accounts: &Accounts) {
        *accounts.slot_stamp.lock().unwrap() = None;
    }

    #[test]
    fn the_default_folder_holds_the_slots_record_from_the_start() {
        let (home, accounts) = managed("g6-seed");
        let stores = accounts.stores().unwrap();
        assert_eq!(
            stores.uuid(&Place::Folder(DEFAULT_ID.into())).as_deref(),
            Some("uuid-work")
        );
        assert!(!home
            .paths
            .account_dir(DEFAULT_ID)
            .join(".credentials.json")
            .exists());
        // Nothing changed by hand: nothing to take in.
        assert!(!accounts.watch_slot_blocking());
    }

    #[test]
    fn a_sign_in_by_hand_to_a_known_account_makes_it_global() {
        let (home, accounts) = managed("g6-known");
        assert!(!accounts.watch_slot_blocking());
        // `/login` as Personal on the slot (another chain of Personal).
        sign_in_on_slot(
            &home,
            "personal-2",
            record("uuid-personal", "Personal", "me@x.dev"),
        );
        look_again(&accounts);
        assert!(accounts.watch_slot_blocking());
        let registry = accounts.registry();
        assert_eq!(registry.global, "personal");
        let last = registry.last_switch.unwrap();
        assert_eq!(
            (last.from.as_str(), last.to.as_str()),
            ("default", "personal")
        );
        assert_eq!(last.reason.as_deref(), Some("login"));
        assert!(!last.auto);
        // Looked at once per change.
        assert!(!accounts.watch_slot_blocking());
    }

    #[test]
    fn a_sign_in_by_hand_to_a_new_account_adds_it() {
        let (home, accounts) = managed("g6-new");
        sign_in_on_slot(&home, "ana", record("uuid-ana", "Personal", "ana@y.dev"));
        look_again(&accounts);
        assert!(accounts.watch_slot_blocking());
        let registry = accounts.registry();
        let added = registry.accounts.last().unwrap();
        assert_eq!(
            added.name, "Personal 2",
            "named after its profile, never twice"
        );
        assert_eq!(registry.global, added.id);
        assert_eq!(added.shared.len(), shared::GROUPS.len());
        assert_eq!(
            registry.last_switch.as_ref().unwrap().reason.as_deref(),
            Some("login_new")
        );
        let stores = accounts.stores().unwrap();
        assert_eq!(
            stores.uuid(&Place::Folder(added.id.clone())).as_deref(),
            Some("uuid-ana"),
            "its folder holds its record"
        );
        assert!(
            !home
                .paths
                .account_dir(&added.id)
                .join(".credentials.json")
                .exists(),
            "its chain stays in the slot"
        );
    }

    #[test]
    fn signing_out_by_hand_keeps_the_global_account() {
        let (home, accounts) = managed("g6-out");
        write(&home.paths.default_json, &json!({"numStartups": 9}));
        let _ = std::fs::remove_file(home.paths.claude_home.join(".credentials.json"));
        look_again(&accounts);
        assert!(!accounts.watch_slot_blocking());
        assert_eq!(accounts.registry().global, "default");
    }

    #[test]
    fn the_old_profile_written_back_after_a_swap_is_put_right() {
        let (home, accounts) = managed("g6-back");
        let stores = accounts.stores().unwrap();
        slot::swap(
            &stores,
            "default",
            "personal",
            None,
            &|journal| accounts.commit_swap(journal),
            Crash::None,
        )
        .unwrap();
        assert_eq!(accounts.registry().global, "personal");
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-personal"));
        // A session that fetched Work's profile right before writes it back.
        write_back_work(&home);
        look_again(&accounts);
        assert!(!accounts.watch_slot_blocking(), "no sign-in by hand");
        assert_eq!(accounts.registry().global, "personal");
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-personal"));
        assert_eq!(
            slot_chain(&home),
            chain_of("personal"),
            "Personal's sign-in"
        );
    }

    #[tokio::test]
    async fn a_swap_left_half_way_is_settled_before_anything_starts() {
        let (_home, accounts) = managed("unsettled");
        {
            let stores = accounts.stores().unwrap();
            slot::swap(
                &stores,
                "default",
                "personal",
                None,
                &|journal| accounts.commit_swap(journal),
                Crash::After(3),
            )
            .unwrap_err();
        }
        // Another worker process holds the swap lock: its swap runs still.
        let held = accounts.stores().unwrap().hold_swap().unwrap();
        let unsettled = accounts.unsettled_swap().await;
        assert!(unsettled.is_some_and(|journal| journal.to == "personal"));
        assert_eq!(accounts.registry().global, "default");
        drop(held);
        assert!(accounts.unsettled_swap().await.is_none(), "settled now");
        assert_eq!(accounts.registry().global, "personal");
    }

    /// Default (Work) swapped out for Personal, as `make_global` does.
    fn swapped_to_personal(name: &str) -> (Home, Arc<Accounts>) {
        let (home, accounts) = managed(name);
        slot::swap(
            &accounts.stores().unwrap(),
            "default",
            "personal",
            None,
            &|journal| accounts.commit_swap(journal),
            Crash::None,
        )
        .unwrap();
        (home, accounts)
    }

    /// Work's profile written over the slot's, as a stale session or the
    /// first step of a `/login` to Work leaves it.
    fn write_back_work(home: &Home) {
        let mut config = slot::read_config(&home.paths.default_json)
            .unwrap()
            .unwrap();
        config.insert(
            slot::IDENTITY_KEY.into(),
            record("uuid-work", "Work", "work@x.dev"),
        );
        write(&home.paths.default_json, &Value::Object(config));
    }

    fn slot_chain(home: &Home) -> Option<String> {
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(home.paths.claude_home.join(".credentials.json")).unwrap(),
        )
        .unwrap();
        super::super::credentials::chain_id(raw.as_object())
    }

    fn chain_of(token: &str) -> Option<String> {
        super::super::credentials::chain_id(chain(token).as_object())
    }

    #[test]
    fn the_old_profile_over_a_renewed_sign_in_is_put_right_too() {
        let (home, accounts) = swapped_to_personal("p23-renewed");
        // Claude Code renews Personal's sign-in in the slot; the worker
        // sees it on its next look.
        write(
            &home.paths.claude_home.join(".credentials.json"),
            &chain("personal-renewed"),
        );
        assert!(!accounts.watch_slot_blocking());
        write_back_work(&home);
        assert!(!accounts.watch_slot_blocking(), "no sign-in by hand");
        assert_eq!(accounts.registry().global, "personal");
        let stores = accounts.stores().unwrap();
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-personal"));
    }

    /// P23: `/login` back to Work within the minute after the swap. Its
    /// new chain with Work's profile is a sign-in by hand (G6), never a
    /// write-back: Work becomes global and the slot keeps Work's profile.
    #[test]
    fn a_sign_in_by_hand_to_the_old_account_right_after_a_swap_makes_it_global() {
        let (home, accounts) = swapped_to_personal("p23-login");
        write(
            &home.paths.claude_home.join(".credentials.json"),
            &chain("work-again"),
        );
        write_back_work(&home);
        assert!(accounts.watch_slot_blocking());
        let registry = accounts.registry();
        assert_eq!(registry.global, "default");
        assert_eq!(
            registry.last_switch.as_ref().unwrap().reason.as_deref(),
            Some("login")
        );
        let stores = accounts.stores().unwrap();
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-work"));
        assert_eq!(slot_chain(&home), chain_of("work-again"));
        // Nothing to put right afterwards either.
        assert!(!accounts.watch_slot_blocking());
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-work"));
    }

    /// A sign-in by hand that writes its chain before its profile: a look
    /// in between sees a new chain under Personal's profile, which is not
    /// taken for Personal's renewal once Work's profile follows.
    #[test]
    fn a_sign_in_by_hand_chain_first_is_not_taken_for_a_renewal() {
        let (home, accounts) = swapped_to_personal("p23-chain-first");
        write(
            &home.paths.claude_home.join(".credentials.json"),
            &chain("work-again"),
        );
        let config = home.paths.default_json.clone();
        let profile = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut json = slot::read_config(&config).unwrap().unwrap();
            json.insert(
                slot::IDENTITY_KEY.into(),
                record("uuid-work", "Work", "work@x.dev"),
            );
            write(&config, &Value::Object(json));
        });
        assert!(!accounts.watch_slot_blocking());
        profile.join().unwrap();
        assert!(accounts.watch_slot_blocking(), "a sign-in by hand");
        assert_eq!(accounts.registry().global, "default");
        let stores = accounts.stores().unwrap();
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-work"));
    }

    /// `/login` writes the profile before the sign-in: a look in between
    /// sees Work's profile over Personal's chain, and waits for the
    /// sign-in to land before it puts anything back.
    #[test]
    fn a_sign_in_by_hand_seen_halfway_is_not_put_right() {
        let (home, accounts) = swapped_to_personal("p23-halfway");
        write_back_work(&home);
        let credentials = home.paths.claude_home.join(".credentials.json");
        let login = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            write(&credentials, &chain("work-again"));
        });
        assert!(accounts.watch_slot_blocking());
        login.join().unwrap();
        assert_eq!(accounts.registry().global, "default");
        let stores = accounts.stores().unwrap();
        assert_eq!(stores.uuid(&Place::Slot).as_deref(), Some("uuid-work"));
        assert_eq!(slot_chain(&home), chain_of("work-again"));
    }

    #[test]
    fn nothing_is_watched_without_accounts_json() {
        let home = Home::new("g6-plain");
        sign_in_on_slot(&home, "work", record("uuid-work", "Work", "work@x.dev"));
        let accounts = home.accounts();
        sign_in_on_slot(&home, "other", record("uuid-other", "Other", "o@x.dev"));
        assert!(!accounts.watch_slot_blocking());
        assert!(!accounts.managed());
        assert!(!home.paths.account_dir(DEFAULT_ID).exists());
    }

    /// `accounts.json` as 0.3.0 left it: no `slot_version`, `global`
    /// naming a folder account.
    fn old_registry(home: &Home, global: &str) {
        let mut registry = Registry::default();
        registry.accounts.push(registry::Entry {
            id: "personal".into(),
            name: "Personal".into(),
            color: "green".into(),
            shared: vec!["settings".into()],
            created_at_ms: 1,
        });
        registry.accounts[0].shared = Vec::new();
        registry.global = global.into();
        registry.slot_version = 0;
        let text = serde_json::to_string(&registry).unwrap();
        assert!(!text.contains("slot_version"));
        write(
            &home.paths.registry_file(),
            &serde_json::from_str(&text).unwrap(),
        );
    }

    #[test]
    fn an_upgrade_takes_the_slots_account_for_global() {
        let home = Home::new("g11");
        sign_in_on_slot(&home, "work", record("uuid-work", "Work", "work@x.dev"));
        let folder = home.paths.account_dir("personal");
        write(&folder.join(".credentials.json"), &chain("personal"));
        write(
            &home.paths.account_json("personal"),
            &json!({"oauthAccount": record("uuid-personal", "Personal", "me@x.dev")}),
        );
        old_registry(&home, "personal");
        let accounts = home.accounts();
        assert!(accounts.needs_upgrade());
        accounts.upgrade().unwrap();
        let registry = accounts.registry();
        assert_eq!(registry.slot_version, registry::SLOT_VERSION);
        assert_eq!(registry.global, "default", "the slot holds Work");
        let last = registry.last_switch.clone().unwrap();
        assert_eq!(last.reason.as_deref(), Some("upgrade"));
        assert_eq!((last.from.as_str(), last.auto), ("personal", false));
        assert_eq!(
            registry.get(DEFAULT_ID).unwrap().shared.len(),
            shared::GROUPS.len()
        );
        let stores = accounts.stores().unwrap();
        assert_eq!(
            stores.uuid(&Place::Folder(DEFAULT_ID.into())).as_deref(),
            Some("uuid-work")
        );
        assert!(!accounts.needs_upgrade());
        // Personal's chain stayed in its folder.
        assert!(accounts.folder_signed_in("personal"));
    }

    #[test]
    fn an_upgrade_keeps_a_global_account_already_in_the_slot() {
        let home = Home::new("g11-same");
        sign_in_on_slot(
            &home,
            "personal",
            record("uuid-personal", "Personal", "me@x.dev"),
        );
        write(
            &home.paths.account_json("personal"),
            &json!({"oauthAccount": record("uuid-personal", "Personal", "me@x.dev")}),
        );
        old_registry(&home, "personal");
        let accounts = home.accounts();
        accounts.upgrade().unwrap();
        let registry = accounts.registry();
        assert_eq!(registry.global, "personal");
        assert!(registry.last_switch.is_none(), "nothing changed: no notice");
        // Default's folder exists, with no record of the slot's (Personal's).
        assert!(home.paths.account_dir(DEFAULT_ID).is_dir());
        assert_eq!(
            accounts
                .stores()
                .unwrap()
                .uuid(&Place::Folder(DEFAULT_ID.into())),
            None
        );
    }

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

    /// A tmux nothing here may call: these switches have no pane to stop.
    fn no_tmux() -> Tmux {
        Tmux {
            path: "/nonexistent/tmux".into(),
            socket: crate::tmux_cli::Socket::Name("iii-unit-never".into()),
            timeout: std::time::Duration::from_secs(1),
            version: String::new(),
        }
    }

    fn wait_for_personal(accounts: &Accounts, at_ms: u64) {
        accounts
            .update_registry(|registry| {
                registry.pending_global = Some(PendingSwitch {
                    account: "personal".into(),
                    by: "person".into(),
                    at_ms,
                });
                Ok(())
            })
            .unwrap();
    }

    /// A Claude Code on Personal's folder that can't be restarted.
    fn unmovable_on_personal() -> HashMap<String, PaneState> {
        use crate::model::{AgentState, Follow, PaneClaude};
        let pane = PaneState {
            pane_id: "%3".into(),
            pane_pid: 10,
            claude_pid: Some(11),
            record: Some(crate::activity::ClaudeRecord {
                config_dir: "/h/personal".into(),
                session_id: Some("s1".into()),
                status: Some("idle".into()),
                cwd: Some("/p".into()),
            }),
            marks: Default::default(),
            info: PaneClaude {
                account: Some("personal".into()),
                config_dir: None,
                follow: Follow::Manual,
                pending: None,
                skip_permissions: false,
                session_id: Some("s1".into()),
                movable: false,
                not_movable_reason: Some(super::super::panes::NESTED.into()),
                move_error: None,
            },
            agent_state: Some(AgentState::Idle),
            cwd: "/p".into(),
            place: Some(Place::Folder("personal".into())),
            kind: None,
            marks_shared: false,
        };
        HashMap::from([("%3".to_string(), pane)])
    }

    #[test]
    fn an_account_a_swap_involves_is_not_removed() {
        let (home, accounts) = managed("remove-mid-swap");
        // A swap to Personal under way (or left half-way): its journal.
        write(
            &home.paths.root.join(slot::JOURNAL),
            &json!({"from": "default", "to": "personal", "step": 4}),
        );
        let refused = accounts.remove("personal", true, 0).unwrap_err();
        assert_eq!(refused, super::super::manage::SWAP_UNDER_WAY);
        assert!(accounts.registry().has("personal"));
        assert!(home.paths.account_dir("personal").is_dir(), "folder kept");
        std::fs::remove_file(home.paths.root.join(slot::JOURNAL)).unwrap();
        accounts.remove("personal", true, 0).unwrap();
        assert!(!home.paths.account_dir("personal").exists());
    }

    /// A sign-in from the app on the slot or on either folder: no switch
    /// until it is over; one on another account's folder is no matter.
    #[tokio::test]
    async fn no_switch_crosses_a_sign_in_under_way() {
        let (_home, accounts) = managed("switch-signing-in");
        for (account, on_slot) in [("personal", false), ("default", true)] {
            let reserved = accounts.logins.reserve("l1", account, on_slot);
            let refused = switch_locked(
                &no_tmux(),
                &accounts,
                &HashMap::new(),
                "personal",
                By::Person,
            )
            .await;
            assert_eq!(
                refused,
                Err(SwitchError::SigningIn(name_of(&accounts, account)))
            );
            assert_eq!(refused.unwrap_err().code(), "signing_in");
            assert_eq!(accounts.registry().global, "default");
            drop(reserved);
        }
        let elsewhere = accounts.logins.reserve("l2", "side-project", false);
        let done = switch_locked(
            &no_tmux(),
            &accounts,
            &HashMap::new(),
            "personal",
            By::Person,
        )
        .await;
        assert_eq!(done.unwrap().global, "personal");
        drop(elsewhere);
    }

    #[tokio::test]
    async fn a_waiting_switch_goes_on_only_as_it_was_asked() {
        let (_home, accounts) = managed("pending-same");
        wait_for_personal(&accounts, 7);
        // Another request since (another time): this one is not it.
        let other = By::Pending {
            by: "person".into(),
            at_ms: 8,
        };
        let stale = switch_locked(&no_tmux(), &accounts, &HashMap::new(), "personal", other).await;
        assert_eq!(stale, Err(SwitchError::Stale));
        assert_eq!(accounts.registry().global, "default");
        // Called off meanwhile: not it either.
        let asked = By::Pending {
            by: "person".into(),
            at_ms: 7,
        };
        accounts
            .update_registry(|registry| {
                registry.pending_global = None;
                Ok(())
            })
            .unwrap();
        let stale = switch_locked(
            &no_tmux(),
            &accounts,
            &HashMap::new(),
            "personal",
            asked.clone(),
        )
        .await;
        assert_eq!(stale, Err(SwitchError::Stale));
        // As it was asked: it goes on.
        wait_for_personal(&accounts, 7);
        let done = switch_locked(&no_tmux(), &accounts, &HashMap::new(), "personal", asked).await;
        assert_eq!(done.unwrap().global, "personal");
        let registry = accounts.registry();
        assert_eq!(registry.global, "personal");
        assert!(registry.pending_global.is_none());
    }

    #[tokio::test]
    async fn a_waiting_switch_is_tried_again_after_a_failure_that_may_pass() {
        let (home, accounts) = managed("pending-retry");
        wait_for_personal(&accounts, 7);
        // The journal can't be written (a folder in its place): the swap
        // fails before it touches anything.
        let journal = home.paths.root.join(slot::JOURNAL);
        std::fs::create_dir_all(&journal).unwrap();
        let panes = HashMap::new();
        assert!(!finish_pending(&no_tmux(), &accounts, &panes).await);
        let registry = accounts.registry();
        assert_eq!(registry.global, "default");
        assert!(registry.pending_global.is_some(), "kept for the next try");
        // It passed: the next try switches.
        std::fs::remove_dir(&journal).unwrap();
        assert!(finish_pending(&no_tmux(), &accounts, &panes).await);
        let registry = accounts.registry();
        assert_eq!(registry.global, "personal");
        assert!(registry.pending_global.is_none());
    }

    #[tokio::test]
    async fn a_waiting_switch_is_called_off_after_its_tries() {
        let (home, accounts) = managed("pending-tries");
        wait_for_personal(&accounts, 7);
        std::fs::create_dir_all(home.paths.root.join(slot::JOURNAL)).unwrap();
        let panes = HashMap::new();
        for _ in 1..PENDING_TRIES {
            assert!(!finish_pending(&no_tmux(), &accounts, &panes).await);
            assert!(accounts.registry().pending_global.is_some());
        }
        assert!(finish_pending(&no_tmux(), &accounts, &panes).await);
        let registry = accounts.registry();
        assert_eq!(registry.global, "default");
        assert!(registry.pending_global.is_none(), "called off");
    }

    #[tokio::test]
    async fn a_person_s_switch_that_waits_calls_off_the_switch_back() {
        let (_home, accounts) = managed("wait-no-back");
        // The rules made Work global, planning to switch back once the
        // account they left resets.
        accounts
            .update_registry(|registry| {
                registry.last_switch = Some(AccountSwitch {
                    from: "side".into(),
                    to: "default".into(),
                    reason: Some("week_90".into()),
                    auto: true,
                    following: 0,
                    restarted: Vec::new(),
                    after_turn: Vec::new(),
                    at_ms: 1,
                    back_at_ms: Some(9_000),
                });
                Ok(())
            })
            .unwrap();
        // Personal's folder has Claude Code in a turn: the switch waits.
        let mut panes = unmovable_on_personal();
        let pane = panes.get_mut("%3").unwrap();
        pane.info.movable = true;
        pane.info.not_movable_reason = None;
        pane.kind = Some(super::super::panes::MoveKind::Respawn);
        pane.record.as_mut().unwrap().status = Some("busy".into());
        let outcome = switch_locked(&no_tmux(), &accounts, &panes, "personal", By::Person)
            .await
            .unwrap();
        assert!(outcome.waiting);
        let registry = accounts.registry();
        assert_eq!(registry.pending_global.unwrap().account, "personal");
        assert_eq!(
            registry.last_switch.unwrap().back_at_ms,
            None,
            "no switch back to discard the one asked for"
        );
    }

    #[test]
    fn a_claude_code_still_ending_does_not_call_off_a_waiting_switch() {
        assert!(passes(moves::DID_NOT_STOP));
        assert!(passes(moves::DID_NOT_STOP_SHELL));
        assert!(passes(moves::IN_MODE));
        assert!(!passes(super::super::panes::NESTED));
    }

    #[tokio::test]
    async fn a_waiting_switch_that_cant_happen_says_why_on_its_panes() {
        let (home, accounts) = managed("pending-off");
        wait_for_personal(&accounts, 7);
        let panes = unmovable_on_personal();
        assert!(finish_pending(&no_tmux(), &accounts, &panes).await);
        let registry = accounts.registry();
        assert_eq!(registry.global, "default");
        assert!(registry.pending_global.is_none(), "called off");
        let alive = std::collections::HashSet::from(["%3"]);
        let why = accounts.move_errors(&alive).remove("%3").unwrap();
        assert!(
            why.starts_with("The switch to Personal was called off: Claude Code in %3"),
            "{why}"
        );
        // Signed out meanwhile: called off too, at once.
        wait_for_personal(&accounts, 9);
        std::fs::remove_file(home.paths.account_dir("personal").join(".credentials.json")).unwrap();
        assert!(finish_pending(&no_tmux(), &accounts, &HashMap::new()).await);
        assert!(accounts.registry().pending_global.is_none());
    }
}
