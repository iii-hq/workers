//! Claude Code accounts and the usage of Claude Code and Codex
//! (agent-accounts, D1–D19 of the brief; the README's "Claude Code
//! accounts" section is the user-facing account of it).
//!
//! One account is one Claude Code config folder (D1). The registry
//! (`accounts.json`, D2) names them, says which one is global (what plain
//! `claude` gets in new tmux windows, through tmux's global environment, D3)
//! and holds the rules for the global account (D14). Usage readings come
//! from Claude Code's usage endpoint per account (D10–D12) and from what
//! Codex records (D13); they are cached in the worker's cache folder, never
//! with a token.

pub mod codex;
pub mod credentials;
pub mod daemon;
pub mod launch;
pub mod manage;
pub mod moves;
pub mod panes;
pub mod paths;
pub mod registry;
pub mod rules;
pub mod shared;
pub mod time;
pub mod tmux_env;
pub mod usage;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::model::{now_ms, ClaudeAccount, ClaudeSnapshot, CodexUsage, UsageSnapshot};
use credentials::{Credentials, Profile, SecretStore};
use paths::ClaudePaths;
use registry::{Registry, Store};
use sha2::{Digest, Sha256};
use usage::{CacheFile, Reading, UsageClient};

/// The built-in account: `~/.claude`, `CLAUDE_CONFIG_DIR` unset.
pub const DEFAULT: &str = "default";
/// What a pane that follows the global account is marked with (D4).
pub const GLOBAL: &str = "global";

/// How long the app counts as watching after a `tmux::usage::read` or while
/// the live feed is observed (D12).
pub const WATCH_MS: u64 = 10 * 60_000;
/// How old the global account's reading may get while nobody watches, the
/// rules are on and another account could take over (O1).
pub const UNWATCHED_GLOBAL_EVERY_MS: u64 = 10 * 60_000;
/// How long Codex's records are not looked at again.
const CODEX_EVERY_MS: u64 = 30_000;
/// How long a sign-in read from the Keychain is kept (macOS asks `security`).
const KEYCHAIN_TTL_MS: u64 = 5 * 60_000;
/// How long "is Claude Code here" is kept.
const AVAILABLE_TTL_MS: u64 = 60_000;
/// How often the feed has sign-ins and profiles looked at again (in the
/// background).
const LOAD_EVERY_MS: u64 = 1_000;

type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| Some((meta.modified().ok()?, meta.len())))
}

/// The sign-in last read for an account.
struct CachedCredentials {
    /// Its `.credentials.json` and `.claude.json` when it was read.
    stamps: (Stamp, Stamp),
    read_at_ms: u64,
    value: Result<Option<Credentials>, String>,
    /// Since when it is to be read again on the next load, whatever the
    /// stamps and the Keychain's age say ([`Accounts::reread_credentials`]).
    /// `value` stands until then.
    reread_since: Option<u64>,
}

/// The `.claude.json` last read for an account.
struct CachedProfile {
    stamp: Stamp,
    value: Option<Profile>,
}

/// What the worker last read about the accounts. Filled by
/// [`Accounts::load_blocking`], which never holds the lock while it reads;
/// the snapshot only looks here.
#[derive(Default)]
struct State {
    credentials: HashMap<String, CachedCredentials>,
    profiles: HashMap<String, CachedProfile>,
    /// A digest of each account's token, in memory only: a new token lifts
    /// the endpoint's refusal (`rejected`), wherever the sign-in lives.
    token_digests: HashMap<String, [u8; 32]>,
    readings: HashMap<String, Reading>,
    readings_loaded: bool,
    codex: Option<(u64, Option<CodexUsage>)>,
    available: Option<(u64, bool)>,
    /// When the global account's current reason to switch was first seen.
    first_seen: HashMap<String, u64>,
    /// When the last load finished; 0 before the first.
    loaded_at_ms: u64,
}

/// One account as the snapshot needs it.
#[derive(Debug, Clone)]
pub struct AccountView {
    pub id: String,
    pub dir: PathBuf,
    pub signed_in: bool,
    pub token_expired: bool,
}

/// The accounts service: the registry, sign-ins, profiles and readings.
pub struct Accounts {
    paths: Option<ClaudePaths>,
    secrets: Box<dyn SecretStore>,
    keychain: bool,
    client: Option<UsageClient>,
    /// `accounts.json`, under a lock of its own (small file work only).
    store: Mutex<Option<Store>>,
    state: Mutex<State>,
    /// One load of sign-ins, profiles and Codex at a time.
    load_gate: Arc<tokio::sync::Mutex<()>>,
    watched_until_ms: AtomicU64,
    /// One round of readings at a time.
    reading: tokio::sync::Mutex<()>,
    /// The panes being moved now (one move at a time each).
    pub moving: moves::Moving,
    /// Some pane may carry `@iii_claude_pending` (§3.7) that no look has
    /// seen yet; true at start so the first look finds those a restart left.
    pending: AtomicBool,
    /// The panes whose move waits for their turn to end, with the Claude
    /// Code process last seen there.
    waiting: Mutex<HashMap<String, Option<u32>>>,
    /// Why the last move of a pane failed, by pane id.
    move_errors: Mutex<HashMap<String, String>>,
}

impl Accounts {
    /// From this process's environment and this platform's sign-in store.
    /// In the worker's own unit tests, a sandbox instead: no test reads
    /// this computer's Claude Code sign-in.
    pub fn from_env() -> Arc<Self> {
        #[cfg(test)]
        {
            let dir = std::env::temp_dir().join(format!(
                "tmux-accounts-sandbox-{}",
                uuid::Uuid::new_v4().simple()
            ));
            Self::sandbox(&dir)
        }
        #[cfg(not(test))]
        Arc::new(Self::new(
            ClaudePaths::from_env(),
            credentials::platform_store(),
            cfg!(target_os = "macos"),
        ))
    }

