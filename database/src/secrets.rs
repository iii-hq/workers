//! Credential references in connection entries: a `password`, `tls.*` or
//! `ssh.*` value of `secret://NAME` (a value in the `secrets` worker's
//! encrypted vault) or `env://NAME` (an environment variable the `secrets`
//! worker reads from the project's `.env` or its own environment) is resolved
//! through `secrets::resolve` and never used as a literal.
//!
//! Resolved values live only in [`SecretCache`] and in the connection
//! settings built from them — never in the configuration snapshot, a reply,
//! logs or traces — and are wiped from memory when replaced or evicted. The
//! cache is keyed by the canonical reference ([`SecretRef::key`]). An entry is
//! re-read on `secrets::changed`, after [`RESOLVED_TTL`] / [`FAILED_TTL`] by
//! the maintenance tick, and on demand when a configuration that references
//! it is applied. The caller id the secrets worker checks against a secret's
//! `consumers` is stamped by the engine: the secret must list [`CONSUMER`].
//!
//! Mirrors `llm-router/src/secrets` (same wire contract, same redaction
//! rules); kept local because the router's copy is not a library.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::future::{join_all, BoxFuture};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::time::Instant;

/// Reference scheme for a value in the vault.
pub const SCHEME: &str = "secret://";
/// Reference scheme for an environment variable kept by the secrets worker.
pub const ENV_SCHEME: &str = "env://";
pub const RESOLVE_ID: &str = "secrets::resolve";
/// Trigger type the `secrets` worker fires on create / rotate / delete /
/// access change. Payload: `{name, ref, action, fingerprint?, updated_at}`.
pub const CHANGED_TRIGGER_TYPE: &str = "secrets::changed";
/// The worker name a secret's `consumers` must list for this worker to read it.
pub const CONSUMER: &str = "database";
/// A missing secrets worker answers `function_not_found` at once; this only
/// bounds a hung one.
pub const RESOLVE_TIMEOUT_MS: u64 = 3_000;
/// Re-read a resolved value at least this often, bounding staleness when a
/// `secrets::changed` event is missed.
pub const RESOLVED_TTL: Duration = Duration::from_secs(300);
/// Retry a failed resolution after this long.
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

/// Where the secrets worker keeps a referenced value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Store {
    /// `secret://NAME`: encrypted in the vault.
    Vault,
    /// `env://NAME`: an environment variable.
    Env,
}

impl Store {
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Vault => SCHEME,
            Self::Env => ENV_SCHEME,
        }
    }
}

/// A well-formed reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretRef {
    pub store: Store,
    pub name: String,
}

impl SecretRef {
    /// The canonical spelling (lowercase scheme): the cache key and the `ref`
    /// sent to `secrets::resolve`.
    pub fn key(&self) -> String {
        format!("{}{}", self.store.scheme(), self.name)
    }

    /// Parse a canonical key back; `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        parse_ref(key)?.ok()
    }
}

/// `Some(Ok(_))` for a `secret://NAME` or `env://NAME` reference,
/// `Some(Err(_))` for a malformed one — still a reference, never a literal —
/// and `None` for anything else. The scheme matches case-insensitively and
/// surrounding whitespace is ignored.
pub fn parse_ref(value: &str) -> Option<Result<SecretRef, SecretError>> {
    let value = value.trim();
    let store = [Store::Vault, Store::Env].into_iter().find(|store| {
        value
            .get(..store.scheme().len())
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case(store.scheme()))
    })?;
    let name = &value[store.scheme().len()..];
    Some(if valid_name(name) {
        Ok(SecretRef {
            store,
            name: name.to_string(),
        })
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

impl Clone for SecretValue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
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
        // dependency. black_box keeps the stores from being elided.
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
                } else if code.eq_ignore_ascii_case("function_not_found")
                    || message.contains("function_not_found")
                {
                    Self::Unavailable
                } else {
                    Self::Failed(code.clone())
                }
            }
            Error::Timeout | Error::NotConnected | Error::WebSocket(_) => Self::Unavailable,
            _ => Self::Failed("invocation_failed".into()),
        }
    }

    /// What an operator should do about it, naming the reference (a
    /// canonical [`SecretRef::key`]) but never a value.
    pub fn describe(&self, reference: &str) -> String {
        let parsed = SecretRef::from_key(reference);
        let (store, name) = parsed
            .as_ref()
            .map_or((Store::Vault, ""), |r| (r.store, r.name.as_str()));
        match (self, store) {
            (Self::NotFound, Store::Vault) => format!(
                "{reference} did not resolve: secret {name} is not stored in the secrets \
                 worker; store it there or fix the reference"
            ),
            (Self::NotFound, Store::Env) => format!(
                "{reference} did not resolve: environment variable {name} is not set in the \
                 project's .env or the secrets worker's environment"
            ),
            (Self::Forbidden, Store::Vault) => format!(
                "{reference} did not resolve: {CONSUMER} is not allowed to read secret {name}; \
                 add {CONSUMER} to the secret's consumers"
            ),
            (Self::Forbidden, Store::Env) => format!(
                "{reference} did not resolve: {CONSUMER} is not allowed to read environment \
                 variable {name}; share it with {CONSUMER} in the secrets worker \
                 (secrets::access with store \"env\")"
            ),
            (Self::InvalidReference, _) => self.describe_malformed(),
            (Self::Unavailable, _) => format!(
                "{reference} did not resolve: the secrets worker is not running or did not \
                 answer; start it (the database worker depends on it for secret:// and env:// \
                 values)"
            ),
            (Self::Failed(code), _) => {
                format!("{reference} did not resolve: secrets::resolve failed ({code})")
            }
        }
    }

    /// The malformed-reference message. The value itself is not echoed: it
    /// may be a pasted credential.
    pub fn describe_malformed(&self) -> String {
        format!(
            "holds a malformed reference; the name after {SCHEME} or {ENV_SCHEME} must match \
             ^[A-Za-z_][A-Za-z0-9_.-]{{0,127}}$"
        )
    }
}

