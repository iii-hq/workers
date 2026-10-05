//! Credential references: a provider slice's `api_key: "secret://NAME"` is
//! resolved through the `secrets` worker (`secrets::resolve`) and is never
//! forwarded to a provider as a literal key.
//!
//! Resolved values live only in [`SecretCache`] — never in the configuration
//! snapshot, logs or traces — and are wiped from memory when replaced or
//! evicted (copies handed to a resolve response are ordinary strings). An
//! entry is re-read on `secrets::changed`, on a configuration change of a
//! slice that references it, after [`RESOLVED_TTL`] / [`FAILED_TTL`] on
//! demand, and when new functions register while the secrets worker was
//! unreachable — so the router keeps working when the secrets worker starts
//! after it. Every observed change of a cached outcome is reported to the
//! change listener, which nudges the providers referencing that secret.
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::future::{join_all, BoxFuture};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::time::Instant;

use crate::types::errors::is_function_not_found;

pub mod on_changed;

/// Reference scheme in a slice's `api_key`.
pub const SCHEME: &str = "secret://";
pub const RESOLVE_ID: &str = "secrets::resolve";
/// Trigger type the `secrets` worker fires on create / rotate / delete /
/// access change. Payload: `{name, ref, action, fingerprint?, updated_at}`.
pub const CHANGED_TRIGGER_TYPE: &str = "secrets::changed";
/// The worker name a secret's `consumers` must list for this router to read it.
pub const CONSUMER: &str = "llm-router";
/// A missing secrets worker answers `function_not_found` at once; this only
/// bounds a hung one, which every caller of an uncached reference waits on.
pub const RESOLVE_TIMEOUT_MS: u64 = 3_000;
/// Re-read a resolved value at least this often, bounding staleness when a
/// `secrets::changed` event is missed.
pub const RESOLVED_TTL: Duration = Duration::from_secs(300);
/// Retry a failed resolution on demand after this long.
pub const FAILED_TTL: Duration = Duration::from_secs(10);

/// `NAME` in `secret://NAME`: `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// `Some(Ok(NAME))` for a `secret://NAME` reference, `Some(Err(_))` for a
/// malformed one — still a reference, never a literal key — and `None` for
/// anything else. The scheme matches case-insensitively and surrounding
/// whitespace is ignored, so no spelling of a reference reaches a provider.
pub fn parse_ref(value: &str) -> Option<Result<&str, SecretError>> {
    let value = value.trim();
    let scheme = value.get(..SCHEME.len())?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let name = &value[SCHEME.len()..];
    Some(if valid_name(name) {
        Ok(name)
    } else {
        Err(SecretError::InvalidReference)
    })
}

/// A resolved value. `Debug` never prints it and dropping it overwrites it.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}

impl PartialEq for SecretValue {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        // Best-effort wipe of the whole allocation, without a zeroize
        // dependency (the provider crates pin this crate's dependency set in
        // their lockfiles). black_box keeps the stores from being elided.
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        let capacity = bytes.capacity();
        bytes.clear();
        bytes.resize(capacity, 0);
        std::hint::black_box(&bytes);
    }
}

/// Why a reference did not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// `SECRET_NOT_FOUND`.
    NotFound,
    /// `SECRET_FORBIDDEN`: this worker is not among the secret's consumers.
    Forbidden,
    /// A malformed reference, rejected here or as `INVALID_REFERENCE`.
    InvalidReference,
    /// The secrets worker is not running, or did not answer in time.
    Unavailable,
    /// Anything else, by error code (the message is not ours to echo).
    Failed(String),
}

impl SecretError {
    /// Classify a `secrets::resolve` failure. Codes are matched in the
    /// message too, for a handler that reported one as plain text.
    pub fn from_bus(error: &Error) -> Self {
        match error {
            Error::Remote { code, message, .. } => {
                let says =
                    |wanted: &str| code.eq_ignore_ascii_case(wanted) || message.contains(wanted);
                if says("SECRET_NOT_FOUND") {
                    Self::NotFound
                } else if says("SECRET_FORBIDDEN") {
                    Self::Forbidden
                } else if says("INVALID_REFERENCE") {
                    Self::InvalidReference
                } else if is_function_not_found(error) {
                    Self::Unavailable
                } else {
                    Self::Failed(code.clone())
                }
            }
            Error::Timeout | Error::NotConnected | Error::WebSocket(_) => Self::Unavailable,
            _ => Self::Failed("invocation_failed".into()),
        }
    }