    /// Accounts that live in `dir` alone, for tests: its own home folder,
    /// `~/.claude`, accounts folder, Codex folder and cache, sign-ins read
    /// from files only (never a Keychain), and a usage client that never
    /// sends anything ([`Accounts::can_fetch`] is false).
    pub fn sandbox(dir: &Path) -> Arc<Self> {
        let home = dir.join("home");
        let vars: HashMap<&str, String> = HashMap::from([
            ("HOME", home.to_string_lossy().into_owned()),
            (
                "III_CLAUDE_HOME",
                home.join(".claude").to_string_lossy().into_owned(),
            ),
            (
                "III_CLAUDE_ACCOUNTS_DIR",
                dir.join("accounts").to_string_lossy().into_owned(),
            ),
            (
                "CODEX_HOME",
                home.join(".codex").to_string_lossy().into_owned(),
            ),
            (
                "XDG_CACHE_HOME",
                dir.join("cache").to_string_lossy().into_owned(),
            ),
        ]);
        let paths = ClaudePaths::from_vars(|key| vars.get(key).cloned(), false);
        let mut accounts = Self::new(paths, Box::new(credentials::FileStore), false);
        accounts.client = None;
        Arc::new(accounts)
    }

    /// `keychain`: sign-ins come from a store that is slow to ask, so they
    /// are kept for a while instead of read on every change.
    pub fn new(paths: Option<ClaudePaths>, secrets: Box<dyn SecretStore>, keychain: bool) -> Self {
        let client = paths
            .as_ref()
            .and_then(|paths| UsageClient::new(&paths.usage_origin));
        let store = paths
            .as_ref()
            .map(|paths| Store::new(paths.registry_file()));
        Self {
            paths,
            secrets,
            keychain,
            client,
            store: Mutex::new(store),
            state: Mutex::new(State::default()),
            load_gate: Arc::new(tokio::sync::Mutex::new(())),
            watched_until_ms: AtomicU64::new(0),
            reading: tokio::sync::Mutex::new(()),
            moving: moves::Moving::default(),
            pending: AtomicBool::new(true),
            waiting: Mutex::new(HashMap::new()),
            move_errors: Mutex::new(HashMap::new()),
        }
    }

    pub fn paths(&self) -> Option<&ClaudePaths> {
        self.paths.as_ref()
    }

    /// Whether readings can be asked for at all (a sandbox can't).
    pub fn can_fetch(&self) -> bool {
        self.client.is_some()
    }

