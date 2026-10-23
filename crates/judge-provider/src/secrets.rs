//! `api_key: "secret://NAME"` is a reference resolved through the `secrets` worker
//! (`secrets::resolve`) at call time, never sent upstream as a key. Resolved values stay
//! in memory, are re-read after a TTL or when `secrets::changed` names them, are wiped
//! when replaced, and are never logged. A reference that does not resolve is an error
//! with an actionable reason: it never falls back to the worker's environment key.
//! Any other `scheme://` value is rejected the same way, never sent as a literal key.
use iii_sdk::{
    errors::Error,
    protocol::{RegisterTriggerInput, TriggerRequest},
    IIIClient, RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const SCHEME: &str = "secret://";
const RESOLVE_ID: &str = "secrets::resolve";
const CHANGED_TRIGGER_TYPE: &str = "secrets::changed";
/// A missing secrets worker answers `function_not_found` at once; this bounds a hung one.
const RESOLVE_TIMEOUT_MS: u64 = 3_000;
/// Re-read a resolved value at least this often, bounding staleness if an event is missed.
const RESOLVED_TTL: Duration = Duration::from_secs(300);
/// Retry a failed resolution after this long.
const FAILED_TTL: Duration = Duration::from_secs(10);

/// `Some(Ok(NAME))` for `secret://NAME`, `Some(Err(_))` for a malformed reference (still
/// never a literal key), `None` for anything else. NAME matches
/// `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`; the scheme matches case-insensitively.
pub fn parse_ref(value: &str) -> Option<Result<&str, SecretError>> {
    let value = value.trim();
    if !value.get(..SCHEME.len())?.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let name = &value[SCHEME.len()..];
    let mut chars = name.chars();
    let valid = name.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    Some(if valid {
        Ok(name)
    } else {
        Err(SecretError::InvalidReference)
    })
}

/// The scheme of a `scheme://…` value, matching `^[A-Za-z][A-Za-z0-9+.-]*://`.
fn scheme(value: &str) -> Option<&str> {
    let (scheme, _) = value.trim().split_once("://")?;
    let mut chars = scheme.chars();
    (chars.next()?.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')))
    .then_some(scheme)
}

/// A resolved value: redacted in `Debug`, overwritten on drop (best effort, no extra
/// dependency).
pub struct SecretValue(String);
impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}
impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
impl Drop for SecretValue {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        let capacity = bytes.capacity();
        bytes.clear();
        bytes.resize(capacity, 0);
        std::hint::black_box(&bytes);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretError {
    /// `SECRET_NOT_FOUND`.
    NotFound,
    /// `SECRET_FORBIDDEN`: this worker is not among the secret's consumers.
    Forbidden,
    /// Malformed, rejected here or as `INVALID_REFERENCE`.
    InvalidReference,
    /// The secrets worker is not running or did not answer in time.
    Unavailable,
    /// Anything else, by code only (remote messages are not echoed).
    Failed(String),
}
impl SecretError {
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
                } else if code.eq_ignore_ascii_case("function_not_found") {
                    Self::Unavailable
                } else {
                    Self::Failed(code.clone())
                }
            }
            Error::Timeout | Error::NotConnected | Error::WebSocket(_) => Self::Unavailable,
            _ => Self::Failed("invocation_failed".into()),
        }
    }
    /// What an operator should do, naming the secret but never a value. A malformed
    /// reference is not echoed: it may be a pasted key. `consumer` is the worker name
    /// a secret's `consumers` must list for this worker to read it.
    pub fn describe(&self, name: &str, consumer: &str) -> String {
        match self {
            Self::NotFound => format!(
                "secret {name} not found in the secrets worker; store it there or fix the reference"
            ),
            Self::Forbidden => format!(
                "{consumer} is not allowed to read secret {name}; add {consumer} to the secret's consumers"
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

type FetchFuture = Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send>>;
/// One `secrets::resolve` round trip for a valid name.
pub type Fetch = Arc<dyn Fn(String) -> FetchFuture + Send + Sync>;

/// [`Fetch`] over the bus; the engine stamps the caller the secrets worker authorizes.
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
                .map_err(|error| SecretError::from_bus(&error))?;
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

struct Entry {
    outcome: Result<SecretValue, SecretError>,
    fetched_at: Instant,
}

/// Name → last resolution; the lock is never held across an await.
pub struct SecretCache {
    fetch: Fetch,
    entries: Mutex<HashMap<String, Entry>>,
    consumer: &'static str,
}
impl SecretCache {
    /// `consumer` is this worker's name, as a secret's `consumers` must list it.
    pub fn new(fetch: Fetch, consumer: &'static str) -> Self {
        Self {
            fetch,
            entries: Mutex::default(),
            consumer,
        }
    }

    /// The key a call uses: `Ok(None)` when unset (the boot key applies), the literal
    /// key, or a resolved reference. `Err` is the actionable reason a reference failed
    /// or a `scheme://` value is unsupported.
    pub async fn configured_key(&self, api_key: Option<&str>) -> Result<Option<String>, String> {
        let Some(api_key) = api_key else {
            return Ok(None);
        };
        match parse_ref(api_key) {
            None => match scheme(api_key) {
                // Only the scheme is echoed: the rest may be a pasted key.
                Some(scheme) => Err(format!(
                    "api_key uses an unsupported {scheme}:// reference; use {SCHEME}NAME"
                )),
                None => Ok(Some(api_key.to_owned())),
            },
            Some(Err(error)) => Err(error.describe("", self.consumer)),
            Some(Ok(name)) => self
                .resolve(name)
                .await
                .map(Some)
                .map_err(|error| error.describe(name, self.consumer)),
        }
    }

    async fn resolve(&self, name: &str) -> Result<String, SecretError> {
        if let Some(outcome) = self.fresh(name) {
            return outcome;
        }
        let outcome = (self.fetch)(name.to_owned()).await;
        let mut entries = self.entries.lock().unwrap();
        let now = Instant::now();
        // The secrets worker cannot rotate or revoke while unreachable: keep a resolved
        // value through an outage instead of failing every call.
        if let (Err(SecretError::Unavailable), Some(entry)) = (&outcome, entries.get_mut(name)) {
            if let Ok(value) = &entry.outcome {
                entry.fetched_at = now;
                return Ok(value.0.clone());
            }
        }
        let result = match &outcome {
            Ok(value) => Ok(value.0.clone()),
            Err(error) => Err(error.clone()),
        };
        entries.insert(
            name.to_owned(),
            Entry {
                outcome,
                fetched_at: now,
            },
        );
        result
    }

    fn fresh(&self, name: &str) -> Option<Result<String, SecretError>> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(name)?;
        let ttl = if entry.outcome.is_ok() {
            RESOLVED_TTL
        } else {
            FAILED_TTL
        };
        (entry.fetched_at.elapsed() < ttl).then(|| match &entry.outcome {
            Ok(value) => Ok(value.0.clone()),
            Err(error) => Err(error.clone()),
        })
    }

    /// Drop (and wipe) one entry so the next call re-resolves it.
    pub fn evict(&self, name: &str) {
        self.entries.lock().unwrap().remove(name);
    }
}

/// Advisory `secrets::changed` payload; it never carries a value. Other fields
/// (`action`, `fingerprint`, `updated_at`) are ignored.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct SecretChangedEvent {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SecretChangedResponse {
    pub ok: bool,
}

/// Register `fn_id` (`<worker>::on-secret-change`) to evict the named secret on
/// `secrets::changed`. The type belongs to the optional `secrets` worker: when it is
/// absent the engine parks this binding and activates it once the type registers, so
/// nothing retries here; the TTLs bound staleness.
pub fn register_secret_trigger(iii: &IIIClient, cache: Arc<SecretCache>, fn_id: &str) {
    iii.register_function(
        fn_id,
        RegisterFunction::new_async(move |event: SecretChangedEvent| {
            let cache = cache.clone();
            async move {
                let name = event.name.or_else(|| {
                    let reference = event.reference?;
                    parse_ref(&reference)?.ok().map(str::to_owned)
                });
                if let Some(name) = &name {
                    cache.evict(name);
                }
                Ok::<_, Error>(SecretChangedResponse { ok: name.is_some() })
            }
        })
        .description(
            "Internal: drop a cached secret:// API key after the secrets worker changed it.",
        )
        .metadata(json!({ "internal": true })),
    );
    if let Err(error) = iii.register_trigger(RegisterTriggerInput::new(
        CHANGED_TRIGGER_TYPE,
        fn_id,
        json!({}),
    )) {
        tracing::warn!(%error, "binding secrets::changed failed; secret:// keys refresh on their TTL only");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(
        answers: Arc<Mutex<HashMap<String, Result<String, SecretError>>>>,
        calls: Arc<Mutex<u32>>,
    ) -> Fetch {
        Arc::new(move |name: String| {
            *calls.lock().unwrap() += 1;
            let outcome = answers
                .lock()
                .unwrap()
                .get(&name)
                .cloned()
                .unwrap_or(Err(SecretError::Unavailable));
            Box::pin(async move { outcome.map(SecretValue::new) })
        })
    }

    #[test]
    fn references_parse_and_malformed_ones_are_never_literal_keys() {
        assert_eq!(
            parse_ref("secret://TYPESAFE_API_KEY"),
            Some(Ok("TYPESAFE_API_KEY"))
        );
        assert_eq!(parse_ref(" Secret://a.b-c\n"), Some(Ok("a.b-c")));
        assert_eq!(parse_ref("ts-live-key"), None);
        for bad in [
            "secret://",
            "secret://9x",
            "secret://a b",
            &format!("secret://{}", "a".repeat(129)),
        ] {
            assert_eq!(
                parse_ref(bad),
                Some(Err(SecretError::InvalidReference)),
                "{bad}"
            );
        }
        assert_eq!(
            format!("{:?}", SecretValue::new("ts-live".into())),
            "SecretValue([REDACTED])"
        );
    }

    #[test]
    fn bus_failures_map_to_actionable_reasons() {
        let remote = |code: &str| Error::Remote {
            code: code.into(),
            message: String::new(),
            stacktrace: None,
        };
        assert_eq!(
            SecretError::from_bus(&remote("SECRET_NOT_FOUND")),
            SecretError::NotFound
        );
        assert_eq!(
            SecretError::from_bus(&remote("SECRET_FORBIDDEN")),
            SecretError::Forbidden
        );
        assert_eq!(
            SecretError::from_bus(&remote("function_not_found")),
            SecretError::Unavailable
        );
        assert_eq!(
            SecretError::from_bus(&Error::Timeout),
            SecretError::Unavailable
        );
        assert_eq!(
            SecretError::Forbidden.describe("TYPESAFE_API_KEY", "judge-typesafe"),
            "judge-typesafe is not allowed to read secret TYPESAFE_API_KEY; add judge-typesafe to the secret's consumers"
        );
    }

    /// Pretend the cached entry was fetched `age` ago.
    fn age(cache: &SecretCache, name: &str, age: Duration) {
        let mut entries = cache.entries.lock().unwrap();
        let entry = entries.get_mut(name).unwrap();
        entry.fetched_at = Instant::now().checked_sub(age).unwrap();
    }

    #[tokio::test]
    async fn configured_keys_resolve_cache_evict_and_never_fall_back() {
        let answers: Arc<Mutex<HashMap<String, Result<String, SecretError>>>> = Arc::default();
        let calls: Arc<Mutex<u32>> = Arc::default();
        let cache = SecretCache::new(fake(answers.clone(), calls.clone()), "judge-typesafe");
        let set = |outcome: Result<&str, SecretError>| {
            answers
                .lock()
                .unwrap()
                .insert("TYPESAFE_API_KEY".into(), outcome.map(str::to_owned));
        };
        let reference = Some("secret://TYPESAFE_API_KEY");

        assert_eq!(cache.configured_key(None).await, Ok(None));
        assert_eq!(
            cache.configured_key(Some("ts-literal")).await,
            Ok(Some("ts-literal".into()))
        );
        assert!(cache
            .configured_key(Some("secret://a b"))
            .await
            .unwrap_err()
            .contains("malformed"));
        assert_eq!(*calls.lock().unwrap(), 0);

        // absent worker: actionable, and no fallback key is produced
        let error = cache.configured_key(reference).await.unwrap_err();
        assert!(
            error.starts_with("secrets worker is not running"),
            "{error}"
        );

        // failures retry after their short TTL; values are then cached
        set(Ok("ts-from-secrets"));
        assert!(
            cache.configured_key(reference).await.is_err(),
            "failure still fresh"
        );
        age(&cache, "TYPESAFE_API_KEY", FAILED_TTL);
        assert_eq!(
            cache.configured_key(reference).await,
            Ok(Some("ts-from-secrets".into()))
        );
        let fetched = *calls.lock().unwrap();
        assert_eq!(
            cache.configured_key(reference).await,
            Ok(Some("ts-from-secrets".into()))
        );
        assert_eq!(*calls.lock().unwrap(), fetched, "served from cache");

        // an outage keeps the last resolved value
        set(Err(SecretError::Unavailable));
        age(&cache, "TYPESAFE_API_KEY", RESOLVED_TTL);
        assert_eq!(
            cache.configured_key(reference).await,
            Ok(Some("ts-from-secrets".into()))
        );

        // secrets::changed evicts: a rotation or revocation lands on the next call
        set(Err(SecretError::Forbidden));
        cache.evict("TYPESAFE_API_KEY");
        let error = cache.configured_key(reference).await.unwrap_err();
        assert!(
            error.contains("add judge-typesafe to the secret's consumers"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn unsupported_schemes_are_errors_that_echo_only_the_scheme() {
        let calls: Arc<Mutex<u32>> = Arc::default();
        let cache = SecretCache::new(fake(Arc::default(), calls.clone()), "judge-openai");
        for (value, scheme) in [
            ("env://X", "env"),
            ("ENV://x", "ENV"),
            (" vault+kv://sk-123 ", "vault+kv"),
        ] {
            assert_eq!(
                cache.configured_key(Some(value)).await,
                Err(format!(
                    "api_key uses an unsupported {scheme}:// reference; use secret://NAME"
                )),
                "{value}"
            );
        }
        // Not a scheme: still a literal key.
        for literal in ["9x://y", "sk-live", "a:b://c"] {
            assert_eq!(
                cache.configured_key(Some(literal)).await,
                Ok(Some(literal.into()))
            );
        }
        assert_eq!(*calls.lock().unwrap(), 0);
    }
}
