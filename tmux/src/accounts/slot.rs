//! The global slot (agent-accounts G1–G3, G6–G8, G11): the default config
//! folder, `~/.claude` with `~/.claude.json` next to it, whose sign-in is
//! the global account's. Every account also has a folder of its own under
//! the accounts root (`<root>/<id>`, `default` included, G2) that holds its
//! sign-in while it is not global.
//!
//! Making X global while G is (G3) moves sign-ins between stores under
//! Claude Code's own locks, so a running Claude Code on the slot goes out as
//! X on its next request without a restart: G's sign-in (its newest token
//! chain) goes from the slot to G's folder, X's from X's folder to the
//! slot, and `~/.claude.json`'s `oauthAccount` becomes X's record. A chain
//! lives in one store at a time: two copies would part on the first refresh
//! and Claude Code would clear the loser.
//!
//! Every step is noted in `<root>/swap.json` (G7, no tokens): a worker that
//! stops midway finishes the swap when it starts again, deciding from what
//! the stores hold. Nothing here logs, prints or returns a token.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::credentials::{chain_id, Raw, SecretStore, OAUTH_KEY};
use super::lockdir::{self, LockId, LockSet};
use super::paths::ClaudePaths;
use crate::model::AccountSwitch;

/// What `busy` says.
pub const BUSY_TEXT: &str = "Claude Code is renewing its sign-in; try again in a moment.";
/// The Claude Code feature gate that ties a running session to the account
/// it started with (G8).
pub const OWNER_GATE: &str = "tengu_bubbly_fountain";
/// What `/login` deletes from `~/.claude.json` along with `oauthAccount`
/// (Claude Code 2.1.322's `Yor()`): caches of the account signed in.
pub const LOGIN_CACHE_KEYS: [&str; 18] = [
    "managedSettingsNoticeSeenOrg",
    "additionalModelOptionsCache",
    "additionalModelOptionsAnsweredAt",
    "additionalModelCostsCache",
    "modelAccessCache",
    "orgModelDefaultCache",
    "cachedArtifactRoster",
    "artifactRosterDenied",
    "lastSeenOrgDefaultUpdatedAt",
    "clientDataCache",
    "clientDataCacheSlots",
    "autoCompactWindowsCache",
    "cachedUsageUtilization",
    "metricsStatusCache",
    "metricsStatusCacheByPrincipal",
    "githubWebConnectionStatusCache",
    "unusedLimitResetStatusCache",
    "startupPrefetchedAt",
];
/// The record of the account signed in, in a `.claude.json`.
pub const IDENTITY_KEY: &str = "oauthAccount";
/// The journal's name in the accounts root.
pub const JOURNAL: &str = "swap.json";
/// `<root>/swap.lock`: held (`flock`) by the worker that swaps or settles
/// a swap, for its whole run. Another worker process on the same accounts
/// finds it taken and leaves that swap, its journal and its locks alone,
/// which the gate inside one process can't tell it. The system lets go of
/// it when its process ends, crashed or not.
pub const SWAP_LOCK: &str = "swap.lock";
/// The keys of a sign-in store that belong to the Anthropic account signed
/// in there: Claude Code 2.1.322's logout removes exactly these (`E()` in
/// its auth chunk) and revokes the three token chains among them
/// (`claudeAiOauth`, `designOauth`, `cloudSessionsOauth`). They move with
/// the account in a swap. The rest of the store stays where it is:
/// `mcpOAuth` holds sign-ins to MCP servers, which are the person's, not
/// the Claude account's.
pub const ACCOUNT_KEYS: [&str; 7] = [
    OAUTH_KEY,
    "designOauth",
    "cloudSessionsOauth",
    "organizationUuid",
    "trustedDeviceToken",
    "trustedDeviceEnrolledAfterRefusalAtMs",
    "enterpriseGateway",
];
/// How long a swap may hold the stores' locks. The locks are touched
/// before every store call (a Keychain call may take up to five seconds),
/// and a swap still going past this stops safely, well before Claude Code
/// would take `.storage-write.lock` over (15 s untouched).
pub const HOLD_BUDGET: Duration = Duration::from_secs(8);
/// What a swap that ran out of [`HOLD_BUDGET`] says.
pub const TOO_SLOW: &str =
    "The sign-in store answered too slowly (is the Keychain locked?); try again.";

/// [`SWAP_LOCK`] held, until dropped.
pub struct SwapHold {
    _file: std::fs::File,
}

/// Why a swap did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwapError {
    /// The account to make global holds no sign-in.
    NotSignedIn,
    /// A lock stayed taken for ten seconds (Claude Code renewing a sign-in).
    Busy,
    /// Claude Code ties running sessions to their account (G8).
    OwnerCheckOn,
    /// Someone signed in by hand on the slot: it holds this account
    /// (`accountUuid`) and not the global one (G6 first).
    Identity(Option<String>),
    /// Anything else, in one sentence.
    Failed(String),
}

impl SwapError {
    pub fn sentence(&self) -> String {
        match self {
            Self::NotSignedIn => "The account is not signed in.".into(),
            Self::Busy => BUSY_TEXT.into(),
            Self::OwnerCheckOn => "This Claude Code ties running sessions to their account, so switching would stop them.".into(),
            Self::Identity(_) => "Someone signed in to another account with /login; the worker took it in first.".into(),
            Self::Failed(text) => text.clone(),
        }
    }
}

impl From<lockdir::LockError> for SwapError {
    fn from(error: lockdir::LockError) -> Self {
        match error {
            lockdir::LockError::Busy => Self::Busy,
            lockdir::LockError::Io(text) => Self::Failed(text),
        }
    }
}

/// `<root>/swap.json`: a swap under way (G7). Hashes only, never a token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    /// The global account before (G).
    pub from: String,
    /// The account made global (X).
    pub to: String,
    /// The last step done: 0 (written), 1 (locks taken), 3, 4, 5, 6.
    pub step: u8,
    /// [`chain_id`] of X's chain.
    #[serde(default)]
    pub x_rt: Option<String>,
    /// [`chain_id`] of G's chain as it left the slot.
    #[serde(default)]
    pub g_rt: Option<String>,
    /// The locks this swap took, so a recovery knows them for its own.
    #[serde(default)]
    pub locks: Vec<LockId>,
    /// The `last_switch` the registry gets with the swap.
    #[serde(default)]
    pub record: Option<AccountSwitch>,
    /// The swap failed (not a crash) before the slot held X's chain, and
    /// could not be undone then: the next look undoes it.
    #[serde(default)]
    pub aborted: bool,
}

/// Where a sign-in lives: the slot, or account `id`'s folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Place {
    Slot,
    Folder(String),
}

/// The paths of the slot and the folders, and the store that holds their
/// sign-ins.
pub struct Stores<'a> {
    pub paths: &'a ClaudePaths,
    pub secrets: &'a dyn SecretStore,
    /// How long the locks are waited for ([`lockdir::WAIT`]).
    pub wait: Duration,
    /// How long a swap may hold them ([`HOLD_BUDGET`]).
    pub budget: Duration,
}

/// A test's way to stop a swap midway, as if the worker died there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crash {
    None,
    /// Right after the given step was done (and noted).
    After(u8),
}

impl<'a> Stores<'a> {
    pub fn new(paths: &'a ClaudePaths, secrets: &'a dyn SecretStore) -> Self {
        Self {
            paths,
            secrets,
            wait: lockdir::WAIT,
            budget: HOLD_BUDGET,
        }
    }

    pub fn dir(&self, place: &Place) -> PathBuf {
        match place {
            Place::Slot => self.paths.claude_home.clone(),
            Place::Folder(id) => self.paths.account_dir(id),
        }
    }

    /// The `CLAUDE_CONFIG_DIR` a Claude Code on `place` runs with.
    pub fn env_dir(&self, place: &Place) -> Option<String> {
        match place {
            Place::Slot => None,
            Place::Folder(id) => Some(self.paths.account_dir(id).to_string_lossy().into_owned()),
        }
    }