    /// The origin readings would be asked from, if any.
    pub fn usage_origin(&self) -> Option<&str> {
        self.client.as_ref().map(UsageClient::origin)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Option<Store>> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The registry (the implicit one without `accounts.json`).
    pub fn registry(&self) -> Registry {
        match self.store().as_mut() {
            Some(store) => store.current(),
            None => Registry::default(),
        }
    }

    /// `accounts.json` exists and reads.
    pub fn managed(&self) -> bool {
        self.store().as_mut().is_some_and(Store::managed)
    }

    /// The registry with its generation (see [`Accounts::update_registry_at`]).
    pub fn registry_at(&self) -> (Registry, u64) {
        match self.store().as_mut() {
            Some(store) => store.current_at(),
            None => (Registry::default(), 0),
        }
    }

    /// Change the registry and save it.
    pub fn update_registry<T>(
        &self,
        change: impl FnOnce(&mut Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        self.update_registry_at(|registry, _| change(registry))
    }

    /// [`Accounts::update_registry_at`] for async code: waiting for another
    /// process's change sleeps the task, never the thread.
    pub async fn update_registry_at_async<T>(
        &self,
        change: impl FnOnce(&mut Registry, u64) -> Result<T, String>,
    ) -> Result<T, String> {
        let Some(file) = self.paths.as_ref().map(ClaudePaths::registry_file) else {
            return Err("This computer's user has no home folder for Claude Code accounts.".into());
        };
        let lock = registry::FileLock::take_async(&Store::lock_path(&file)).await?;
        match self.store().as_mut() {
            Some(store) => store.update_locked(&lock, change),
            None => Err("This computer's user has no home folder for Claude Code accounts.".into()),
        }
    }

    /// Change the registry and save it; `change` gets the generation of
    /// the registry it changes, to refuse when it is not the one a decision
    /// was made on. The lock against other processes is taken before the
    /// store's mutex, so waiting for it never holds up a reader here.
    pub fn update_registry_at<T>(
        &self,
        change: impl FnOnce(&mut Registry, u64) -> Result<T, String>,
    ) -> Result<T, String> {
        let Some(file) = self.paths.as_ref().map(ClaudePaths::registry_file) else {
            return Err("This computer's user has no home folder for Claude Code accounts.".into());
        };
        let lock = registry::FileLock::take(&Store::lock_path(&file))?;
        match self.store().as_mut() {
            Some(store) => store.update_locked(&lock, change),
            None => Err("This computer's user has no home folder for Claude Code accounts.".into()),
        }
    }

    /// The `CLAUDE_CONFIG_DIR` account `id` runs with (`None` for default).
    pub fn env_dir(&self, id: &str) -> Option<String> {
        if id == DEFAULT {
            return None;
        }
        self.paths
            .as_ref()
            .map(|paths| paths.account_dir(id).to_string_lossy().into_owned())
    }

    /// The account whose folder is `dir`: `default` for `~/.claude`, a
    /// registered id, else `None` (D5).
    pub fn account_of_dir(&self, registry: &Registry, dir: &Path) -> Option<String> {
        let paths = self.paths.as_ref()?;
        if paths.is_default_dir(dir) {
            return Some(DEFAULT.to_string());
        }
        registry
            .accounts
            .iter()
            .filter(|entry| entry.id != DEFAULT)
            .find(|entry| paths::same_dir(dir, &paths.account_dir(&entry.id)))
            .map(|entry| entry.id.clone())
    }

    /// A look found a pane with a pending move, or the worker started:
    /// the next pending round looks at every pane once.
    pub fn note_pending(&self) {
        self.pending.store(true, Ordering::Relaxed);
    }

    /// Whether every pane should be looked at for pending moves, clearing
    /// the flag.
    pub fn take_pending(&self) -> bool {
        self.pending.swap(false, Ordering::Relaxed)
    }

    /// `pane`'s move waits for its turn; `claude_pid` runs there.
    pub fn note_waiting(&self, pane: &str, claude_pid: Option<u32>) {
        self.waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(pane.to_string(), claude_pid);
    }

    /// The panes whose move waits, with their Claude Code process.
    pub fn waiting(&self) -> Vec<(String, Option<u32>)> {
        let mut out: Vec<_> = self
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(pane, pid)| (pane.clone(), *pid))
            .collect();
        out.sort();
        out
    }

    pub fn forget_waiting(&self, pane: &str) {
        self.waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(pane);
    }

    /// The last move of `pane` failed with `error`.
    pub fn note_move_error(&self, pane: &str, error: &str) {
        self.move_errors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(pane.to_string(), error.to_string());
    }

    /// `pane`'s account changed again: its last failure is old news.
    pub fn clear_move_error(&self, pane: &str) {
        self.move_errors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(pane);
    }

    /// Why the last move of each pane failed; panes not in `alive` are
    /// dropped.
    pub fn move_errors(&self, alive: &std::collections::HashSet<&str>) -> HashMap<String, String> {
        let mut errors = self.move_errors.lock().unwrap_or_else(|p| p.into_inner());
        errors.retain(|pane, _| alive.contains(pane.as_str()));
        errors.clone()
    }

    /// The app looked at usage just now (D12).
    pub fn mark_watched(&self) {
        self.watched_until_ms
            .fetch_max(now_ms() + WATCH_MS, Ordering::Relaxed);
    }

    pub fn watched(&self, now_ms: u64) -> bool {
        self.watched_until_ms.load(Ordering::Relaxed) > now_ms
    }

    /// Whether the worker may read usage while nobody watches (O1):
    /// `accounts.json` exists and another account than the global one is
    /// signed in, so a switch could happen. With one account, never.
    pub fn may_read_unwatched(&self) -> bool {
        if !self.managed() {
            return false;
        }
        let (registry, views) = self.views();
        views
            .iter()
            .any(|view| view.id != registry.global && view.signed_in)
    }

    fn read_cache_file(&self) -> HashMap<String, Reading> {
        let Some(file) = self.paths.as_ref().and_then(|p| p.cache_file.clone()) else {
            return HashMap::new();
        };
        std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<CacheFile>(&text).ok())
            .map(|cache| cache.claude)
            .unwrap_or_default()
    }

    /// The readings, from the cache file the first time. Blocking.
    fn ensure_readings(&self) {
        if self.lock().readings_loaded {
            return;
        }
        let read = self.read_cache_file();
        let mut state = self.lock();
        if !state.readings_loaded {
            state.readings = read;
            state.readings_loaded = true;
        }
    }

    fn save_readings(&self, readings: &HashMap<String, Reading>) {
        let Some(file) = self.paths.as_ref().and_then(|p| p.cache_file.clone()) else {
            return;
        };
        let cache = CacheFile {
            claude: readings.clone(),
        };
        if let Err(error) = registry::write_atomic(&file, &cache) {
            tracing::debug!(%error, "usage cache not saved");
        }
    }

    /// Read account `id`'s sign-in again on the next load, whatever its
    /// files' stamps and the Keychain's age say: Claude Code just started
    /// on it (and may renew its token), or someone signs it in. What was
    /// read stands until then: an account a pane just moved to still counts
    /// as signed in for the next pane of the same switch.
    pub fn reread_credentials(&self, id: &str) {
        if let Some(cached) = self.lock().credentials.get_mut(id) {
            cached.reread_since = Some(now_ms());
        }
    }

    /// Forget all about account `id`, which was removed.
    fn forget(&self, id: &str) {
        self.ensure_readings();
        let readings = {
            let mut state = self.lock();
            state.credentials.remove(id);
            state.profiles.remove(id);
            state.token_digests.remove(id);
            state.readings.remove(id);
            state.readings.clone()
        };
        self.save_readings(&readings);
    }

