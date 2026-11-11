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
pub mod panes;
pub mod paths;
pub mod registry;
pub mod time;
pub mod usage;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::model::{now_ms, ClaudeAccount, ClaudeSnapshot, CodexUsage, UsageSnapshot};
use credentials::{Credentials, FileCache, Profile, SecretStore};
use paths::ClaudePaths;
use registry::{Registry, Store};
use usage::{CacheFile, Reading, UsageClient};

/// The built-in account: `~/.claude`, `CLAUDE_CONFIG_DIR` unset.
pub const DEFAULT: &str = "default";
/// What a pane that follows the global account is marked with (D4).
pub const GLOBAL: &str = "global";

/// How long the app counts as watching after a `tmux::usage::read` or while
/// the live feed is observed (D12).
pub const WATCH_MS: u64 = 10 * 60_000;
/// How old the global account's reading may get while nobody watches and the
/// rules are on.
pub const UNWATCHED_GLOBAL_EVERY_MS: u64 = 10 * 60_000;
/// How long Codex's records are not looked at again.
const CODEX_EVERY_MS: u64 = 30_000;
/// How long a sign-in read from the Keychain is kept (macOS asks `security`).
const KEYCHAIN_TTL_MS: u64 = 5 * 60_000;
/// How long "is Claude Code here" is kept.
const AVAILABLE_TTL_MS: u64 = 60_000;

type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| Some((meta.modified().ok()?, meta.len())))
}

struct CachedCredentials {
    stamps: (Stamp, Stamp),
    read_at_ms: u64,
    value: Result<Option<Credentials>, String>,
}

#[derive(Default)]
struct State {
    store: Option<Store>,
    credentials: HashMap<String, CachedCredentials>,
    profiles: FileCache<Profile>,
    readings: HashMap<String, Reading>,
    readings_loaded: bool,
    codex: Option<(u64, Option<CodexUsage>)>,
    available: Option<(u64, bool)>,
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
    state: Mutex<State>,
    watched_until_ms: AtomicU64,
    /// One round of readings at a time.
    reading: tokio::sync::Mutex<()>,
}

impl Accounts {
    /// From this process's environment and this platform's sign-in store.
    pub fn from_env() -> Arc<Self> {
        Arc::new(Self::new(
            ClaudePaths::from_env(),
            credentials::platform_store(),
            cfg!(target_os = "macos"),
        ))
    }

    /// `keychain`: sign-ins come from a store that is slow to ask, so they
    /// are kept for a while instead of read on every change.
    pub fn new(paths: Option<ClaudePaths>, secrets: Box<dyn SecretStore>, keychain: bool) -> Self {
        let client = paths
            .as_ref()
            .map(|paths| UsageClient::new(&paths.usage_origin));
        let store = paths
            .as_ref()
            .map(|paths| Store::new(paths.registry_file()));
        Self {
            paths,
            secrets,
            keychain,
            client,
            state: Mutex::new(State {
                store,
                ..State::default()
            }),
            watched_until_ms: AtomicU64::new(0),
            reading: tokio::sync::Mutex::new(()),
        }
    }