    pub fn json(&self, place: &Place) -> PathBuf {
        match place {
            Place::Slot => self.paths.default_json.clone(),
            Place::Folder(id) => self.paths.account_json(id),
        }
    }

    pub fn read(&self, place: &Place) -> Result<Option<Raw>, String> {
        self.secrets
            .read_raw(&self.dir(place), self.env_dir(place).as_deref())
    }

    fn write(&self, place: &Place, raw: &Raw) -> Result<(), String> {
        self.secrets
            .write_raw(&self.dir(place), self.env_dir(place).as_deref(), raw)
    }

    /// Hold [`SWAP_LOCK`], or [`SwapError::Busy`] while another process
    /// holds it.
    pub fn hold_swap(&self) -> Result<SwapHold, SwapError> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        super::registry::make_private_dir(&self.paths.root).map_err(SwapError::Failed)?;
        let path = self.paths.root.join(SWAP_LOCK);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                SwapError::Failed(format!("Could not open {} ({error}).", path.display()))
            })?;
        // SAFETY: `file` owns a valid descriptor for the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(SwapHold { _file: file });
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Err(SwapError::Busy)
        } else {
            Err(SwapError::Failed(format!(
                "Could not lock {} ({error}).",
                path.display()
            )))
        }
    }

    fn journal_path(&self) -> PathBuf {
        self.paths.root.join(JOURNAL)
    }

    pub fn read_journal(&self) -> Option<Journal> {
        let text = std::fs::read_to_string(self.journal_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write_journal(&self, journal: &Journal) -> Result<(), SwapError> {
        super::registry::write_atomic(&self.journal_path(), journal).map_err(SwapError::Failed)
    }

    fn drop_journal(&self) {
        let _ = std::fs::remove_file(self.journal_path());
    }

    /// The `accountUuid` of `place`'s `.claude.json` record.
    pub fn uuid(&self, place: &Place) -> Option<String> {
        read_config(&self.json(place))
            .ok()
            .flatten()
            .as_ref()
            .and_then(account_uuid)
    }

    /// Remove what the worker moved aside from the locks of the slot and of
    /// `folders` (`<lock>.stale-*`) once past their stale period: at start.
    /// Every lock taken does the same for itself first.
    pub fn sweep_aside(&self, folders: &[Place]) {
        for place in std::iter::once(&Place::Slot).chain(folders) {
            let dir = self.dir(place);
            lockdir::sweep_aside(&dir.join(lockdir::REFRESH), lockdir::REFRESH_STALE);
            lockdir::sweep_aside(&lockdir::legacy_path(&dir), lockdir::LEGACY_STALE);
            lockdir::sweep_aside(&dir.join(lockdir::STORAGE), lockdir::STORAGE_STALE);
            lockdir::sweep_aside(
                &lockdir::config_lock_path(&self.json(place)),
                lockdir::CONFIG_STALE,
            );
        }
    }

    /// Take the locks of the slot, then of `folders`, in Claude Code's order
    /// (refresh, legacy, storage), waiting at most [`lockdir::WAIT`].
    fn lock_all(&self, folders: &[&Place]) -> Result<(LockSet, PathBuf), SwapError> {
        let deadline = Instant::now() + self.wait;
        let mut locks = LockSet::new();
        let slot = self.paths.claude_home.clone();
        super::registry::make_private_dir(&slot).map_err(SwapError::Failed)?;
        let storage = slot.join(lockdir::STORAGE);
        let take = |dir: &Path, locks: &mut LockSet| -> Result<(), SwapError> {
            locks.take(
                &dir.join(lockdir::REFRESH),
                lockdir::REFRESH_STALE,
                deadline,
            )?;
            locks.take(&lockdir::legacy_path(dir), lockdir::LEGACY_STALE, deadline)?;
            locks.take(
                &dir.join(lockdir::STORAGE),
                lockdir::STORAGE_STALE,
                deadline,
            )?;
            Ok(())
        };
        take(&slot, &mut locks)?;
        for place in folders {
            let dir = self.dir(place);
            super::registry::make_private_dir(&dir).map_err(SwapError::Failed)?;
            take(&dir, &mut locks)?;
        }
        Ok((locks, storage))
    }
}

/// `accountUuid` of a `.claude.json`'s `oauthAccount`.
pub fn account_uuid(config: &Map<String, Value>) -> Option<String> {
    config
        .get(IDENTITY_KEY)?
        .get("accountUuid")?
        .as_str()
        .filter(|uuid| !uuid.is_empty())
        .map(str::to_string)
}

/// A `.claude.json`, `None` when missing.
pub fn read_config(path: &Path) -> Result<Option<Map<String, Value>>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Ok(Some(map)),
            _ => Err(format!("{} can't be read.", path.display())),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "{} can't be read ({}).",
            path.display(),
            error.kind()
        )),
    }
}

/// Write a `.claude.json` as Claude Code does: a temporary file next to the
/// file it names (a symlink is followed and kept), `0600`, synced, renamed.
pub fn write_config(path: &Path, value: &Map<String, Value>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let target = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path)
            .map_err(|error| format!("Could not follow {} ({error}).", path.display()))?,
        _ => path.to_path_buf(),
    };
    let folder = target
        .parent()
        .ok_or_else(|| format!("{} has no folder.", target.display()))?;
    super::registry::make_private_dir(folder)?;
    let mut text = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    text.push(b'\n');
    let temp = folder.join(format!(
        ".{}.tmp.{}.{}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("claude.json"),
        std::process::id(),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&text)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Could not save {} ({error}).", target.display()));
    }
    Ok(())
}

/// Change a `.claude.json` under its own lock (`<file>.lock`), read again
/// under it, as Claude Code does. `change` returns whether to write.
pub fn update_config(
    path: &Path,
    change: impl FnOnce(&mut Map<String, Value>) -> bool,
) -> Result<(), SwapError> {
    let mut locks = LockSet::new();
    locks.take(
        &lockdir::config_lock_path(path),
        lockdir::CONFIG_STALE,
        Instant::now() + lockdir::WAIT,
    )?;
    let mut config = read_config(path)
        .map_err(SwapError::Failed)?
        .unwrap_or_default();
    if change(&mut config) {
        write_config(path, &config).map_err(SwapError::Failed)?;
    }
    Ok(())
}