    fn claude_on_path() -> bool {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| {
                let candidate = dir.join("claude");
                std::fs::metadata(&candidate)
                    .map(|meta| {
                        use std::os::unix::fs::PermissionsExt;
                        meta.is_file() && meta.permissions().mode() & 0o111 != 0
                    })
                    .unwrap_or(false)
            })
        })
    }

    /// Read what the snapshot shows (M2): the sign-ins whose files changed
    /// (from a Keychain, at most every five minutes), the `.claude.json`
    /// files that changed, Codex's records every 30 seconds and whether
    /// Claude Code is here. Blocking, slow at times (`security` waits for a
    /// locked Keychain; Codex's records can be large), so the state lock is
    /// only taken to see what is due and to store what was read.
    pub fn load_blocking(&self) {
        self.load_blocking_for(None);
    }

    /// [`Accounts::load_blocking`] for the accounts in `only` alone (their
    /// sign-ins and profiles: what a move to one of them needs, nothing
    /// else, no Codex), or for everything with `None`.
    fn load_blocking_for(&self, only: Option<&[String]>) {
        let Some(paths) = self.paths.clone() else {
            self.lock().loaded_at_ms = now_ms();
            return;
        };
        self.ensure_readings();
        let registry = self.registry();
        let now = now_ms();
        let (known_credentials, known_profiles, codex_due, available_due) = {
            let state = self.lock();
            let credentials: HashMap<String, ((Stamp, Stamp), u64, bool)> = state
                .credentials
                .iter()
                .map(|(id, cached)| {
                    let reread = cached.reread_since.is_some();
                    (id.clone(), (cached.stamps, cached.read_at_ms, reread))
                })
                .collect();
            let profiles: HashMap<String, Stamp> = state
                .profiles
                .iter()
                .map(|(id, cached)| (id.clone(), cached.stamp))
                .collect();
            let codex_due = only.is_none()
                && state
                    .codex
                    .as_ref()
                    .is_none_or(|(at, _)| now.saturating_sub(*at) >= CODEX_EVERY_MS);
            let available_due = only.is_none()
                && state
                    .available
                    .is_none_or(|(at, _)| now.saturating_sub(at) >= AVAILABLE_TTL_MS);
            (credentials, profiles, codex_due, available_due)
        };
        let mut credentials = Vec::new();
        let mut profiles = Vec::new();
        let wanted = |id: &str| only.is_none_or(|ids| ids.iter().any(|wanted| wanted == id));
        for entry in registry.accounts.iter().filter(|entry| wanted(&entry.id)) {
            let dir = paths.account_dir(&entry.id);
            let json = paths.account_json(&entry.id);
            let stamps = (stamp(&dir.join(".credentials.json")), stamp(&json));
            let fresh = known_credentials
                .get(&entry.id)
                .is_some_and(|(seen, at, reread)| {
                    !reread
                        && *seen == stamps
                        && (!self.keychain || now.saturating_sub(*at) < KEYCHAIN_TTL_MS)
                });
            if !fresh {
                let value = self.secrets.read(&dir, self.env_dir(&entry.id).as_deref());
                credentials.push((entry.id.clone(), stamps, value));
            }
            let profile_stamp = stamps.1;
            if known_profiles.get(&entry.id) != Some(&profile_stamp) {
                let value = profile_stamp
                    .and_then(|_| std::fs::read_to_string(&json).ok())
                    .and_then(|text| credentials::parse_profile(&text));
                profiles.push((entry.id.clone(), profile_stamp, value));
            }
        }
        let codex = codex_due.then(|| codex::read(&paths.codex_home));
        let available = available_due.then(|| Self::claude_on_path() || paths.claude_home.is_dir());

        let mut state = self.lock();
        for (id, stamps, value) in credentials {
            let digest = value
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .and_then(|c| c.token.as_ref())
                .map(|token| -> [u8; 32] { Sha256::digest(token.expose().as_bytes()).into() });
            // A new token (or the first look since the worker started) is
            // worth asking the endpoint again.
            if state.token_digests.get(&id).copied() != digest {
                if let Some(reading) = state.readings.get_mut(&id) {
                    reading.rejected = false;
                }
            }
            match digest {
                Some(digest) => state.token_digests.insert(id.clone(), digest),
                None => state.token_digests.remove(&id),
            };
            // Asked to read it again after this load began: what was read
            // here may be older than that.
            let reread_since = state
                .credentials
                .get(&id)
                .and_then(|cached| cached.reread_since)
                .filter(|since| *since >= now);
            state.credentials.insert(
                id,
                CachedCredentials {
                    stamps,
                    read_at_ms: now,
                    value,
                    reread_since,
                },
            );
        }
        for (id, stamp, value) in profiles {
            state.profiles.insert(id, CachedProfile { stamp, value });
        }
        if let Some(codex) = codex {
            state.codex = Some((now, codex));
        }
        if let Some(available) = available {
            state.available = Some((now, available));
        }
        let ids: std::collections::HashSet<&str> =
            registry.accounts.iter().map(|e| e.id.as_str()).collect();
        state.credentials.retain(|id, _| ids.contains(id.as_str()));
        state.profiles.retain(|id, _| ids.contains(id.as_str()));
        if only.is_none() {
            state.loaded_at_ms = now_ms();
        }
    }

    /// The sign-ins of `ids` alone, read now off the async threads: what an
    /// explicit move or switch to one of them needs (D9), even with nobody
    /// watching, without reading every account (or Codex).
    pub async fn load_accounts_now(self: &Arc<Self>, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let _gate = self.load_gate.lock().await;
        let this = self.clone();
        let _ = tokio::task::spawn_blocking(move || this.load_blocking_for(Some(&ids))).await;
    }

    /// Load now, off the async threads, and wait for it.
    pub async fn load_now(self: &Arc<Self>) {
        let _gate = self.load_gate.lock().await;
        let this = self.clone();
        let _ = tokio::task::spawn_blocking(move || this.load_blocking()).await;
    }

    /// Whether the worker may read anything local about the accounts now
    /// (sign-in files, the Keychain, profiles, Codex's records): someone
    /// watches, or another signed-in account could take over (O1). With
    /// neither, and without `accounts.json`, nothing is read at all.
    pub fn may_load(&self, now_ms: u64) -> bool {
        self.watched(now_ms) || self.may_read_unwatched()
    }

    /// Load in the background when the last load is a second old (the
    /// first time, wait for it): the feed never waits for a Keychain. Only
    /// while [`Accounts::may_load`]; otherwise the caches are left as they
    /// are.
    pub async fn load_soon(self: &Arc<Self>) {
        if !self.may_load(now_ms()) {
            return;
        }
        let loaded_at = self.lock().loaded_at_ms;
        if loaded_at == 0 {
            self.load_now().await;
            return;
        }
        if now_ms().saturating_sub(loaded_at) < LOAD_EVERY_MS {
            return;
        }
        if let Ok(gate) = self.load_gate.clone().try_lock_owned() {
            let this = self.clone();
            tokio::spawn(async move {
                let _ = tokio::task::spawn_blocking(move || this.load_blocking()).await;
                drop(gate);
            });
        }
    }

    fn cached_credentials(state: &State, id: &str) -> Result<Option<Credentials>, String> {
        state
            .credentials
            .get(id)
            .map(|cached| cached.value.clone())
            .unwrap_or(Ok(None))
    }

    /// Whether account `id` is signed in, as last loaded.
    pub fn is_signed_in(&self, id: &str) -> bool {
        Self::cached_credentials(&self.lock(), id)
            .ok()
            .flatten()
            .is_some_and(|c| c.signed_in())
    }

    /// Every account with what the worker last loaded of it.
    pub fn views(&self) -> (Registry, Vec<AccountView>) {
        let now = now_ms();
        let registry = self.registry();
        let Some(paths) = self.paths.clone() else {
            return (registry, Vec::new());
        };
        let state = self.lock();
        let views = registry
            .accounts
            .iter()
            .map(|entry| {
                let credentials = Self::cached_credentials(&state, &entry.id).ok().flatten();
                let rejected = state.readings.get(&entry.id).is_some_and(|r| r.rejected);
                AccountView {
                    id: entry.id.clone(),
                    dir: paths.account_dir(&entry.id),
                    signed_in: credentials.as_ref().is_some_and(Credentials::signed_in),
                    token_expired: credentials.as_ref().is_some_and(|c| c.expired(now)) || rejected,
                }
            })
            .collect();
        (registry, views)
    }

    /// The usage part of the feed, from what was last loaded (no file is
    /// read here). `panes` counts the Claude Code panes running on each
    /// account; `following`, those that follow the global one.
    pub fn snapshot(&self, panes: &HashMap<String, u32>, following: u32) -> UsageSnapshot {
        let now = now_ms();
        let managed = self.managed();
        let registry = self.registry();
        let mut state = self.lock();
        let available = state.available.is_some_and(|(_, value)| value);
        let codex = state
            .codex
            .as_ref()
            .and_then(|(_, usage)| usage.clone())
            .map(|mut usage| {
                codex::settle(&mut usage, now);
                usage
            });
        let mut accounts = Vec::new();
        if let Some(paths) = self.paths.clone() {
            for entry in &registry.accounts {
                let credentials = Self::cached_credentials(&state, &entry.id);
                let profile = state
                    .profiles
                    .get(&entry.id)
                    .and_then(|cached| cached.value.clone());
                let reading = state.readings.entry(entry.id.clone()).or_default();
                if let Some((at, value)) = profile.as_ref().and_then(|p| p.cached_usage.clone()) {
                    reading.adopt(usage::parse_usage(&value, at));
                }
                let reading = reading.clone();
                let signed = credentials.as_ref().ok().cloned().flatten();
                let signed_in = signed.as_ref().is_some_and(Credentials::signed_in);
                let token_expired =
                    signed.as_ref().is_some_and(|c| c.expired(now)) || reading.rejected;
                // Without a good reading, a failed attempt still says why.
                let mut usage = reading.usage.clone().or_else(|| {
                    reading.error.as_ref().map(|_| crate::model::ClaudeUsage {
                        five_hour: None,
                        week: None,
                        week_opus: None,
                        week_sonnet: None,
                        week_models: Vec::new(),
                        extra: None,
                        as_of_ms: reading.attempted_at_ms,
                        error: None,
                    })
                });
                if let Some(usage) = usage.as_mut() {
                    usage::settle(usage, now);
                    usage.error = match (&credentials, signed_in, token_expired) {
                        (Err(_), _, _) => Some("unreadable".to_string()),
                        (_, false, _) => Some("not_signed_in".to_string()),
                        (_, true, true) => Some("token_expired".to_string()),
                        _ => reading.error.clone(),
                    };
                }
                let dir = paths.account_dir(&entry.id);
                accounts.push(ClaudeAccount {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    color: entry.color.clone(),
                    dir: dir.to_string_lossy().into_owned(),
                    dir_display: paths.display(&dir),
                    is_default: entry.id == DEFAULT,
                    email: profile.as_ref().and_then(|p| p.email.clone()),
                    organization: profile.as_ref().and_then(|p| p.organization.clone()),
                    plan: credentials::plan(signed.as_ref(), profile.as_ref()),
                    tier: signed
                        .as_ref()
                        .and_then(|c| c.rate_limit_tier.clone())
                        .or_else(|| profile.as_ref().and_then(|p| p.rate_limit_tier.clone())),
                    signed_in,
                    token_expired: signed_in && token_expired,
                    shared: entry.shared.clone(),
                    panes: panes.get(&entry.id).copied().unwrap_or(0),
                    usage,
                });
            }
        }
        let rules = registry.rules.clone();
        let suggestion =
            rules::suggestion(&registry, &accounts, following, now, &mut state.first_seen);
        UsageSnapshot {
            claude: ClaudeSnapshot {
                available,
                managed,
                global: registry.global.clone(),
                runs_as_root: runs_as_root(),
                accounts,
                rules,
                suggestion,
                last_switch: registry.last_switch.clone(),
            },
            codex,
            generated_at_ms: now,
        }
    }

    /// Read the usage of `ids` (every account when `None`) whose readings
    /// are due: older than five minutes, or than one with `force` (D12).
    /// Accounts without a sign-in, or whose sign-in expired, are not asked
    /// (D11). Returns whether any reading changed.
    pub async fn refresh(self: &Arc<Self>, ids: Option<&[String]>, force: bool) -> bool {
        let Some(client) = self.client.clone() else {
            return false;
        };
        let _round = self.reading.lock().await;
        // The sign-ins as they are now (a login may have just finished).
        self.load_now().await;
        let now = now_ms();
        let registry = self.registry();
        let mut asks = Vec::new();
        {
            let mut state = self.lock();
            for entry in &registry.accounts {
                if ids.is_some_and(|ids| !ids.contains(&entry.id)) {
                    continue;
                }
                let Ok(Some(credentials)) = Self::cached_credentials(&state, &entry.id) else {
                    continue;
                };
                let Some(token) = credentials.token.clone() else {
                    continue;
                };
                let reading = state.readings.entry(entry.id.clone()).or_default();
                if credentials.expired(now) || reading.rejected || !reading.due(now, force) {
                    continue;
                }
                let digest: [u8; 32] = Sha256::digest(token.expose().as_bytes()).into();
                asks.push((entry.id.clone(), entry.created_at_ms, token, digest));
            }
        }
        if asks.is_empty() {
            return false;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for (id, created_at_ms, token, digest) in asks {
            let client = client.clone();
            tasks.spawn(async move {
                let outcome = client.fetch(&token).await;
                (id, created_at_ms, digest, outcome)
            });
        }
        let mut results = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Ok(result) = joined {
                results.push(result);
            }
        }
        let now = now_ms();
        // An account removed (or removed and added again) while it was
        // asked keeps no reading of the old one.
        let registry = self.registry();
        let readings = {
            let mut state = self.lock();
            for (id, created_at_ms, digest, outcome) in results {
                if registry.get(&id).map(|entry| entry.created_at_ms) != Some(created_at_ms) {
                    continue;
                }
                // Signed in again while it was asked: an answer about the
                // old token (a late 401 above all) says nothing of the new
                // one, which the next round asks about.
                if state.token_digests.get(&id) != Some(&digest) {
                    continue;
                }
                if let Err(error) = &outcome {
                    tracing::info!(account = %id, error = %error.word(), "usage not read");
                }
                state
                    .readings
                    .entry(id)
                    .or_default()
                    .record(outcome.map(|value| usage::parse_usage(&value, now)), now);
            }
            state.readings.clone()
        };
        self.save_readings(&readings);
        true
    }

    /// The age of account `id`'s reading.
    pub fn reading_age_ms(&self, id: &str, now: u64) -> Option<u64> {
        self.lock().readings.get(id).and_then(|r| r.age_ms(now))
    }
}