    /// What an operator should do about it, naming the secret but never a
    /// value. A malformed reference is not echoed: it may be a pasted key.
    pub fn describe(&self, name: &str) -> String {
        match self {
            Self::NotFound => format!(
                "secret {name} not found in the secrets worker; store it there or fix the reference"
            ),
            Self::Forbidden => format!(
                "{CONSUMER} is not allowed to read secret {name}; add {CONSUMER} to the secret's consumers"
            ),
            Self::InvalidReference => format!(
                "api_key holds a malformed secret reference; the name after {SCHEME} must match \
                 ^[A-Za-z_][A-Za-z0-9_.-]{{0,127}}$"
            ),
            Self::Unavailable => {
                format!("secrets worker is not running; start it to resolve {SCHEME}{name}")
            }
            Self::Failed(code) => format!("secrets::resolve failed for secret {name} ({code})"),
        }
    }
}

/// One `secrets::resolve` round trip for a valid name.
pub type Fetch =
    Arc<dyn Fn(String) -> BoxFuture<'static, Result<SecretValue, SecretError>> + Send + Sync>;

/// Called with the names whose cached outcome changed.
pub type ChangeListener = Arc<dyn Fn(Vec<String>) + Send + Sync>;

/// [`Fetch`] over the bus. The engine stamps the caller's worker id on the
/// call, which is what the secrets worker checks against `consumers`.
pub fn bus_fetch(iii: IIIClient) -> Fetch {
    Arc::new(move |name: String| {
        let iii = iii.clone();
        Box::pin(async move {
            let response = iii
                .trigger(TriggerRequest {
                    function_id: RESOLVE_ID.into(),
                    payload: json!({ "ref": format!("{SCHEME}{name}") }),
                    action: None,
                    timeout_ms: Some(RESOLVE_TIMEOUT_MS),
                })
                .await
                .map_err(|e| SecretError::from_bus(&e))?;
            // Move the value out instead of copying it.
            match response {
                Value::Object(mut fields) => match fields.remove("value") {
                    Some(Value::String(value)) if !value.is_empty() => Ok(SecretValue::new(value)),
                    _ => Err(SecretError::Failed("malformed_response".into())),
                },
                _ => Err(SecretError::Failed("malformed_response".into())),
            }
        })
    })
}

/// How [`SecretCache::refresh`] treats an unreachable secrets worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refetch {
    /// Keep a resolved value: nothing can rotate or revoke a secret while
    /// the secrets worker is down. For routine re-reads.
    KeepLastGood,
    /// Drop it: a change was announced, so the old value is suspect.
    Invalidate,
}

struct Entry {
    outcome: Result<SecretValue, SecretError>,
    fetched_at: Instant,
}

impl Entry {
    fn fresh(&self, now: Instant) -> bool {
        let ttl = if self.outcome.is_ok() {
            RESOLVED_TTL
        } else {
            FAILED_TTL
        };
        now.duration_since(self.fetched_at) < ttl
    }
}

fn same_outcome(
    a: &Result<SecretValue, SecretError>,
    b: &Result<SecretValue, SecretError>,
) -> bool {
    match (a, b) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => a == b,
        _ => false,
    }
}

/// Name → last `secrets::resolve` outcome. Reads are sync (the resolve
/// precedence stays a pure function over a snapshot); the async methods
/// fill it. The lock is never held across an await.
pub struct SecretCache {
    fetch: Fetch,
    entries: RwLock<HashMap<String, Entry>>,
    listener: Option<ChangeListener>,
}

impl SecretCache {
    pub fn new(fetch: Fetch) -> Self {
        Self {
            fetch,
            entries: RwLock::default(),
            listener: None,
        }
    }

    /// Report names whose cached outcome changed (a first resolution is not
    /// a change: nothing could have used the secret through this cache yet).
    pub fn with_listener(mut self, listener: ChangeListener) -> Self {
        self.listener = Some(listener);
        self
    }