    pub fn paths(&self) -> Option<&ClaudePaths> {
        self.paths.as_ref()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The registry (the implicit one without `accounts.json`).
    pub fn registry(&self) -> Registry {
        let mut state = self.lock();
        match state.store.as_mut() {
            Some(store) => store.current(),
            None => Registry::default(),
        }
    }

    /// `accounts.json` exists and reads.
    pub fn managed(&self) -> bool {
        let mut state = self.lock();
        state.store.as_mut().is_some_and(Store::managed)
    }

    /// Change the registry and save it.
    pub fn update_registry<T>(
        &self,
        change: impl FnOnce(&mut Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut state = self.lock();
        match state.store.as_mut() {
            Some(store) => store.update(change),
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

    /// The app looked at usage just now (D12).
    pub fn mark_watched(&self) {
        self.watched_until_ms
            .fetch_max(now_ms() + WATCH_MS, Ordering::Relaxed);
    }

    pub fn watched(&self, now_ms: u64) -> bool {
        self.watched_until_ms.load(Ordering::Relaxed) > now_ms
    }

    fn load_readings(&self, state: &mut State) {
        if state.readings_loaded {
            return;
        }
        state.readings_loaded = true;
        let Some(file) = self.paths.as_ref().and_then(|p| p.cache_file.clone()) else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Ok(cache) = serde_json::from_str::<CacheFile>(&text) {
                state.readings = cache.claude;
            }
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

    /// The sign-in of account `id`, read again when its files change (and,
    /// from a Keychain, at most every five minutes).
    fn credentials(
        &self,
        state: &mut State,
        id: &str,
        now: u64,
    ) -> Result<Option<Credentials>, String> {
        let Some(paths) = self.paths.as_ref() else {
            return Ok(None);
        };
        let dir = paths.account_dir(id);
        let stamps = (
            stamp(&dir.join(".credentials.json")),
            stamp(&paths.account_json(id)),
        );
        if let Some(cached) = state.credentials.get(id) {
            let fresh = cached.stamps == stamps
                && (!self.keychain || now.saturating_sub(cached.read_at_ms) < KEYCHAIN_TTL_MS);
            if fresh {
                return cached.value.clone();
            }
        }
        let value = self.secrets.read(&dir, self.env_dir(id).as_deref());
        // A new sign-in is a new token: the endpoint's refusal no longer holds.
        if let Some(reading) = state.readings.get_mut(id) {
            if state
                .credentials
                .get(id)
                .is_none_or(|cached| cached.stamps != stamps)
            {
                reading.rejected = false;
            }
        }
        state.credentials.insert(
            id.to_string(),
            CachedCredentials {
                stamps,
                read_at_ms: now,
                value: value.clone(),
            },
        );
        value
    }

    /// Forget account `id`'s sign-in so the next look reads it again.
    pub fn forget_credentials(&self, id: &str) {
        self.lock().credentials.remove(id);
    }

    fn profile(&self, state: &mut State, id: &str) -> Option<Profile> {
        let path = self.paths.as_ref()?.account_json(id);
        state.profiles.get(&path, credentials::parse_profile)
    }

    fn available(&self, state: &mut State, now: u64) -> bool {
        if let Some((at, value)) = state.available {
            if now.saturating_sub(at) < AVAILABLE_TTL_MS {
                return value;
            }
        }
        let on_path = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| {
                let candidate = dir.join("claude");
                std::fs::metadata(&candidate)
                    .map(|meta| {
                        use std::os::unix::fs::PermissionsExt;
                        meta.is_file() && meta.permissions().mode() & 0o111 != 0
                    })
                    .unwrap_or(false)
            })
        });
        let value = on_path
            || self
                .paths
                .as_ref()
                .is_some_and(|paths| paths.claude_home.is_dir());
        state.available = Some((now, value));
        value
    }

    fn codex(&self, state: &mut State, now: u64) -> Option<CodexUsage> {
        let paths = self.paths.as_ref()?;
        let stale = state
            .codex
            .as_ref()
            .is_none_or(|(at, _)| now.saturating_sub(*at) >= CODEX_EVERY_MS);
        if stale {
            state.codex = Some((now, codex::read(&paths.codex_home)));
        }
        let mut usage = state.codex.as_ref()?.1.clone()?;
        codex::settle(&mut usage, now);
        Some(usage)
    }

    /// Every account with what the worker knows of it.
    pub fn views(&self) -> (Registry, Vec<AccountView>) {
        let now = now_ms();
        let mut state = self.lock();
        let registry = match state.store.as_mut() {
            Some(store) => store.current(),
            None => Registry::default(),
        };
        let Some(paths) = self.paths.clone() else {
            return (registry, Vec::new());
        };
        self.load_readings(&mut state);
        let views = registry
            .accounts
            .iter()
            .map(|entry| {
                let credentials = self.credentials(&mut state, &entry.id, now).ok().flatten();
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

    /// The usage part of the feed. `panes` counts the Claude Code panes
    /// running on each account; `following`, those that follow the global one.
    pub fn snapshot(&self, panes: &HashMap<String, u32>, following: u32) -> UsageSnapshot {
        let now = now_ms();
        let mut state = self.lock();
        let managed = state.store.as_mut().is_some_and(Store::managed);
        let registry = match state.store.as_mut() {
            Some(store) => store.current(),
            None => Registry::default(),
        };
        self.load_readings(&mut state);
        let available = self.available(&mut state, now);
        let codex = self.codex(&mut state, now);
        let mut accounts = Vec::new();
        if let Some(paths) = self.paths.clone() {
            for entry in &registry.accounts {
                let credentials = self.credentials(&mut state, &entry.id, now);
                let profile = self.profile(&mut state, &entry.id);
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
        let suggestion = None;
        let _ = following;
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
    pub async fn refresh(&self, ids: Option<&[String]>, force: bool) -> bool {
        let Some(client) = self.client.clone() else {
            return false;
        };
        let _round = self.reading.lock().await;
        let now = now_ms();
        let mut asks = Vec::new();
        {
            let mut state = self.lock();
            let registry = match state.store.as_mut() {
                Some(store) => store.current(),
                None => Registry::default(),
            };
            self.load_readings(&mut state);
            for entry in &registry.accounts {
                if ids.is_some_and(|ids| !ids.contains(&entry.id)) {
                    continue;
                }
                let Ok(Some(credentials)) = self.credentials(&mut state, &entry.id, now) else {
                    continue;
                };
                let Some(token) = credentials.token.clone() else {
                    continue;
                };
                let reading = state.readings.entry(entry.id.clone()).or_default();
                if credentials.expired(now) || reading.rejected || !reading.due(now, force) {
                    continue;
                }
                asks.push((entry.id.clone(), token));
            }
        }
        if asks.is_empty() {
            return false;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for (id, token) in asks {
            let client = client.clone();
            tasks.spawn(async move {
                let outcome = client.fetch(&token).await;
                (id, outcome)
            });
        }
        let mut results = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Ok(result) = joined {
                results.push(result);
            }
        }
        let now = now_ms();
        let readings = {
            let mut state = self.lock();
            for (id, outcome) in results {
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
                ("III_CLAUDE_USAGE_URL", "http://127.0.0.1:9".to_string()),
            ]);
            let paths = ClaudePaths::from_vars(|key| vars.get(key).cloned(), false).unwrap();
            Self { dir, paths }
        }

        pub fn accounts(&self) -> Accounts {
            Accounts::new(
                Some(self.paths.clone()),
                Box::new(credentials::FileStore),
                false,
            )
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