pub fn runs_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A home folder in a temporary directory with the given overrides.
    pub struct Home {
        pub dir: PathBuf,
        pub paths: ClaudePaths,
    }

    impl Home {
        pub fn new(name: &str) -> Self {
            Self::with_usage_url(name, "http://127.0.0.1:9")
        }

        /// A home whose usage endpoint is `url` (a test's own server).
        pub fn with_usage_url(name: &str, url: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "tmux-acct-{name}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(dir.join("home/.claude")).unwrap();
            let home = dir.join("home");
            let vars: HashMap<&str, String> = HashMap::from([
                ("HOME", home.to_string_lossy().into_owned()),
                (
                    "XDG_CACHE_HOME",
                    dir.join("cache").to_string_lossy().into_owned(),
                ),
                (
                    "III_CLAUDE_ACCOUNTS_DIR",
                    dir.join("accounts").to_string_lossy().into_owned(),
                ),
                ("III_CLAUDE_USAGE_URL", url.to_string()),
            ]);
            let paths = ClaudePaths::from_vars(|key| vars.get(key).cloned(), false).unwrap();
            Self { dir, paths }
        }

        /// Accounts on this home; sign-ins and profiles are loaded once.
        pub fn accounts(&self) -> Arc<Accounts> {
            let accounts = Arc::new(Accounts::new(
                Some(self.paths.clone()),
                Box::new(credentials::FileStore),
                false,
            ));
            accounts.load_blocking();
            accounts
        }

        pub fn sign_in(&self, dir: &Path, token: &str, expires_at_ms: u64) {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join(".credentials.json"),
                serde_json::json!({"claudeAiOauth": {"accessToken": token, "expiresAt": expires_at_ms,
                    "subscriptionType": "team", "rateLimitTier": "default_claude_team"}})
                .to_string(),
            )
            .unwrap();
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::Home;
    use super::*;

    #[test]
    fn without_a_registry_there_is_the_default_account() {
        let home = Home::new("plain");
        home.sign_in(&home.paths.claude_home, "demo-work", now_ms() + 3_600_000);
        std::fs::write(
            &home.paths.default_json,
            r#"{"oauthAccount":{"emailAddress":"you@company.com","organizationName":"Company","organizationType":"claude_team"},
               "cachedUsageUtilization":{"fetchedAtMs":1000,"utilization":{"seven_day":{"utilization":91,"resets_at":"2999-01-01T00:00:00Z"}}}}"#,
        )
        .unwrap();
        let accounts = home.accounts();
        let snapshot = accounts.snapshot(&HashMap::from([("default".into(), 2)]), 2);
        assert!(accounts.can_fetch());
        assert_eq!(accounts.usage_origin(), Some("http://127.0.0.1:9"));
        let claude = snapshot.claude;
        assert!(!claude.managed);
        assert!(claude.available);
        assert_eq!(claude.global, "default");
        assert_eq!(claude.accounts.len(), 1);
        let work = &claude.accounts[0];
        assert!(work.is_default && work.signed_in && !work.token_expired);
        assert_eq!(work.dir_display, "~/.claude");
        assert_eq!(work.email.as_deref(), Some("you@company.com"));
        assert_eq!(work.plan.as_deref(), Some("team"));
        assert_eq!(work.panes, 2);
        let usage = work.usage.as_ref().unwrap();
        assert_eq!(usage.week.as_ref().unwrap().used_percent, 91.0);
        assert_eq!(usage.as_of_ms, 1000, "Claude Code's own reading");
        assert!(usage.error.is_none());
        let json = serde_json::to_string(&work).unwrap();
        assert!(!json.contains("demo-work"), "no token in the feed");
    }

    #[test]
    fn registered_folders_are_their_accounts() {
        let home = Home::new("dirs");
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
        accounts.load_blocking();
        let registry = accounts.registry();
        let personal = home.paths.account_dir("personal");
        assert_eq!(
            accounts.account_of_dir(&registry, &personal).as_deref(),
            Some("personal")
        );
        assert_eq!(
            accounts
                .account_of_dir(&registry, &home.paths.claude_home)
                .as_deref(),
            Some("default")
        );
        assert_eq!(
            accounts.account_of_dir(&registry, Path::new("/elsewhere")),
            None
        );
        assert_eq!(accounts.env_dir("default"), None);
        assert_eq!(
            accounts.env_dir("personal").as_deref(),
            Some(personal.to_string_lossy().as_ref())
        );
        let (_, views) = accounts.views();
        assert_eq!(views.len(), 2);
        assert!(!views[1].signed_in);
    }

    #[tokio::test]
    async fn expired_or_missing_sign_ins_are_never_sent() {
        let home = Home::new("expired");
        home.sign_in(&home.paths.claude_home, "demo-work", 1_000);
        let accounts = home.accounts();
        assert!(!accounts.refresh(None, true).await, "nothing was asked");
        let snapshot = accounts.snapshot(&HashMap::new(), 0);
        let work = &snapshot.claude.accounts[0];
        assert!(work.signed_in && work.token_expired);
    }

    #[test]
    fn a_new_token_lifts_a_refusal() {
        let home = Home::new("rejected");
        home.sign_in(&home.paths.claude_home, "demo-old", now_ms() + 3_600_000);
        let accounts = home.accounts();
        accounts
            .lock()
            .readings
            .entry("default".into())
            .or_default()
            .record(Err(usage::FetchError::Unauthorized), now_ms());
        assert!(
            accounts.views().1[0].token_expired,
            "refused: counts as expired"
        );
        // Same token read again (a Keychain looks the same on disk): still refused.
        accounts.reread_credentials("default");
        accounts.load_blocking();
        assert!(accounts.views().1[0].token_expired);
        // Signed in again: a new token, asked again.
        home.sign_in(&home.paths.claude_home, "demo-new", now_ms() + 3_600_000);
        accounts.reread_credentials("default");
        accounts.load_blocking();
        assert!(!accounts.views().1[0].token_expired);
    }

    #[test]
    fn nobody_watching_reads_only_when_another_account_could_take_over() {
        let home = Home::new("unwatched");
        home.sign_in(&home.paths.claude_home, "demo-work", now_ms() + 3_600_000);
        let accounts = home.accounts();
        assert!(!accounts.may_read_unwatched(), "no accounts.json");
        let id = accounts.add("Personal", "green", Some(vec![])).unwrap();
        accounts.load_blocking();
        assert!(
            !accounts.may_read_unwatched(),
            "the other one is signed out"
        );
        home.sign_in(
            &home.paths.account_dir(&id),
            "demo-personal",
            now_ms() + 3_600_000,
        );
        accounts.load_blocking();
        assert!(accounts.may_read_unwatched());
        // The global one alone signed in: nobody could take over.
        std::fs::remove_file(home.paths.claude_home.join(".credentials.json")).unwrap();
        accounts
            .update_registry(|registry| {
                registry.global = id.clone();
                Ok(())
            })
            .unwrap();
        accounts.load_blocking();
        assert!(!accounts.may_read_unwatched());
    }

    /// A sign-in store that counts how often it is asked (a Keychain).
    struct CountingStore(Arc<AtomicU64>);

    impl SecretStore for CountingStore {
        fn read(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Credentials>, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            credentials::FileStore.read(dir, env_dir)
        }
    }

    #[tokio::test]
    async fn nobody_watching_without_accounts_json_reads_no_sign_in() {
        let home = Home::new("gate");
        home.sign_in(&home.paths.claude_home, "demo-work", now_ms() + 3_600_000);
        let asked = Arc::new(AtomicU64::new(0));
        let accounts = Arc::new(Accounts::new(
            Some(home.paths.clone()),
            Box::new(CountingStore(asked.clone())),
            true,
        ));
        assert!(!accounts.may_load(now_ms()));
        accounts.load_soon().await;
        assert_eq!(
            asked.load(Ordering::SeqCst),
            0,
            "no Keychain while nobody watches"
        );
        assert!(!accounts.snapshot(&HashMap::new(), 0).claude.accounts[0].signed_in);
        accounts.mark_watched();
        accounts.load_soon().await;
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert!(accounts.snapshot(&HashMap::new(), 0).claude.accounts[0].signed_in);
    }

    #[tokio::test]
    async fn an_account_moved_to_stays_signed_in_until_read_again() {
        let home = Home::new("reread");
        home.sign_in(&home.paths.claude_home, "demo-work", now_ms() + 3_600_000);
        let asked = Arc::new(AtomicU64::new(0));
        let accounts = Arc::new(Accounts::new(
            Some(home.paths.clone()),
            Box::new(CountingStore(asked.clone())),
            true,
        ));
        accounts.load_accounts_now(vec![DEFAULT.into()]).await;
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // A pane just moved there: the next pane of the same switch still
        // finds it signed in.
        accounts.reread_credentials(DEFAULT);
        assert!(accounts.is_signed_in(DEFAULT));
        std::thread::sleep(std::time::Duration::from_millis(2));
        // Read again on the next load, though nothing changed on disk and
        // the Keychain's last read is recent.
        accounts.load_accounts_now(vec![DEFAULT.into()]).await;
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        assert!(accounts.is_signed_in(DEFAULT));
        accounts.load_accounts_now(vec![DEFAULT.into()]).await;
        assert_eq!(asked.load(Ordering::SeqCst), 2, "once");
    }

    #[tokio::test]
    async fn an_explicit_move_reads_its_target_alone() {
        let home = Home::new("target");
        home.sign_in(&home.paths.claude_home, "demo-work", now_ms() + 3_600_000);
        let asked = Arc::new(AtomicU64::new(0));
        let accounts = Arc::new(Accounts::new(
            Some(home.paths.clone()),
            Box::new(CountingStore(asked.clone())),
            true,
        ));
        let id = accounts.add("Personal", "green", Some(vec![])).unwrap();
        home.sign_in(
            &home.paths.account_dir(&id),
            "demo-personal",
            now_ms() + 3_600_000,
        );
        accounts.load_accounts_now(vec![id.clone()]).await;
        assert_eq!(
            asked.load(Ordering::SeqCst),
            1,
            "one Keychain read, the target's"
        );
        assert!(accounts.is_signed_in(&id));
        assert!(!accounts.is_signed_in(DEFAULT), "the others are not read");
        assert_eq!(accounts.lock().loaded_at_ms, 0, "no full load");
        assert!(accounts.lock().codex.is_none(), "no Codex");
    }

    #[test]
    fn the_snapshot_reads_nothing_itself() {
        let home = Home::new("cached");
        let accounts = home.accounts();
        assert!(!accounts.snapshot(&HashMap::new(), 0).claude.accounts[0].signed_in);
        home.sign_in(&home.paths.claude_home, "demo-work", now_ms() + 3_600_000);
        assert!(
            !accounts.snapshot(&HashMap::new(), 0).claude.accounts[0].signed_in,
            "not loaded yet"
        );
        accounts.load_blocking();
        assert!(accounts.snapshot(&HashMap::new(), 0).claude.accounts[0].signed_in);
    }

    #[test]
    fn a_sandbox_never_reads_this_computer_nor_asks_the_endpoint() {
        let dir =
            std::env::temp_dir().join(format!("tmux-sandbox-{}", uuid::Uuid::new_v4().simple()));
        let accounts = Accounts::sandbox(&dir);
        let paths = accounts.paths().unwrap();
        for path in [
            &paths.home,
            &paths.claude_home,
            &paths.root,
            &paths.codex_home,
        ] {
            assert!(path.starts_with(&dir), "{}", path.display());
        }
        assert!(paths.default_json.starts_with(&dir));
        assert!(!accounts.can_fetch());
        // The worker's own unit tests get a sandbox from `from_env` too.
        let from_env = Accounts::from_env();
        assert!(!from_env.can_fetch());
        assert!(from_env
            .paths()
            .unwrap()
            .home
            .starts_with(std::env::temp_dir()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_late_refusal_of_an_old_token_leaves_the_new_one_alone() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let home = Arc::new(Home::with_usage_url("late-401", &url));
        home.sign_in(&home.paths.claude_home, "demo-old", now_ms() + 3_600_000);
        let accounts = home.accounts();
        let (server_home, server_accounts) = (home.clone(), accounts.clone());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            // Signed in again while the old token was being asked about.
            server_home.sign_in(
                &server_home.paths.claude_home,
                "demo-new-token",
                now_ms() + 3_600_000,
            );
            tokio::task::spawn_blocking(move || server_accounts.load_blocking())
                .await
                .unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        accounts.refresh(None, false).await;
        server.await.unwrap();
        assert!(
            !accounts.views().1[0].token_expired,
            "the 401 was about the old token"
        );
    }

    #[tokio::test]
    async fn a_failed_reading_is_cached_without_the_token() {
        let home = Home::new("cache");
        home.sign_in(
            &home.paths.claude_home,
            "demo-secret-token",
            now_ms() + 3_600_000,
        );
        let accounts = home.accounts();
        // Port 9 on loopback: nothing listens, so it fails as `network`.
        assert!(accounts.refresh(None, false).await);
        assert!(!accounts.refresh(None, false).await, "not due again yet");
        let cache = std::fs::read_to_string(home.paths.cache_file.as_ref().unwrap()).unwrap();
        assert!(cache.contains("network"));
        assert!(!cache.contains("demo-secret-token"));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.paths.cache_file.as_ref().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