    /// The cached outcome for `name`, possibly stale; `None` until resolved.
    pub fn lookup(&self, name: &str) -> Option<Result<String, SecretError>> {
        let entries = self.entries.read().unwrap();
        let entry = entries.get(name)?;
        Some(match &entry.outcome {
            Ok(value) => Ok(value.expose().to_string()),
            Err(error) => Err(error.clone()),
        })
    }

    /// Resolve the names that are missing or past their TTL.
    pub async fn ensure(&self, names: impl IntoIterator<Item = String>) {
        let now = Instant::now();
        let due: BTreeSet<String> = {
            let entries = self.entries.read().unwrap();
            names
                .into_iter()
                .filter(|name| !entries.get(name).is_some_and(|e| e.fresh(now)))
                .collect()
        };
        self.fetch_and_store(due, Refetch::KeepLastGood).await;
    }

    /// Re-read the names now, cached or not.
    pub async fn refresh(&self, names: impl IntoIterator<Item = String>, mode: Refetch) {
        self.fetch_and_store(names.into_iter().collect(), mode)
            .await;
    }

    /// Re-read every name that failed because the secrets worker was
    /// unreachable.
    pub async fn retry_unavailable(&self) {
        let names: BTreeSet<String> = {
            let entries = self.entries.read().unwrap();
            entries
                .iter()
                .filter(|(_, e)| matches!(e.outcome, Err(SecretError::Unavailable)))
                .map(|(name, _)| name.clone())
                .collect()
        };
        self.fetch_and_store(names, Refetch::KeepLastGood).await;
    }

    pub fn has_unavailable(&self) -> bool {
        self.entries
            .read()
            .unwrap()
            .values()
            .any(|e| matches!(e.outcome, Err(SecretError::Unavailable)))
    }

    /// Drop (and wipe) every entry not in `keep`.
    pub fn retain(&self, keep: &BTreeSet<String>) {
        self.entries
            .write()
            .unwrap()
            .retain(|name, _| keep.contains(name));
    }

    /// Fetch concurrently, so a hung secrets worker costs one timeout rather
    /// than one per name.
    async fn fetch_and_store(&self, names: BTreeSet<String>, mode: Refetch) {
        if names.is_empty() {
            return;
        }
        let results = join_all(names.into_iter().map(|name| {
            let fetch = (self.fetch)(name.clone());
            async move { (name, fetch.await) }
        }))
        .await;
        let now = Instant::now();
        let mut changed = Vec::new();
        {
            let mut entries = self.entries.write().unwrap();
            for (name, outcome) in results {
                if let Err(error) = &outcome {
                    // Names and codes only; never a value.
                    tracing::debug!(secret = %name, ?error, "secret reference did not resolve");
                }
                if store(&mut entries, &name, outcome, mode, now) {
                    changed.push(name);
                }
            }
        }
        if let (false, Some(listener)) = (changed.is_empty(), &self.listener) {
            listener(changed);
        }
    }
}

/// Insert one outcome; true when it differs from the previous one.
fn store(
    entries: &mut HashMap<String, Entry>,
    name: &str,
    outcome: Result<SecretValue, SecretError>,
    mode: Refetch,
    now: Instant,
) -> bool {
    if let Some(previous) = entries.get_mut(name) {
        if mode == Refetch::KeepLastGood
            && previous.outcome.is_ok()
            && matches!(outcome, Err(SecretError::Unavailable))
        {
            previous.fetched_at = now;
            return false;
        }
        let changed = !same_outcome(&previous.outcome, &outcome);
        // Replacing drops (wipes) the previous value.
        *previous = Entry {
            outcome,
            fetched_at: now,
        };
        return changed;
    }
    entries.insert(
        name.to_string(),
        Entry {
            outcome,
            fetched_at: now,
        },
    );
    false
}

/// Debounced re-read of unreachable references, requested whenever the
/// engine's function registry changes: the secrets worker registering is
/// how a router that booted first learns it can now resolve.
#[derive(Clone)]
pub struct RetryHandle {
    tx: tokio::sync::mpsc::Sender<()>,
    cache: Arc<SecretCache>,
}