/// One `secrets::resolve` round trip for a canonical reference.
pub type Fetch =
    Arc<dyn Fn(String) -> BoxFuture<'static, Result<SecretValue, SecretError>> + Send + Sync>;

/// [`Fetch`] over the bus. The engine stamps the caller's worker id on the
/// call, which is what the secrets worker checks against `consumers`.
pub fn bus_fetch(iii: Arc<IIIClient>) -> Fetch {
    Arc::new(move |reference: String| {
        let iii = iii.clone();
        Box::pin(async move {
            let response = iii
                .trigger(TriggerRequest {
                    function_id: RESOLVE_ID.into(),
                    payload: json!({ "ref": reference }),
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

/// A [`Fetch`] for a worker with no engine connection (tests, offline
/// tools): every reference is unavailable.
pub fn unavailable_fetch() -> Fetch {
    Arc::new(|_reference: String| Box::pin(async { Err(SecretError::Unavailable) }))
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

/// Reference → last `secrets::resolve` outcome. Reads are sync; the async
/// methods fill it. The lock is never held across an await.
pub struct SecretCache {
    fetch: Fetch,
    entries: RwLock<HashMap<String, Entry>>,
}

impl Default for SecretCache {
    fn default() -> Self {
        Self::new(unavailable_fetch())
    }
}

impl SecretCache {
    pub fn new(fetch: Fetch) -> Self {
        Self {
            fetch,
            entries: RwLock::default(),
        }
    }

    /// The cached outcome for a canonical reference, possibly stale; `None`
    /// until resolved.
    pub fn lookup(&self, key: &str) -> Option<Result<SecretValue, SecretError>> {
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        let entry = entries.get(key)?;
        Some(match &entry.outcome {
            Ok(value) => Ok(value.clone()),
            Err(error) => Err(error.clone()),
        })
    }

    /// Resolve the references that are missing or past their TTL.
    pub async fn ensure(&self, keys: impl IntoIterator<Item = String>) {
        let now = Instant::now();
        let due: BTreeSet<String> = {
            let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
            keys.into_iter()
                .filter(|key| !entries.get(key).is_some_and(|e| e.fresh(now)))
                .collect()
        };
        self.fetch_and_store(due, Refetch::KeepLastGood).await;
    }

    /// Re-read the references now, cached or not. Returns the ones whose
    /// outcome changed.
    pub async fn refresh(
        &self,
        keys: impl IntoIterator<Item = String>,
        mode: Refetch,
    ) -> Vec<String> {
        self.fetch_and_store(keys.into_iter().collect(), mode).await
    }

    /// Resolve the references now, bypassing (and leaving untouched) the
    /// cache: for a probe, which must see the value as it is this moment —
    /// a password corrected a second ago, a secret deleted and re-created —
    /// not one cached for a stored connection.
    pub async fn resolve_fresh(
        &self,
        keys: impl IntoIterator<Item = String>,
    ) -> HashMap<String, Result<SecretValue, SecretError>> {
        let keys: BTreeSet<String> = keys.into_iter().collect();
        join_all(keys.into_iter().map(|key| {
            let fetch = (self.fetch)(key.clone());
            async move { (key, fetch.await) }
        }))
        .await
        .into_iter()
        .collect()
    }

    /// Drop (and wipe) one entry; the next [`Self::ensure`] re-reads it.
    pub fn forget(&self, key: &str) {
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    /// Drop (and wipe) every entry not in `keep`.
    pub fn retain(&self, keep: &BTreeSet<String>) {
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|key, _| keep.contains(key));
    }

    /// Whether some reference is past its TTL (failed ones after
    /// [`FAILED_TTL`], resolved ones after [`RESOLVED_TTL`]).
    pub fn any_stale(&self) -> bool {
        let now = Instant::now();
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|e| !e.fresh(now))
    }

    /// Fetch concurrently, so a hung secrets worker costs one timeout rather
    /// than one per reference.
    async fn fetch_and_store(&self, keys: BTreeSet<String>, mode: Refetch) -> Vec<String> {
        if keys.is_empty() {
            return Vec::new();
        }
        let results = join_all(keys.into_iter().map(|key| {
            let fetch = (self.fetch)(key.clone());
            async move { (key, fetch.await) }
        }))
        .await;
        let now = Instant::now();
        let mut changed = Vec::new();
        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        for (key, outcome) in results {
            if let Err(error) = &outcome {
                // References and codes only; never a value.
                tracing::debug!(secret = %key, ?error, "secret reference did not resolve");
            }
            if store(&mut entries, &key, outcome, mode, now) {
                changed.push(key);
            }
        }
        changed
    }
}

/// Insert one outcome; true when it differs from the previous one.
fn store(
    entries: &mut HashMap<String, Entry>,
    key: &str,
    outcome: Result<SecretValue, SecretError>,
    mode: Refetch,
    now: Instant,
) -> bool {
    if let Some(previous) = entries.get_mut(key) {
        if mode == Refetch::KeepLastGood
            && previous.outcome.is_ok()
            && matches!(outcome, Err(SecretError::Unavailable))
        {
            previous.fetched_at = now;
            return false;
        }
        let changed = match (&previous.outcome, &outcome) {
            (Ok(a), Ok(b)) => a != b,
            (Err(a), Err(b)) => a != b,
            _ => true,
        };
        // Replacing drops (wipes) the previous value.
        *previous = Entry {
            outcome,
            fetched_at: now,
        };
        return changed;
    }
    entries.insert(
        key.to_string(),
        Entry {
            outcome,
            fetched_at: now,
        },
    );
    true
}

/// The reference a `secrets::changed` event is about, canonical: from `ref`
/// (which names the store), or else from `name` as a vault secret.
pub fn event_reference(event: &Value) -> Option<String> {
    if let Some(Some(Ok(reference))) = event.get("ref").and_then(Value::as_str).map(parse_ref) {
        return Some(reference.key());
    }
    event
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| valid_name(name))
        .map(|name| {
            SecretRef {
                store: Store::Vault,
                name: name.to_string(),
            }
            .key()
        })
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory secrets worker: `set` a reference's outcome, read back
    /// which references were fetched.
    #[derive(Clone, Default)]
    pub struct FakeSecrets {
        values: Arc<Mutex<HashMap<String, Result<String, SecretError>>>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeSecrets {
        pub fn set(&self, key: &str, outcome: Result<&str, SecretError>) {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_string(), outcome.map(str::to_string));
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        pub fn fetch(&self) -> Fetch {
            let me = self.clone();
            Arc::new(move |key: String| {
                let me = me.clone();
                Box::pin(async move {
                    me.calls.lock().unwrap().push(key.clone());
                    match me.values.lock().unwrap().get(&key) {
                        Some(Ok(v)) => Ok(SecretValue::new(v.clone())),
                        Some(Err(e)) => Err(e.clone()),
                        None => Err(SecretError::NotFound),
                    }
                })
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

    #[test]
    fn references_parse_case_insensitively_and_reject_bad_names() {
        assert_eq!(
            parse_ref(" Secret://DB_PASS ").unwrap().unwrap().key(),
            "secret://DB_PASS"
        );
        assert_eq!(
            parse_ref("ENV://X.Y-z").unwrap().unwrap().key(),
            "env://X.Y-z"
        );
        assert!(matches!(
            parse_ref("secret://has space"),
            Some(Err(SecretError::InvalidReference))
        ));
        assert!(matches!(parse_ref("secret://"), Some(Err(_))));
        assert!(parse_ref("hunter2").is_none());
        assert!(parse_ref("postgres://u:p@h/db").is_none());
    }

    #[test]
    fn secret_value_debug_never_prints_the_value() {
        let v = SecretValue::new("hunter2".into());
        assert!(!format!("{v:?}").contains("hunter2"));
    }

    #[test]
    fn describe_names_the_reference_never_a_value() {
        let msg = SecretError::Forbidden.describe("secret://DB_PASS");
        assert!(msg.contains("secret://DB_PASS"));
        assert!(
            msg.contains("add database to the secret's consumers"),
            "{msg}"
        );
        let msg = SecretError::NotFound.describe("env://DB_PASS");
        assert!(msg.contains("environment variable DB_PASS"), "{msg}");
        assert!(SecretError::Unavailable
            .describe("secret://X")
            .contains("secrets worker is not running"));
    }

    #[tokio::test]
    async fn the_cache_resolves_once_and_reports_changes_on_refresh() {
        let fake = FakeSecrets::default();
        fake.set("secret://A", Ok("v1"));
        let cache = fake.cache();
        cache.ensure(["secret://A".to_string()]).await;
        cache.ensure(["secret://A".to_string()]).await;
        assert_eq!(fake.calls().len(), 1, "fresh entries are not re-fetched");
        assert_eq!(cache.lookup("secret://A").unwrap().unwrap().expose(), "v1");

        // Unchanged → nothing reported; rotated → reported.
        assert!(cache
            .refresh(["secret://A".to_string()], Refetch::Invalidate)
            .await
            .is_empty());
        fake.set("secret://A", Ok("v2"));
        assert_eq!(
            cache
                .refresh(["secret://A".to_string()], Refetch::Invalidate)
                .await,
            vec!["secret://A".to_string()]
        );
        assert_eq!(cache.lookup("secret://A").unwrap().unwrap().expose(), "v2");
    }

    #[tokio::test]
    async fn a_fresh_resolution_bypasses_the_cache_and_forget_drops_an_entry() {
        let fake = FakeSecrets::default();
        fake.set("secret://A", Ok("wrong"));
        let cache = fake.cache();
        cache.ensure(["secret://A".to_string()]).await;
        fake.set("secret://A", Ok("right"));
        // The cache still holds the old value (its TTL has not run out)…
        assert_eq!(
            cache.lookup("secret://A").unwrap().unwrap().expose(),
            "wrong"
        );
        // …a fresh resolution reads the new one and leaves the cache alone.
        let fresh = cache
            .resolve_fresh(["secret://A".to_string(), "secret://GONE".to_string()])
            .await;
        assert_eq!(fresh["secret://A"].as_ref().unwrap().expose(), "right");
        assert_eq!(
            fresh["secret://GONE"].as_ref().unwrap_err(),
            &SecretError::NotFound
        );
        assert_eq!(
            cache.lookup("secret://A").unwrap().unwrap().expose(),
            "wrong"
        );
        assert!(cache.lookup("secret://GONE").is_none());
        // Forgotten → the next ensure re-reads it.
        cache.forget("secret://A");
        assert!(cache.lookup("secret://A").is_none());
        cache.ensure(["secret://A".to_string()]).await;
        assert_eq!(
            cache.lookup("secret://A").unwrap().unwrap().expose(),
            "right"
        );
    }

    #[tokio::test]
    async fn an_unreachable_secrets_worker_keeps_the_last_good_value_unless_invalidated() {
        let fake = FakeSecrets::default();
        fake.set("secret://A", Ok("v1"));
        let cache = fake.cache();
        cache.ensure(["secret://A".to_string()]).await;
        fake.set("secret://A", Err(SecretError::Unavailable));
        cache
            .refresh(["secret://A".to_string()], Refetch::KeepLastGood)
            .await;
        assert!(cache.lookup("secret://A").unwrap().is_ok());
        cache
            .refresh(["secret://A".to_string()], Refetch::Invalidate)
            .await;
        assert_eq!(
            cache.lookup("secret://A").unwrap().unwrap_err(),
            SecretError::Unavailable
        );
    }

    #[test]
    fn a_changed_event_names_its_reference() {
        assert_eq!(
            event_reference(&json!({"name": "A", "ref": "env://A"})).as_deref(),
            Some("env://A")
        );
        assert_eq!(
            event_reference(&json!({"name": "B"})).as_deref(),
            Some("secret://B")
        );
        assert_eq!(event_reference(&json!({"name": "bad name"})), None);
    }
}