/// Whether Claude Code's owner gate is on in `~/.claude.json` (G8).
pub fn owner_gate_on(config: Option<&Map<String, Value>>) -> bool {
    config
        .and_then(|config| config.get("cachedGrowthBookFeatures"))
        .and_then(|features| features.get(OWNER_GATE))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// What a session on a folder needs from `~/.claude.json` so it starts
/// without asking again (G2): onboarding done, the bypass mode accepted,
/// each project's trust, the user-scope MCP servers (when the folder has
/// none). Only adds; returns whether anything changed.
pub fn carry_session_keys(from: &Map<String, Value>, to: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    for key in [
        "hasCompletedOnboarding",
        "lastOnboardingVersion",
        "bypassPermissionsModeAccepted",
    ] {
        if let Some(value) = from.get(key) {
            if !to.contains_key(key) {
                to.insert(key.to_string(), value.clone());
                changed = true;
            }
        }
    }
    if let Some(servers) = from
        .get("mcpServers")
        .and_then(Value::as_object)
        .filter(|servers| !servers.is_empty())
    {
        if !to.contains_key("mcpServers") {
            to.insert("mcpServers".into(), Value::Object(servers.clone()));
            changed = true;
        }
    }
    if let Some(projects) = from.get("projects").and_then(Value::as_object) {
        for (project, entry) in projects {
            if entry.get("hasTrustDialogAccepted").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let theirs = to
                .entry("projects")
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(theirs) = theirs.as_object_mut() else {
                continue;
            };
            let slot = theirs
                .entry(project.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(slot) = slot.as_object_mut() {
                if slot.get("hasTrustDialogAccepted").and_then(Value::as_bool) != Some(true) {
                    slot.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Make account `id`'s folder when missing (`0700`), link what it shares,
/// and give its `.claude.json` what a session needs from `~/.claude.json`,
/// with the slot's `oauthAccount` too when `identity` (G2: `default` at
/// the registry's start, whose sign-in is the slot's).
pub fn seed_folder(
    paths: &ClaudePaths,
    id: &str,
    groups: &[String],
    identity: bool,
) -> Result<(), String> {
    let dir = paths.account_dir(id);
    super::registry::make_private_dir(&paths.root)?;
    if !dir.is_dir() {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|error| format!("Could not make {} ({error}).", dir.display()))?;
    }
    if let Err(error) = super::shared::link(&dir, &paths.claude_home, groups) {
        tracing::warn!(account = %id, %error, "shared items not linked");
    }
    let slot = read_config(&paths.default_json)
        .ok()
        .flatten()
        .unwrap_or_default();
    update_config(&paths.account_json(id), |config| {
        let mut changed = carry_session_keys(&slot, config);
        if identity {
            if let Some(record) = slot.get(IDENTITY_KEY) {
                if config.get(IDENTITY_KEY) != Some(record) {
                    config.insert(IDENTITY_KEY.into(), record.clone());
                    changed = true;
                }
            }
        }
        changed
    })
    .map_err(|error| error.sentence())
}

/// The swap of G3: make `to` (X) the global account while `from` (G) is.
/// The caller made sure nothing runs on X's folder (G4) and that no other
/// swap runs in this process; a journal left by another one is `Busy`.
/// `commit` writes the registry (step 7) while the slot is still locked.
///
/// An error that is not a crash leaves the stores as one account's (G7):
/// before X's chain is in the slot, G's copy in its folder goes again and
/// nothing changed; once it is, the journal stays and the next look
/// ([`recover`]) finishes the swap. A journal that can't be settled now is
/// kept for that look too, never dropped with a chain in two stores.
pub fn swap(
    stores: &Stores<'_>,
    from: &str,
    to: &str,
    record: Option<AccountSwitch>,
    commit: &dyn Fn(&Journal) -> Result<(), String>,
    crash: Crash,
) -> Result<(), SwapError> {
    if from == to {
        return Err(SwapError::Failed(
            "It is the global account already.".into(),
        ));
    }
    // One swap at a time across processes too.
    let _hold = stores.hold_swap()?;
    if stores.read_journal().is_some() {
        return Err(SwapError::Busy);
    }
    let (g, x) = (
        Place::Folder(from.to_string()),
        Place::Folder(to.to_string()),
    );
    // Step 0: X has a sign-in to move; the owner gate is off.
    let x_raw = stores.read(&x).map_err(SwapError::Failed)?;
    if !signed_in(x_raw.as_ref()) {
        return Err(SwapError::NotSignedIn);
    }
    let slot_config = read_config(&stores.paths.default_json).map_err(SwapError::Failed)?;
    if owner_gate_on(slot_config.as_ref()) {
        return Err(SwapError::OwnerCheckOn);
    }
    let journal = Journal {
        from: from.to_string(),
        to: to.to_string(),
        step: 0,
        x_rt: chain_id(x_raw.as_ref()),
        g_rt: None,
        locks: Vec::new(),
        record,
        aborted: false,
    };
    stores.write_journal(&journal)?;
    // Step 1: the locks.
    let (locks, storage) = match stores.lock_all(&[&g, &x]) {
        Ok(taken) => taken,
        Err(error) => {
            stores.drop_journal();
            return Err(error);
        }
    };
    let mut run = Run::new(stores, journal, locks, storage, crash);
    // X's chain as it is under the locks: the one the slot gets.
    match run.read(&x) {
        Ok(raw) if signed_in(raw.as_ref()) => run.journal.x_rt = chain_id(raw.as_ref()),
        Ok(_) => return Err(run.give_up(SwapError::NotSignedIn)),
        Err(error) => return Err(run.give_up(error)),
    }
    if let Err(error) = run.note(1) {
        return Err(run.give_up(error));
    }
    if crash == Crash::After(1) {
        return Err(run.crash());
    }
    // Step 2: the slot still holds G.
    let g_uuid = stores.uuid(&g);
    let slot_uuid = stores.uuid(&Place::Slot);
    if g_uuid.is_some() && slot_uuid.is_some() && slot_uuid != g_uuid {
        return Err(run.give_up(SwapError::Identity(slot_uuid)));
    }
    match run.finish(commit) {
        Ok(()) => {
            run.done();
            Ok(())
        }
        Err(SwapError::Failed(text)) if text == CRASH => Err(run.crash()),
        Err(error) => Err(run.settle(error)),
    }
}

/// What a test's crash says (the worker died there).
const CRASH: &str = "crash";

fn signed_in(raw: Option<&Raw>) -> bool {
    raw.map(super::credentials::credentials_of)
        .is_some_and(|c| c.signed_in())
}

/// The account keys of a store (see [`ACCOUNT_KEYS`]).
fn account_part(raw: Option<&Raw>) -> Map<String, Value> {
    let mut part = Map::new();
    if let Some(raw) = raw {
        for key in ACCOUNT_KEYS {
            if let Some(value) = raw.get(key) {
                part.insert(key.to_string(), value.clone());
            }
        }
    }
    part
}

/// `raw` with its account keys replaced by `part`'s, the rest kept.
fn with_account_part(mut raw: Raw, part: &Map<String, Value>) -> Raw {
    for key in ACCOUNT_KEYS {
        raw.remove(key);
    }
    for (key, value) in part {
        raw.insert(key.clone(), value.clone());
    }
    raw
}

/// A swap under way: its journal and its locks, touched before every
/// store call (a Keychain call can take seconds, and Claude Code takes
/// `.storage-write.lock` over once it has gone 15 s untouched).
struct Run<'s, 'a> {
    stores: &'s Stores<'a>,
    journal: Journal,
    locks: LockSet,
    /// The slot's store lock: released once the stores are done.
    storage: PathBuf,
    crash: Crash,
    /// Since when every lock is held.
    since: Instant,
}

impl<'s, 'a> Run<'s, 'a> {
    fn new(
        stores: &'s Stores<'a>,
        journal: Journal,
        mut locks: LockSet,
        storage: PathBuf,
        crash: Crash,
    ) -> Self {
        locks.touch();
        Self {
            stores,
            journal,
            locks,
            storage,
            crash,
            since: Instant::now(),
        }
    }

    /// Touch the locks; past [`Stores::budget`], stop (the stores are left
    /// as the journal says, for [`swap`]'s settling or the next look).
    fn fresh(&mut self) -> Result<(), SwapError> {
        self.locks.touch();
        if self.since.elapsed() > self.stores.budget {
            return Err(SwapError::Failed(TOO_SLOW.into()));
        }
        Ok(())
    }

    fn read(&mut self, place: &Place) -> Result<Option<Raw>, SwapError> {
        self.fresh()?;
        self.stores.read(place).map_err(SwapError::Failed)
    }

    fn write(&mut self, place: &Place, raw: &Raw) -> Result<(), SwapError> {
        self.fresh()?;
        self.stores.write(place, raw).map_err(SwapError::Failed)
    }

    /// Step `step` is done: the locks are touched and the journal says so,
    /// with the locks as they are now.
    fn note(&mut self, step: u8) -> Result<(), SwapError> {
        self.locks.touch();
        self.journal.locks = self.locks.ids();
        self.journal.step = step;
        self.stores.write_journal(&self.journal)
    }

    fn crash_at(&self, step: u8) -> bool {
        self.crash == Crash::After(step)
    }

    /// Steps 3–7 from where the journal is.
    fn finish(&mut self, commit: &dyn Fn(&Journal) -> Result<(), String>) -> Result<(), SwapError> {
        let (g, x) = (
            Place::Folder(self.journal.from.clone()),
            Place::Folder(self.journal.to.clone()),
        );
        if self.journal.step < 3 {
            // Step 3: the slot's sign-in is G's newest chain; G's folder gets
            // it (all of G's account keys), the rest of its store kept. A
            // sign-in Claude Code cleared moves too: G then reads "sign in
            // again".
            let slot_raw = self.read(&Place::Slot)?;
            let g_raw = self.read(&g)?.unwrap_or_default();
            // Noted before the write: from then on G's folder may hold it.
            self.journal.g_rt = chain_id(slot_raw.as_ref());
            self.write(
                &g,
                &with_account_part(g_raw, &account_part(slot_raw.as_ref())),
            )?;
            self.note(3)?;
            if self.crash_at(3) {
                return Err(SwapError::Failed(CRASH.into()));
            }
        }
        if self.journal.step < 4 {
            // Step 4: X's account keys into the slot, replacing only those.
            let x_raw = self.read(&x)?;
            if !signed_in(x_raw.as_ref()) {
                return Err(SwapError::NotSignedIn);
            }
            if chain_id(x_raw.as_ref()) != self.journal.x_rt {
                return Err(SwapError::Failed(
                    "The sign-in to move changed since the swap began.".into(),
                ));
            }
            let slot_raw = self.read(&Place::Slot)?.unwrap_or_default();
            self.write(
                &Place::Slot,
                &with_account_part(slot_raw, &account_part(x_raw.as_ref())),
            )?;
            self.note(4)?;
            if self.crash_at(4) {
                return Err(SwapError::Failed(CRASH.into()));
            }
        }
        if self.journal.step < 5 {
            // Step 5: X's chain lives in the slot alone. Only that very
            // chain goes from X's folder: another one there (X signed in on
            // its folder meanwhile) is a sign-in of its own, and stays.
            if let Some(raw) = self.read(&x)? {
                let chain = chain_id(Some(&raw));
                if chain.is_some() && chain == self.journal.x_rt {
                    self.write(&x, &with_account_part(raw, &Map::new()))?;
                } else if chain.is_some() {
                    tracing::warn!(account = %self.journal.to, "its folder holds another sign-in of it now; kept");
                }
            }
            self.note(5)?;
            if self.crash_at(5) {
                return Err(SwapError::Failed(CRASH.into()));
            }
        }
        // The stores are done: their locks go (the slot's and the folders').
        let storage = self.storage.clone();
        self.locks.release_from(&storage);
        if self.journal.step < 6 {
            // Step 6: `~/.claude.json` says X, as after `/login`; G's folder
            // keeps G's record (it may have gained fields while G was global).
            swap_records(self.stores, &self.journal.from, &self.journal.to, true)?;
            self.note(6)?;
            if self.crash_at(6) {
                return Err(SwapError::Failed(CRASH.into()));
            }
        }
        // Step 7: the registry, while the refresh locks are still held.
        commit(&self.journal).map_err(SwapError::Failed)?;
        Ok(())
    }

    /// G's copy of the slot's chain leaves G's folder again: only the copy
    /// this swap made (`chain`), never another sign-in. Touches the locks
    /// but ignores the budget: it is what makes stopping safe.
    fn roll_back(&mut self, chain: Option<&String>) -> Result<(), SwapError> {
        let Some(chain) = chain else {
            return Ok(());
        };
        let g = Place::Folder(self.journal.from.clone());
        self.locks.touch();
        let Some(raw) = self.stores.read(&g).map_err(SwapError::Failed)? else {
            return Ok(());
        };
        if chain_id(Some(&raw)).as_ref() != Some(chain) {
            return Ok(());
        }
        self.locks.touch();
        self.stores
            .write(&g, &with_account_part(raw, &Map::new()))
            .map_err(SwapError::Failed)
    }

    /// Done: locks released, journal gone.
    fn done(self) {
        let Self { stores, locks, .. } = self;
        drop(locks);
        stores.drop_journal();
    }

    /// Nothing changed yet: as [`Run::done`], returning `error`.
    fn give_up(self, error: SwapError) -> SwapError {
        self.done();
        error
    }

    /// A test's crash: the locks and the journal stay, as a dead worker
    /// leaves them.
    fn crash(self) -> SwapError {
        let Self { locks, .. } = self;
        std::mem::forget(locks);
        SwapError::Failed(CRASH.into())
    }

    /// After an error that is not a crash (G7): the stores are left as one
    /// account's. Before X's chain is in the slot, G's copy leaves G's folder
    /// and the journal goes; once it is, the journal stays for the next
    /// look, which finishes the swap. What can't be told now (the slot
    /// unreadable, the copy not removed) keeps the journal, marked
    /// `aborted`: that look rolls back unless the slot holds X.
    fn settle(mut self, error: SwapError) -> SwapError {
        let slot = self
            .stores
            .read(&Place::Slot)
            .map(|raw| chain_id(raw.as_ref()));
        match slot {
            Ok(Some(chain)) if Some(&chain) == self.journal.x_rt.as_ref() => {
                tracing::warn!(from = %self.journal.from, to = %self.journal.to, error = %error.sentence(), "the swap stopped with the new sign-in in place; it is finished on the next look");
            }
            // Nothing written yet.
            Ok(_) if self.journal.g_rt.is_none() => self.done(),
            Ok(Some(chain)) if Some(&chain) == self.journal.g_rt.as_ref() => {
                let copy = self.journal.g_rt.clone();
                match self.roll_back(copy.as_ref()) {
                    Ok(()) => self.done(),
                    Err(undo) => self.keep_aborted(&undo),
                }
            }
            Ok(_) => self.keep_aborted(&SwapError::Failed(
                "The slot holds another sign-in now.".into(),
            )),
            Err(read) => self.keep_aborted(&SwapError::Failed(read)),
        }
        error
    }

    fn keep_aborted(mut self, why: &SwapError) {
        tracing::warn!(from = %self.journal.from, to = %self.journal.to, why = %why.sentence(), "the stopped swap is settled on the next look");
        self.journal.aborted = true;
        if let Err(error) = self.stores.write_journal(&self.journal) {
            tracing::warn!(error = %error.sentence(), "the swap's journal was not updated");
        }
    }
}

/// Step 6: `~/.claude.json`'s `oauthAccount` becomes `to`'s record (from
/// its folder), the slot's current record is saved into `from`'s folder
/// (only when it is `from`'s, or `from`'s folder has none), and with
/// `clear` the caches `/login` clears go too. Also used to put `to`'s
/// record back when a session wrote `from`'s profile over it (G3 step 8).
pub fn swap_records(
    stores: &Stores<'_>,
    from: &str,
    to: &str,
    clear: bool,
) -> Result<(), SwapError> {
    let (g, x) = (
        Place::Folder(from.to_string()),
        Place::Folder(to.to_string()),
    );
    let to_record = read_config(&stores.json(&x))
        .ok()
        .flatten()
        .and_then(|config| config.get(IDENTITY_KEY).cloned());
    let g_uuid = stores.uuid(&g);
    // What the slot says of G, saved into G's folder: the record only when
    // it is G's (or G's folder has none yet), the session keys always.
    let mut saved: Option<(Map<String, Value>, bool)> = None;
    update_config(&stores.paths.default_json, |config| {
        let current_uuid = account_uuid(config);
        let is_g = current_uuid.is_some() && (g_uuid.is_none() || current_uuid == g_uuid);
        if current_uuid.is_none() || is_g {
            saved = Some((config.clone(), is_g));
        }
        match &to_record {
            Some(record) => config.insert(IDENTITY_KEY.into(), record.clone()),
            None => config.remove(IDENTITY_KEY),
        };
        if clear {
            for key in LOGIN_CACHE_KEYS {
                config.remove(key);
            }
        }
        true
    })?;
    if let Some((slot, with_record)) = saved {
        update_config(&stores.json(&g), |config| {
            let mut changed = carry_session_keys(&slot, config);
            if let Some(record) = slot.get(IDENTITY_KEY).filter(|_| with_record) {
                if config.get(IDENTITY_KEY) != Some(record) {
                    config.insert(IDENTITY_KEY.into(), record.clone());
                    changed = true;
                }
            }
            changed
        })?;
    }
    Ok(())
}

/// A swap was left half-way with the new account's chain in the slot
/// (G7): its panes belong on the slot until [`recover`] finishes it.
pub fn holds_new_chain(stores: &Stores<'_>) -> bool {
    let Some(journal) = stores.read_journal() else {
        return false;
    };
    let slot = stores.read(&Place::Slot).ok().flatten();
    journal.x_rt.is_some() && chain_id(slot.as_ref()) == journal.x_rt
}

/// Finish or undo a swap a stopped worker (or a failed step) left (G7),
/// under the same locks, deciding from what the stores hold, never from
/// the journal's step alone:
///
/// - the slot holds X's chain (`x_rt`): steps 5–7 (X's folder loses only
///   that very chain);
/// - the slot holds G's chain (`g_rt`, or before step 3 any chain under
///   G's profile): steps 3–7 with the slot read again, or, for a swap that
///   failed (`aborted`) or an X no longer signed in, G's copy leaves G's
///   folder and nothing else changes;
/// - the slot holds another chain under X's profile (renewed after step
///   6, or X signed in by hand): steps 5–7;
/// - anything else (someone else's profile on the slot, another chain
///   under G's, no sign-in): a sign-in by hand. The journal goes and
///   nothing is touched: G6 takes it from there.
///
/// Returns the journal finished, if any.
pub fn recover(
    stores: &Stores<'_>,
    commit: &dyn Fn(&Journal) -> Result<(), String>,
) -> Result<Option<Journal>, SwapError> {
    if stores.read_journal().is_none() {
        return Ok(None);
    }
    // A journal is left half-way only once no process holds the swap lock:
    // until then another worker's swap runs, and its locks are its own.
    let _hold = stores.hold_swap()?;
    let Some(journal) = stores.read_journal() else {
        return Ok(None);
    };
    tracing::warn!(from = %journal.from, to = %journal.to, step = journal.step, aborted = journal.aborted, "settling a swap of the global account left half-way");
    lockdir::reclaim(&journal.locks);
    let (g, x) = (
        Place::Folder(journal.from.clone()),
        Place::Folder(journal.to.clone()),
    );
    let (locks, storage) = stores.lock_all(&[&g, &x])?;
    let mut run = Run::new(stores, journal, locks, storage, Crash::None);
    let slot_chain = chain_id(run.read(&Place::Slot)?.as_ref());
    let x_raw = run.read(&x)?;
    let x_chain = chain_id(x_raw.as_ref());
    let g_chain = chain_id(run.read(&g)?.as_ref());
    let (slot_uuid, g_uuid, x_uuid) = (stores.uuid(&Place::Slot), stores.uuid(&g), stores.uuid(&x));
    // A profile on the slot that is neither (both records known).
    let not_g = slot_uuid.is_some() && g_uuid.is_some() && slot_uuid != g_uuid;
    let not_x = slot_uuid.is_some() && x_uuid.is_some() && slot_uuid != x_uuid;
    let journal = &run.journal;
    let slot_is_x = slot_chain.is_some() && slot_chain == journal.x_rt;
    let slot_is_g = slot_chain.is_some()
        && (slot_chain == journal.g_rt || (journal.g_rt.is_none() && !slot_is_x));
    if slot_is_x && !(not_g && not_x) {
        run.journal.step = run.journal.step.max(4);
    } else if slot_is_g && !not_g {
        // The copy step 3 made (or may have made, unnoted): the slot's.
        let copy = slot_chain.clone();
        if run.journal.aborted || !signed_in(x_raw.as_ref()) {
            tracing::warn!(from = %run.journal.from, to = %run.journal.to, "the swap is undone: the global account stays");
            run.roll_back(copy.as_ref())?;
            run.done();
            return Ok(None);
        }
        if x_chain != run.journal.x_rt {
            tracing::info!(account = %run.journal.to, "its sign-in changed since the swap began; the newest one moves");
            run.journal.x_rt = x_chain;
        }
        run.journal.step = if g_chain.is_some() && g_chain == copy {
            3
        } else {
            2
        };
        run.journal.g_rt = if run.journal.step == 3 { copy } else { None };
        run.note(run.journal.step)?;
    } else if slot_chain.is_some() && x_uuid.is_some() && !not_x {
        run.journal.step = run.journal.step.max(5);
        // X's folder still holding the chain the swap moved loses it.
        if let Some(raw) = x_raw {
            if x_chain.is_some() && x_chain == run.journal.x_rt {
                run.write(&x, &with_account_part(raw, &Map::new()))?;
            }
        }
    } else {
        tracing::warn!(from = %run.journal.from, to = %run.journal.to, "someone signed in by hand on the slot since; the swap is dropped and nothing moved");
        run.done();
        return Ok(None);
    }
    run.finish(commit)?;
    let journal = run.journal.clone();
    run.done();
    Ok(Some(journal))
}

/// How long after a swap a session may write the old account's profile
/// back (it fetched it just before), when the worker puts the new one back
/// (G3 step 8) instead of taking it for a sign-in by hand (G6).
pub const WRITE_BACK_WINDOW: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::credentials::FileStore;
    use crate::accounts::test_support::Home;

    fn chain(token: &str) -> Value {
        serde_json::json!({"accessToken": format!("at-{token}"), "refreshToken": format!("rt-{token}"),
            "expiresAt": 4_102_444_800_000u64, "scopes": ["user:inference"], "subscriptionType": "max"})
    }

    fn record(uuid: &str, email: &str) -> Value {
        serde_json::json!({"accountUuid": uuid, "emailAddress": email, "organizationUuid": format!("org-{uuid}")})
    }

    /// A home whose slot holds G (`default`) and whose `personal` folder
    /// holds X, both with records.
    fn setup(home: &Home) {
        let paths = &home.paths;
        std::fs::create_dir_all(&paths.claude_home).unwrap();
        std::fs::write(
            paths.claude_home.join(".credentials.json"),
            serde_json::json!({"claudeAiOauth": chain("g"), "mcpOAuth": {"srv": {"accessToken": "mcp"}}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            &paths.default_json,
            serde_json::json!({"oauthAccount": record("uuid-g", "g@x.dev"), "numStartups": 4,
                "hasCompletedOnboarding": true, "cachedUsageUtilization": {"fetchedAtMs": 1},
                "modelAccessCache": {"a": 1}, "projects": {"/p": {"hasTrustDialogAccepted": true, "lastCost": 2}}})
            .to_string(),
        )
        .unwrap();
        seed_folder(paths, "default", &[], true).unwrap();
        let x = paths.account_dir("personal");
        std::fs::create_dir_all(&x).unwrap();
        std::fs::write(
            x.join(".credentials.json"),
            serde_json::json!({"claudeAiOauth": chain("x"), "mcpOAuth": {"own": 1}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            paths.account_json("personal"),
            serde_json::json!({"oauthAccount": record("uuid-x", "x@x.dev")}).to_string(),
        )
        .unwrap();
    }

    fn raw(path: &Path) -> Map<String, Value> {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn token_of(dir: &Path) -> Option<String> {
        raw(&dir.join(".credentials.json"))
            .get(OAUTH_KEY)
            .and_then(|c| c.get("refreshToken"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    fn no_commit(_: &Journal) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn a_swap_moves_both_chains_and_the_records() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let home = Home::new("swap");
        setup(&home);
        let paths = &home.paths;
        let stores = Stores {
            paths,
            secrets: &FileStore,
            wait: Duration::from_millis(300),
            budget: HOLD_BUDGET,
        };
        let slot_file = paths.claude_home.join(".credentials.json");
        let before = std::fs::metadata(&slot_file).unwrap();
        let committed = std::cell::Cell::new(false);
        let commit = |journal: &Journal| {
            assert_eq!(
                (journal.from.as_str(), journal.to.as_str()),
                ("default", "personal")
            );
            // Still under the refresh lock while the registry is written.
            assert!(paths.claude_home.join(".oauth_refresh.lock").is_dir());
            assert!(!paths.claude_home.join(".storage-write.lock").exists());
            committed.set(true);
            Ok(())
        };
        swap(&stores, "default", "personal", None, &commit, Crash::None).unwrap();
        assert!(committed.get());
        // The slot holds X's chain, its other keys kept, a new private file.
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-x"));
        let slot = raw(&slot_file);
        assert_eq!(
            slot["mcpOAuth"],
            serde_json::json!({"srv": {"accessToken": "mcp"}})
        );
        let after = std::fs::metadata(&slot_file).unwrap();
        assert_ne!(before.ino(), after.ino());
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        // G's folder holds G's chain; X's folder none, its other keys kept.
        assert_eq!(
            token_of(&paths.account_dir("default")).as_deref(),
            Some("rt-g")
        );
        let x = raw(&paths.account_dir("personal").join(".credentials.json"));
        assert!(x.get(OAUTH_KEY).is_none());
        assert_eq!(x["mcpOAuth"], serde_json::json!({"own": 1}));
        // ~/.claude.json says X, with /login's caches gone and the rest kept.
        let config = raw(&paths.default_json);
        assert_eq!(account_uuid(&config).as_deref(), Some("uuid-x"));
        assert!(config.get("cachedUsageUtilization").is_none());
        assert!(config.get("modelAccessCache").is_none());
        assert_eq!(config["numStartups"], 4);
        // G's folder keeps G's record.
        let g = raw(&paths.account_json("default"));
        assert_eq!(account_uuid(&g).as_deref(), Some("uuid-g"));
        assert_eq!(g["projects"]["/p"]["hasTrustDialogAccepted"], true);
        // No lock, no journal left.
        for name in [".oauth_refresh.lock", ".storage-write.lock"] {
            assert!(!paths.claude_home.join(name).exists());
            assert!(!paths.account_dir("personal").join(name).exists());
        }
        assert!(!lockdir::legacy_path(&paths.claude_home).exists());
        assert!(stores.read_journal().is_none());
        // And back: G's chain returns to the slot.
        swap(
            &stores,
            "personal",
            "default",
            None,
            &no_commit,
            Crash::None,
        )
        .unwrap();
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-g"));
        assert_eq!(
            token_of(&paths.account_dir("personal")).as_deref(),
            Some("rt-x")
        );
        assert!(raw(&paths.account_dir("default").join(".credentials.json"))
            .get(OAUTH_KEY)
            .is_none());
        assert_eq!(
            account_uuid(&raw(&paths.default_json)).as_deref(),
            Some("uuid-g")
        );
        assert_eq!(
            account_uuid(&raw(&paths.account_json("personal"))).as_deref(),
            Some("uuid-x")
        );
    }

    #[test]
    fn a_swap_is_refused_without_a_sign_in_with_the_gate_on_or_while_locked() {
        let home = Home::new("refused");
        setup(&home);
        let paths = &home.paths;
        let stores = Stores {
            paths,
            secrets: &FileStore,
            wait: Duration::from_millis(300),
            budget: HOLD_BUDGET,
        };
        assert_eq!(
            swap(&stores, "default", "nobody", None, &no_commit, Crash::None),
            Err(SwapError::NotSignedIn)
        );
        // The owner gate (G8).
        let mut config = raw(&paths.default_json);
        config.insert(
            "cachedGrowthBookFeatures".into(),
            serde_json::json!({OWNER_GATE: true}),
        );
        write_config(&paths.default_json, &config).unwrap();
        assert_eq!(
            swap(
                &stores,
                "default",
                "personal",
                None,
                &no_commit,
                Crash::None
            ),
            Err(SwapError::OwnerCheckOn)
        );
        config.insert(
            "cachedGrowthBookFeatures".into(),
            serde_json::json!({OWNER_GATE: false}),
        );
        write_config(&paths.default_json, &config).unwrap();
        // Claude Code renewing its sign-in holds the refresh lock.
        std::fs::create_dir(paths.claude_home.join(".oauth_refresh.lock")).unwrap();
        let started = Instant::now();
        assert_eq!(
            swap(
                &stores,
                "default",
                "personal",
                None,
                &no_commit,
                Crash::None
            ),
            Err(SwapError::Busy)
        );
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(stores.read_journal().is_none());
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-g"));
        assert!(
            paths.claude_home.join(".oauth_refresh.lock").is_dir(),
            "theirs stays"
        );
        assert!(
            !lockdir::legacy_path(&paths.claude_home).exists(),
            "ours went"
        );
        std::fs::remove_dir(paths.claude_home.join(".oauth_refresh.lock")).unwrap();
        // Someone signed in by hand on the slot: resolved first (G6).
        let mut config = raw(&paths.default_json);
        config.insert(IDENTITY_KEY.into(), record("uuid-z", "z@x.dev"));
        write_config(&paths.default_json, &config).unwrap();
        assert_eq!(
            swap(
                &stores,
                "default",
                "personal",
                None,
                &no_commit,
                Crash::None
            ),
            Err(SwapError::Identity(Some("uuid-z".into())))
        );
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-g"));
        assert!(stores.read_journal().is_none());
    }

    #[test]
    fn a_swap_another_worker_runs_is_left_alone() {
        let home = Home::new("other-worker");
        setup(&home);
        let paths = &home.paths;
        let stores = Stores {
            paths,
            secrets: &FileStore,
            wait: Duration::from_millis(300),
            budget: HOLD_BUDGET,
        };
        // Another worker process is half-way through a swap: its journal,
        // its locks, and the swap lock it holds.
        let theirs = stores.hold_swap().unwrap();
        assert!(swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None
        )
        .is_err_and(|error| error == SwapError::Busy));
        drop(theirs);
        swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(3),
        )
        .unwrap_err();
        let theirs = stores.hold_swap().unwrap();
        assert_eq!(recover(&stores, &no_commit), Err(SwapError::Busy));
        assert!(stores.read_journal().is_some(), "its journal stays");
        assert!(
            paths.claude_home.join(".oauth_refresh.lock").is_dir(),
            "its locks stay"
        );
        // Its process ended (crashed): the swap is settled.
        drop(theirs);
        assert!(recover(&stores, &no_commit).unwrap().is_some());
        assert!(stores.read_journal().is_none());
        assert!(!paths.claude_home.join(".oauth_refresh.lock").exists());
    }

    #[test]
    fn a_swap_stopped_after_any_step_is_finished_by_the_next_start() {
        for step in [1u8, 3, 4, 5, 6] {
            let home = Home::new("crash");
            setup(&home);
            let paths = &home.paths;
            let stores = Stores {
                paths,
                secrets: &FileStore,
                wait: Duration::from_millis(300),
                budget: HOLD_BUDGET,
            };
            let result = swap(
                &stores,
                "default",
                "personal",
                None,
                &no_commit,
                Crash::After(step),
            );
            assert!(result.is_err());
            let journal = stores.read_journal().expect("the journal stays");
            assert_eq!(journal.step, step);
            assert!(
                paths.claude_home.join(".oauth_refresh.lock").is_dir(),
                "the dead worker's lock stays"
            );
            let text = std::fs::read_to_string(paths.root.join(JOURNAL)).unwrap();
            assert!(
                !text.contains("rt-x") && !text.contains("at-g"),
                "no token in the journal"
            );
            let committed = std::cell::Cell::new(false);
            let commit = |journal: &Journal| {
                assert_eq!(journal.to, "personal");
                committed.set(true);
                Ok(())
            };
            let started = Instant::now();
            let done = recover(&stores, &commit).unwrap().expect("finished");
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "its own locks are reclaimed"
            );
            assert_eq!(done.to, "personal");
            assert!(committed.get(), "step {step}");
            assert_eq!(
                token_of(&paths.claude_home).as_deref(),
                Some("rt-x"),
                "step {step}"
            );
            assert_eq!(
                token_of(&paths.account_dir("default")).as_deref(),
                Some("rt-g"),
                "step {step}"
            );
            assert!(
                raw(&paths.account_dir("personal").join(".credentials.json"))
                    .get(OAUTH_KEY)
                    .is_none()
            );
            assert_eq!(
                account_uuid(&raw(&paths.default_json)).as_deref(),
                Some("uuid-x"),
                "step {step}"
            );
            assert_eq!(
                account_uuid(&raw(&paths.account_json("default"))).as_deref(),
                Some("uuid-g")
            );
            assert!(stores.read_journal().is_none());
            assert!(!paths.claude_home.join(".oauth_refresh.lock").exists());
        }
    }

    fn stores_of<'a>(paths: &'a ClaudePaths, secrets: &'a dyn SecretStore) -> Stores<'a> {
        Stores {
            paths,
            secrets,
            wait: Duration::from_millis(300),
            budget: HOLD_BUDGET,
        }
    }

    fn set_slot_chain(paths: &ClaudePaths, token: &str) {
        let mut slot = raw(&paths.claude_home.join(".credentials.json"));
        slot.insert(OAUTH_KEY.into(), chain(token));
        std::fs::write(
            paths.claude_home.join(".credentials.json"),
            Value::Object(slot).to_string(),
        )
        .unwrap();
    }

    fn set_slot_record(paths: &ClaudePaths, uuid: &str) {
        let mut config = raw(&paths.default_json);
        config.insert(IDENTITY_KEY.into(), record(uuid, "someone@x.dev"));
        write_config(&paths.default_json, &config).unwrap();
    }

    #[test]
    fn recovery_decides_from_the_stores_not_the_step() {
        // Stopped after step 3, nothing changed since: steps 4–7.
        let home = Home::new("after-3");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        let _ = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(3),
        );
        recover(&stores, &no_commit).unwrap().unwrap();
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-x"));
        assert_eq!(
            token_of(&paths.account_dir("default")).as_deref(),
            Some("rt-g")
        );

        // Stopped after step 4, then someone signed in by hand as Z (the
        // locks went stale): X's chain is gone from the slot, so its
        // folder's copy is its only one. Nothing is touched; G6 takes Z.
        let home = Home::new("after-4-login");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        let _ = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(4),
        );
        set_slot_record(paths, "uuid-z");
        set_slot_chain(paths, "z");
        let committed = std::cell::Cell::new(false);
        let commit = |_: &Journal| {
            committed.set(true);
            Ok(())
        };
        assert_eq!(recover(&stores, &commit).unwrap(), None);
        assert!(!committed.get());
        assert!(stores.read_journal().is_none());
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-z"));
        assert_eq!(
            account_uuid(&raw(&paths.default_json)).as_deref(),
            Some("uuid-z")
        );
        assert_eq!(
            token_of(&paths.account_dir("personal")).as_deref(),
            Some("rt-x"),
            "X's last chain kept"
        );
        assert!(!paths.claude_home.join(".oauth_refresh.lock").exists());

        // Halfway through that /login (Z's profile written, its chain not
        // yet): the slot still holds X's chain, but the swap is not finished
        // over Z's profile.
        let home = Home::new("after-4-halfway");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        let _ = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(4),
        );
        set_slot_record(paths, "uuid-z");
        assert_eq!(recover(&stores, &no_commit).unwrap(), None);
        assert_eq!(
            account_uuid(&raw(&paths.default_json)).as_deref(),
            Some("uuid-z")
        );
        assert_eq!(
            token_of(&paths.account_dir("personal")).as_deref(),
            Some("rt-x")
        );

        // Stopped after step 4, then another chain under G's profile: a
        // renewal or a /login as G can't be told apart. Nothing is touched.
        let home = Home::new("after-4-other");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        let _ = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(4),
        );
        set_slot_chain(paths, "g-again");
        assert_eq!(recover(&stores, &no_commit).unwrap(), None);
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-g-again"));
        assert_eq!(
            token_of(&paths.account_dir("personal")).as_deref(),
            Some("rt-x")
        );
        assert_eq!(
            token_of(&paths.account_dir("default")).as_deref(),
            Some("rt-g")
        );

        // Stopped after step 6, then X's chain renewed in the slot (its
        // profile is X's): finished, X's folder losing the old copy.
        let home = Home::new("after-6-renewed");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        let _ = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(4),
        );
        // Steps 5 and 6 as a crash after 6 leaves them, then the renewal.
        let mut journal = stores.read_journal().unwrap();
        swap_records(&stores, "default", "personal", true).unwrap();
        journal.step = 6;
        stores.write_journal(&journal).unwrap();
        set_slot_chain(paths, "x2");
        let done = recover(&stores, &no_commit).unwrap();
        assert!(done.is_some());
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-x2"));
        assert!(
            raw(&paths.account_dir("personal").join(".credentials.json"))
                .get(OAUTH_KEY)
                .is_none()
        );

        // Stopped after step 1, then someone signed in by hand: dropped.
        let home = Home::new("dropped");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        let _ = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::After(1),
        );
        set_slot_record(paths, "uuid-z");
        assert_eq!(recover(&stores, &no_commit).unwrap(), None);
        assert!(stores.read_journal().is_none());
        assert_eq!(
            token_of(&paths.account_dir("personal")).as_deref(),
            Some("rt-x")
        );
    }

    /// A store whose writes into chosen folders fail (a full disk, a
    /// Keychain that times out), after letting `skip` through, and whose
    /// calls can each take `slow`.
    struct Flaky {
        fail: std::sync::Mutex<Vec<(PathBuf, u32, u32)>>,
        slow: Duration,
    }

    impl Flaky {
        fn new(fail: Vec<(PathBuf, u32, u32)>) -> Self {
            Self {
                fail: std::sync::Mutex::new(fail),
                slow: Duration::ZERO,
            }
        }
    }

    impl SecretStore for Flaky {
        fn read_raw(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Raw>, String> {
            std::thread::sleep(self.slow);
            FileStore.read_raw(dir, env_dir)
        }

        fn write_raw(&self, dir: &Path, env_dir: Option<&str>, value: &Raw) -> Result<(), String> {
            std::thread::sleep(self.slow);
            let mut fail = self.fail.lock().unwrap();
            if let Some(entry) = fail
                .iter_mut()
                .find(|(at, _, times)| at == dir && *times > 0)
            {
                if entry.1 > 0 {
                    entry.1 -= 1;
                } else {
                    entry.2 -= 1;
                    return Err("No space left on device (os error 28)".into());
                }
            }
            drop(fail);
            FileStore.write_raw(dir, env_dir, value)
        }
    }

    fn unchanged(paths: &ClaudePaths) {
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-g"));
        assert_eq!(
            token_of(&paths.account_dir("personal")).as_deref(),
            Some("rt-x")
        );
        assert_eq!(
            chain_id(
                FileStore
                    .read_raw(&paths.account_dir("default"), None)
                    .unwrap()
                    .as_ref()
            ),
            None,
            "no copy of G's chain left in its folder"
        );
        assert_eq!(
            account_uuid(&raw(&paths.default_json)).as_deref(),
            Some("uuid-g")
        );
    }

    fn finished(paths: &ClaudePaths) {
        assert_eq!(token_of(&paths.claude_home).as_deref(), Some("rt-x"));
        assert_eq!(
            token_of(&paths.account_dir("default")).as_deref(),
            Some("rt-g")
        );
        assert!(
            raw(&paths.account_dir("personal").join(".credentials.json"))
                .get(OAUTH_KEY)
                .is_none()
        );
        assert_eq!(
            account_uuid(&raw(&paths.default_json)).as_deref(),
            Some("uuid-x")
        );
    }

    #[test]
    fn a_write_that_fails_leaves_one_account_in_each_store() {
        // Step 3 (G's folder): nothing changed, no journal.
        let home = Home::new("fail-3");
        setup(&home);
        let paths = &home.paths;
        let flaky = Flaky::new(vec![(paths.account_dir("default"), 0, 1)]);
        let stores = stores_of(paths, &flaky);
        assert!(swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None
        )
        .is_err());
        assert!(stores.read_journal().is_none());
        unchanged(paths);

        // Step 4 (the slot): G's copy leaves its folder again.
        let home = Home::new("fail-4");
        setup(&home);
        let paths = &home.paths;
        let flaky = Flaky::new(vec![(paths.claude_home.clone(), 0, 1)]);
        let stores = stores_of(paths, &flaky);
        let error = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None,
        )
        .unwrap_err();
        assert!(error.sentence().contains("No space left"), "{error:?}");
        assert!(stores.read_journal().is_none());
        unchanged(paths);
        assert!(!paths.claude_home.join(".storage-write.lock").exists());

        // Step 4, and the copy can't be removed either: the journal stays,
        // marked, and the next look undoes it.
        let home = Home::new("fail-4-undo");
        setup(&home);
        let paths = &home.paths;
        let flaky = Flaky::new(vec![
            (paths.claude_home.clone(), 0, 1),
            (paths.account_dir("default"), 1, 1),
        ]);
        let stores = stores_of(paths, &flaky);
        assert!(swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None
        )
        .is_err());
        let journal = stores.read_journal().expect("kept");
        assert!(journal.aborted);
        assert_eq!(
            token_of(&paths.account_dir("default")).as_deref(),
            Some("rt-g")
        );
        assert_eq!(recover(&stores, &no_commit).unwrap(), None);
        assert!(stores.read_journal().is_none());
        unchanged(paths);

        // Step 5 (X's folder): X is in the slot already, so the journal stays
        // and the next look finishes the swap; X's folder is never used
        // meanwhile (its chain is in the slot too).
        let home = Home::new("fail-5");
        setup(&home);
        let paths = &home.paths;
        let flaky = Flaky::new(vec![(paths.account_dir("personal"), 0, 1)]);
        let stores = stores_of(paths, &flaky);
        assert!(swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None
        )
        .is_err());
        assert!(stores.read_journal().is_some());
        assert!(holds_new_chain(&stores));
        let committed = std::cell::Cell::new(false);
        let commit = |_: &Journal| {
            committed.set(true);
            Ok(())
        };
        assert!(recover(&stores, &commit).unwrap().is_some());
        assert!(committed.get());
        finished(paths);

        // Step 6 (`~/.claude.json` can't be written): finished on the next
        // look too.
        let home = Home::new("fail-6");
        setup(&home);
        let paths = &home.paths;
        let stores = stores_of(paths, &FileStore);
        // `~/.claude.json` is a link to a file whose name leaves no room for
        // the temporary file a write makes beside it (over 255 bytes): it
        // reads, but can't be written, by root too.
        let folder = home.dir.join("elsewhere");
        std::fs::create_dir_all(&folder).unwrap();
        let long = folder.join("c".repeat(250));
        std::fs::rename(&paths.default_json, &long).unwrap();
        std::os::unix::fs::symlink(&long, &paths.default_json).unwrap();
        let failed = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None,
        );
        let short = folder.join("claude.json");
        std::fs::rename(&long, &short).unwrap();
        std::fs::remove_file(&paths.default_json).unwrap();
        std::os::unix::fs::symlink(&short, &paths.default_json).unwrap();
        assert!(failed.is_err());
        assert!(holds_new_chain(&stores));
        assert!(recover(&stores, &no_commit).unwrap().is_some());
        finished(paths);
        assert!(!holds_new_chain(&stores), "no journal left");
    }

    #[test]
    fn a_store_too_slow_stops_the_swap_safely() {
        let home = Home::new("slow");
        setup(&home);
        let paths = &home.paths;
        let mut slow = Flaky::new(Vec::new());
        slow.slow = Duration::from_millis(60);
        let stores = Stores {
            paths,
            secrets: &slow,
            wait: Duration::from_millis(300),
            budget: Duration::from_millis(150),
        };
        let error = swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None,
        )
        .unwrap_err();
        assert_eq!(error, SwapError::Failed(TOO_SLOW.into()));
        // Stopped before the slot changed: undone, nothing left.
        assert!(stores.read_journal().is_none());
        unchanged(paths);
        assert!(!paths.claude_home.join(".storage-write.lock").exists());
    }

    #[test]
    fn the_account_keys_move_with_the_account_and_mcp_sign_ins_stay() {
        let home = Home::new("keys");
        setup(&home);
        let paths = &home.paths;
        let mut slot = raw(&paths.claude_home.join(".credentials.json"));
        slot.insert(
            "designOauth".into(),
            serde_json::json!({"refreshToken": "d-g"}),
        );
        slot.insert("organizationUuid".into(), "org-g".into());
        std::fs::write(
            paths.claude_home.join(".credentials.json"),
            Value::Object(slot).to_string(),
        )
        .unwrap();
        let x_file = paths.account_dir("personal").join(".credentials.json");
        let mut x = raw(&x_file);
        x.insert(
            "cloudSessionsOauth".into(),
            serde_json::json!({"refreshToken": "c-x"}),
        );
        std::fs::write(&x_file, Value::Object(x).to_string()).unwrap();
        let stores = stores_of(paths, &FileStore);
        swap(
            &stores,
            "default",
            "personal",
            None,
            &no_commit,
            Crash::None,
        )
        .unwrap();
        let slot = raw(&paths.claude_home.join(".credentials.json"));
        assert_eq!(slot["cloudSessionsOauth"]["refreshToken"], "c-x");
        assert!(slot.get("designOauth").is_none());
        assert!(slot.get("organizationUuid").is_none());
        assert_eq!(
            slot["mcpOAuth"],
            serde_json::json!({"srv": {"accessToken": "mcp"}})
        );
        let g = raw(&paths.account_dir("default").join(".credentials.json"));
        assert_eq!(g["designOauth"]["refreshToken"], "d-g");
        assert_eq!(g["organizationUuid"], "org-g");
        assert!(g.get("mcpOAuth").is_none(), "MCP sign-ins stay in the slot");
        let x = raw(&x_file);
        assert!(x.get("cloudSessionsOauth").is_none());
        assert_eq!(x["mcpOAuth"], serde_json::json!({"own": 1}));
    }

    #[test]
    fn a_commit_that_fails_leaves_the_journal_for_the_next_try() {
        let home = Home::new("commit");
        setup(&home);
        let paths = &home.paths;
        let stores = Stores {
            paths,
            secrets: &FileStore,
            wait: Duration::from_millis(300),
            budget: HOLD_BUDGET,
        };
        let fail = |_: &Journal| Err("accounts.json is being changed".to_string());
        assert!(swap(&stores, "default", "personal", None, &fail, Crash::None).is_err());
        assert_eq!(stores.read_journal().unwrap().step, 6);
        assert!(
            !paths.claude_home.join(".oauth_refresh.lock").exists(),
            "released"
        );
        assert!(recover(&stores, &no_commit).unwrap().is_some());
        assert!(stores.read_journal().is_none());
    }

    #[test]
    fn the_session_keys_are_carried_and_nothing_is_lost() {
        let from: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "hasCompletedOnboarding": true, "lastOnboardingVersion": "2.1.322",
            "mcpServers": {"docs": {"command": "d"}},
            "projects": {"/a": {"hasTrustDialogAccepted": true, "lastCost": 1}, "/b": {"hasTrustDialogAccepted": false}},
            "numStartups": 9}))
        .unwrap();
        let mut to: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "mcpServers": {"own": {}}, "projects": {"/c": {"x": 1}}}))
        .unwrap();
        assert!(carry_session_keys(&from, &mut to));
        assert_eq!(to["hasCompletedOnboarding"], true);
        assert_eq!(
            to["mcpServers"],
            serde_json::json!({"own": {}}),
            "theirs kept"
        );
        assert_eq!(
            to["projects"]["/a"],
            serde_json::json!({"hasTrustDialogAccepted": true})
        );
        assert!(to["projects"].get("/b").is_none());
        assert_eq!(to["projects"]["/c"]["x"], 1);
        assert!(to.get("numStartups").is_none());
        assert!(!carry_session_keys(&from, &mut to), "nothing more to add");
    }

    #[test]
    fn a_symlinked_claude_json_is_followed_and_kept() {
        let home = Home::new("symlink");
        let real = home.dir.join("dotfiles/claude.json");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, r#"{"numStartups":1}"#).unwrap();
        std::os::unix::fs::symlink(&real, &home.paths.default_json).unwrap();
        update_config(&home.paths.default_json, |config| {
            config.insert("x".into(), Value::Bool(true));
            true
        })
        .unwrap();
        assert!(std::fs::symlink_metadata(&home.paths.default_json)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(raw(&real)["x"], true);
        assert!(!lockdir::config_lock_path(&home.paths.default_json).exists());
    }
}