impl RetryHandle {
    /// Never blocks; a no-op unless some reference is waiting on the
    /// secrets worker.
    pub fn request(&self) {
        if self.cache.has_unavailable() {
            let _ = self.tx.try_send(());
        }
    }
}

const RETRY_QUIET: Duration = Duration::from_secs(1);

pub fn spawn_unavailable_retry(cache: Arc<SecretCache>) -> RetryHandle {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    let worker_cache = cache.clone();
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // Let the registration burst settle, then re-read once.
            while tokio::time::timeout(RETRY_QUIET, rx.recv())
                .await
                .is_ok_and(|received| received.is_some())
            {}
            worker_cache.retry_unavailable().await;
        }
    });
    RetryHandle { tx, cache }
}

#[cfg(test)]
pub mod testing {
    //! A scripted [`Fetch`](super::Fetch) for unit tests.
    use super::*;
    use std::sync::Mutex;

    /// Answers from a name → outcome table (`Unavailable` when absent) and
    /// counts calls per name.
    #[derive(Clone, Default)]
    pub struct FakeSecrets {
        answers: Arc<Mutex<HashMap<String, Result<String, SecretError>>>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeSecrets {
        pub fn set(&self, name: &str, outcome: Result<&str, SecretError>) {
            self.answers
                .lock()
                .unwrap()
                .insert(name.into(), outcome.map(String::from));
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        pub fn fetch(&self) -> Fetch {
            let fake = self.clone();
            Arc::new(move |name: String| {
                fake.calls.lock().unwrap().push(name.clone());
                let outcome = fake
                    .answers
                    .lock()
                    .unwrap()
                    .get(&name)
                    .cloned()
                    .unwrap_or(Err(SecretError::Unavailable));
                Box::pin(async move { outcome.map(SecretValue::new) })
            })
        }

        pub fn cache(&self) -> SecretCache {
            SecretCache::new(self.fetch())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeSecrets;
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn parses_references_and_rejects_malformed_names() {
        assert_eq!(
            parse_ref("secret://ANTHROPIC_API_KEY"),
            Some(Ok("ANTHROPIC_API_KEY"))
        );
        assert_eq!(parse_ref("  SECRET://a.b-c_1\n"), Some(Ok("a.b-c_1")));
        assert_eq!(parse_ref("sk-ant-123"), None);
        assert_eq!(parse_ref(""), None);
        assert_eq!(parse_ref("secret:/X"), None);
        for bad in [
            "secret://",
            "secret://1ABC",
            "secret://has space",
            "secret://a/b",
        ] {
            assert_eq!(
                parse_ref(bad),
                Some(Err(SecretError::InvalidReference)),
                "{bad}"
            );
        }
        assert!(valid_name(&"A".repeat(128)));
        assert!(!valid_name(&"A".repeat(129)));
    }

    #[test]
    fn classifies_bus_failures_into_actionable_reasons() {
        let remote = |code: &str, message: &str| Error::Remote {
            code: code.into(),
            message: message.into(),
            stacktrace: None,
        };
        assert_eq!(
            SecretError::from_bus(&remote("SECRET_NOT_FOUND", "no such secret")),
            SecretError::NotFound
        );
        assert_eq!(
            SecretError::from_bus(&remote("invocation_failed", "SECRET_FORBIDDEN: nope")),
            SecretError::Forbidden
        );
        assert_eq!(
            SecretError::from_bus(&remote("INVALID_REFERENCE", "")),
            SecretError::InvalidReference
        );
        assert_eq!(
            SecretError::from_bus(&remote(
                "function_not_found",
                "Function secrets::resolve not found"
            )),
            SecretError::Unavailable
        );
        assert_eq!(
            SecretError::from_bus(&Error::Timeout),
            SecretError::Unavailable
        );
        assert_eq!(
            SecretError::from_bus(&remote("boom", "detail")),
            SecretError::Failed("boom".into())
        );

        assert_eq!(
            SecretError::NotFound.describe("ANTHROPIC_API_KEY"),
            "secret ANTHROPIC_API_KEY not found in the secrets worker; store it there or fix the reference"
        );
        assert_eq!(
            SecretError::Forbidden.describe("X"),
            "llm-router is not allowed to read secret X; add llm-router to the secret's consumers"
        );
        assert!(SecretError::Unavailable
            .describe("X")
            .starts_with("secrets worker is not running"));
        assert!(!SecretError::InvalidReference
            .describe("sk-pasted-key")
            .contains("sk-pasted-key"));
    }

    #[test]
    fn debug_never_prints_a_value() {
        let value = SecretValue::new("sk-live-123".into());
        assert_eq!(format!("{value:?}"), "SecretValue([REDACTED])");
        assert_eq!(value.expose(), "sk-live-123");
    }

    #[tokio::test]
    async fn ensure_resolves_misses_once_and_serves_from_cache() {
        let fake = FakeSecrets::default();
        fake.set("A", Ok("value-a"));
        fake.set("B", Err(SecretError::Forbidden));
        let cache = fake.cache();
        assert_eq!(cache.lookup("A"), None);

        cache.ensure(["A".to_string(), "B".to_string()]).await;
        cache.ensure(["A".to_string(), "B".to_string()]).await;
        assert_eq!(cache.lookup("A"), Some(Ok("value-a".into())));
        assert_eq!(cache.lookup("B"), Some(Err(SecretError::Forbidden)));
        let mut calls = fake.calls();
        calls.sort();
        assert_eq!(calls, vec!["A", "B"], "fresh entries are not re-fetched");
    }

    #[tokio::test]
    async fn changes_reach_the_listener_and_last_good_survives_an_outage() {
        let fake = FakeSecrets::default();
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = seen.clone();
        let cache = fake
            .cache()
            .with_listener(Arc::new(move |names| sink.lock().unwrap().extend(names)));

        // first resolution is not a change
        fake.set("K", Ok("v1"));
        cache
            .refresh(["K".to_string()], Refetch::KeepLastGood)
            .await;
        assert!(seen.lock().unwrap().is_empty());

        // same value: unchanged
        cache.refresh(["K".to_string()], Refetch::Invalidate).await;
        assert!(seen.lock().unwrap().is_empty());

        // worker down: a routine re-read keeps serving the value
        fake.set("K", Err(SecretError::Unavailable));
        cache
            .refresh(["K".to_string()], Refetch::KeepLastGood)
            .await;
        assert_eq!(cache.lookup("K"), Some(Ok("v1".into())));
        assert!(seen.lock().unwrap().is_empty());

        // an announced change (event) drops it even when unreachable
        cache.refresh(["K".to_string()], Refetch::Invalidate).await;
        assert_eq!(cache.lookup("K"), Some(Err(SecretError::Unavailable)));
        assert_eq!(*seen.lock().unwrap(), vec!["K"]);
        assert!(cache.has_unavailable());

        // the worker comes back with a rotated value
        fake.set("K", Ok("v2"));
        cache.retry_unavailable().await;
        assert_eq!(cache.lookup("K"), Some(Ok("v2".into())));
        assert_eq!(*seen.lock().unwrap(), vec!["K", "K"]);
        assert!(!cache.has_unavailable());
    }

    #[tokio::test]
    async fn retain_evicts_unreferenced_entries() {
        let fake = FakeSecrets::default();
        fake.set("KEEP", Ok("a"));
        fake.set("DROP", Ok("b"));
        let cache = fake.cache();
        cache.ensure(["KEEP".to_string(), "DROP".to_string()]).await;
        cache.retain(&BTreeSet::from(["KEEP".to_string()]));
        assert!(cache.lookup("KEEP").is_some());
        assert_eq!(cache.lookup("DROP"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_are_retried_after_their_short_ttl() {
        let fake = FakeSecrets::default();
        fake.set("N", Err(SecretError::NotFound));
        let cache = fake.cache();
        cache.ensure(["N".to_string()]).await;
        cache.ensure(["N".to_string()]).await;
        assert_eq!(fake.calls().len(), 1);
        tokio::time::advance(FAILED_TTL + Duration::from_millis(1)).await;
        fake.set("N", Ok("now-there"));
        cache.ensure(["N".to_string()]).await;
        assert_eq!(cache.lookup("N"), Some(Ok("now-there".into())));
        assert_eq!(fake.calls().len(), 2);
    }
}
