//! Where a Claude Code token comes from: one [`AccountSource`] for every
//! call (chat-projects D30, D140).
//!
//! - A stream uses the account its turn was frozen on,
//!   `provider_options.account` (a concrete id the harness resolved).
//! - Model discovery and token counting have no turn: they use the global
//!   account, which [`AccountSource::global`] names as a concrete id, so the
//!   token cache, keyed by id, never serves the old account after a switch.
//!
//! Today the accounts belong to the tmux worker, and [`TmuxAccounts`] asks it:
//! `tmux::accounts::credential {account}` answers `{account, access_token,
//! expires_at_ms, email?, plan?}` and renews a token that expires within five
//! minutes. A later project moves the accounts into this provider behind the
//! same trait.
//!
//! **"tmux manages the accounts"** means `tmux::accounts::credential` is
//! registered: `engine::functions::list {include_internal: true}` lists it.
//! The answer is kept [`DETECT_TTL`], since the provider has no subscription
//! to function changes. A listing that fails is not absence: it is asked again,
//! then what was known stands, and with nothing known the call fails as
//! transient (absence would send calls to the vault and boot the `~/.claude`
//! import). That function also serves the global account (the
//! `~/.claude` sign-in) whether or not tmux has an accounts registry, and tmux
//! renews what it serves, so the registry flag (`claude.managed` in tmux's
//! usage) is not the signal: with the function there and no registry, a vault
//! copy of the global refresh token would still part ways with the one tmux
//! renews. While tmux manages the accounts the provider neither reads nor
//! writes the `auth-credentials` vault. Without tmux it keeps today's vault /
//! `~/.claude` path ([`Credential::Legacy`]) for calls on the global account;
//! a turn pinned to an account never runs on another account's token.
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use iii_sdk::engine::EngineFunctions;
use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use llm_router::types::errors::is_function_not_found;
use llm_router::types::events::ErrorKind;
use serde_json::{Value, json};

use crate::PROVIDER_ID;
use crate::router_client::list_contains_function;

/// The tmux worker's token for one account (internal, denied to agents).
pub const CREDENTIAL_FN: &str = "tmux::accounts::credential";
/// The tmux worker's accounts; its `global` field names the global account.
pub const ACCOUNTS_LIST_FN: &str = "tmux::accounts::list";
/// Not an account id: the account that is global now.
pub const GLOBAL: &str = "global";
/// How long a function-list answer, and the global account's id, are kept.
pub const DETECT_TTL: Duration = Duration::from_secs(30);
/// A cached token is used until this long before it expires.
pub const EXPIRY_MARGIN_MS: i64 = 60_000;
/// tmux answers `busy` after holding a lock 10 s, and a renewal is a network
/// round trip on top.
const CREDENTIAL_TIMEOUT_MS: u64 = 30_000;
const LIST_TIMEOUT_MS: u64 = 15_000;
/// The engine's own function list answers at once when it answers at all.
const DETECT_TIMEOUT_MS: u64 = 5_000;
/// Tries of a function list before telling it could not be read.
const DETECT_ATTEMPTS: u32 = 3;
/// The pause before the second try; the third waits twice as long.
const DETECT_RETRY_DELAY: Duration = Duration::from_millis(250);
/// The error codes `tmux::accounts::credential` answers with.
const TMUX_CODES: [&str; 4] = ["not_signed_in", "token_expired", "busy", "unknown_account"];

/// One account's token. Its `Debug` leaves the token out.
#[derive(Clone, PartialEq, Eq)]
pub struct AccountToken {
    /// The concrete account id it belongs to.
    pub account: String,
    pub access_token: String,
    /// Milliseconds since the epoch; unknown means it is never cached.
    pub expires_at_ms: Option<i64>,
}

impl AccountToken {
    /// The credential shape `config::build_config` reads (the vault's).
    pub fn credential(&self) -> Value {
        let mut credential = json!({
            "type": "oauth",
            "provider": PROVIDER_ID,
            "access_token": self.access_token,
        });
        if let Some(expires_at_ms) = self.expires_at_ms {
            credential["expires_at"] = json!(expires_at_ms / 1000);
        }
        credential
    }

    /// Good for another call: more than [`EXPIRY_MARGIN_MS`] left.
    fn fresh_at(&self, now_ms: i64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires| now_ms < expires - EXPIRY_MARGIN_MS)
    }
}

/// Stands in for a token in `Debug` output.
const REDACTED: &str = "<redacted>";

impl std::fmt::Debug for AccountToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountToken")
            .field("account", &self.account)
            .field("access_token", &REDACTED)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Why an account has no token to give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
    NotSignedIn(String),
    /// The token expired and tmux could not renew it.
    TokenExpired(String),
    /// tmux held the account's lock too long (a renewal under way).
    Busy(String),
    UnknownAccount(String),
    /// tmux could not be asked, or answered something unreadable.
    Unavailable(String),
}

impl AccountError {
    fn from_code(code: &str, account: &str) -> Option<Self> {
        let account = account.to_string();
        Some(match code {
            "not_signed_in" => Self::NotSignedIn(account),
            "token_expired" => Self::TokenExpired(account),
            "busy" => Self::Busy(account),
            "unknown_account" => Self::UnknownAccount(account),
            _ => return None,
        })
    }

    /// The kind its error frame carries: a sign-in to redo, a moment to wait
    /// (the router retries `transient`), or a turn that cannot run.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::NotSignedIn(_) | Self::TokenExpired(_) => ErrorKind::AuthExpired,
            Self::Busy(_) | Self::Unavailable(_) => ErrorKind::Transient,
            Self::UnknownAccount(_) => ErrorKind::Permanent,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotSignedIn(id) if id == GLOBAL => {
                "No Claude Code account is signed in as the global one. Sign in, then try again."
                    .to_string()
            }
            Self::NotSignedIn(id) => format!(
                "The Claude Code account \"{id}\" is not signed in. Sign in to it, then try again."
            ),
            Self::TokenExpired(id) => format!(
                "The sign-in of the Claude Code account \"{id}\" has expired and could not be \
                 renewed. Sign in to it again, then try again."
            ),
            Self::Busy(id) => format!(
                "The Claude Code account \"{id}\" is busy renewing its sign-in. Try again in a moment."
            ),
            Self::UnknownAccount(id) => format!(
                "There is no Claude Code account \"{id}\". Choose another account, then try again."
            ),
            Self::Unavailable(why) => {
                format!("The Claude Code accounts could not be read ({why}). Try again.")
            }
        }
    }
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// What a 401 that survives the second ask says, for an account's token.
pub fn rejected_message(account: &str) -> String {
    format!(
        "Claude Code rejected the sign-in of the account \"{account}\". Sign in to it again, \
         then try again."
    )
}

/// The provider's one way to Claude Code tokens (D30). Implemented by
/// [`TmuxAccounts`] today.
pub trait AccountSource: Send + Sync {
    /// Whether this source holds the accounts now. `Ok(false)` only when that
    /// is known: it sends calls on the global account down today's vault /
    /// `~/.claude` path and allows the boot import. An error when it could
    /// not be told, which is never taken for absence.
    fn holds_accounts(&self) -> BoxFuture<'_, Result<bool, AccountError>>;
    /// The token of `account`, a concrete id: the cached one while it has
    /// more than [`EXPIRY_MARGIN_MS`] left, else a newly asked one.
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, Result<AccountToken, AccountError>>;
    /// Drop `account`'s cached token (the upstream turned it down), and the
    /// global account's id with it.
    fn invalidate(&self, account: &str);
    /// The id of the account that is global now.
    fn global(&self) -> BoxFuture<'_, Result<String, AccountError>>;
}

/// The engine calls [`TmuxAccounts`] makes, so tests can answer them.
pub trait Engine: Send + Sync + 'static {
    fn invoke(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'static, Result<Value, Error>>;
    /// This worker's namespace (`default` when unset).
    fn worker_namespace(&self) -> String;
}

impl Engine for IIIClient {
    fn invoke(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> BoxFuture<'static, Result<Value, Error>> {
        let (iii, function_id) = (self.clone(), function_id.to_string());
        Box::pin(async move {
            iii.call(CallRequest {
                function_id,
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
        })
    }

    fn worker_namespace(&self) -> String {
        self.namespace().unwrap_or_else(|| "default".into())
    }
}

/// Each account's cached token, and how many times it was invalidated.
#[derive(Default)]
struct TokenCache {
    tokens: HashMap<String, AccountToken>,
    /// One more on every invalidate, so an answer asked for before one is
    /// never cached after it: it may be the token just refused.
    generations: HashMap<String, u64>,
}

impl TokenCache {
    fn generation(&self, account: &str) -> u64 {
        self.generations.get(account).copied().unwrap_or(0)
    }
}

/// The tmux worker as the [`AccountSource`] (contract A of chat-projects
/// phase 4).
pub struct TmuxAccounts<E> {
    engine: E,
    ttl: Duration,
    retry_delay: Duration,
    detected: Mutex<Option<(Instant, bool)>>,
    global: Mutex<Option<(Instant, String)>>,
    tokens: Mutex<TokenCache>,
}

impl<E: Engine> TmuxAccounts<E> {
    pub fn new(engine: E) -> Self {
        Self::with_ttl(engine, DETECT_TTL)
    }

    pub fn with_ttl(engine: E, ttl: Duration) -> Self {
        Self {
            engine,
            ttl,
            retry_delay: DETECT_RETRY_DELAY,
            detected: Mutex::new(None),
            global: Mutex::new(None),
            tokens: Mutex::new(TokenCache::default()),
        }
    }

    /// No pause between detection tries.
    #[cfg(test)]
    pub(crate) fn without_retry_delay(mut self) -> Self {
        self.retry_delay = Duration::ZERO;
        self
    }

    /// Whether tmux lists its credential function, kept [`DETECT_TTL`]. A
    /// listing that fails is tried [`DETECT_ATTEMPTS`] times; then what was
    /// known stands (a hiccup never sends a managed provider back to the
    /// vault), and with nothing known it is an error, never absence.
    async fn detect(&self) -> Result<bool, AccountError> {
        if let Some((at, present)) = *self.detected.lock().expect("detection lock")
            && at.elapsed() < self.ttl
        {
            return Ok(present);
        }
        let namespace = self.engine.worker_namespace();
        let mut failure = String::new();
        for attempt in 0..DETECT_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(self.retry_delay * attempt).await;
            }
            let listed = self
                .engine
                .invoke(
                    EngineFunctions::LIST_FUNCTIONS,
                    json!({ "prefix": "tmux::accounts::", "include_internal": true }),
                    DETECT_TIMEOUT_MS,
                )
                .await;
            let mut detected = self.detected.lock().expect("detection lock");
            match listed {
                Ok(raw) => {
                    let present = list_contains_function(&raw, CREDENTIAL_FN, &namespace);
                    *detected = Some((Instant::now(), present));
                    return Ok(present);
                }
                Err(error) => {
                    if let Some((_, present)) = *detected {
                        *detected = Some((Instant::now(), present));
                        return Ok(present);
                    }
                    failure = error.to_string();
                }
            }
        }
        Err(AccountError::Unavailable(format!(
            "{}: {failure}",
            EngineFunctions::LIST_FUNCTIONS
        )))
    }

    /// tmux went away: ask the function list again on the next call.
    fn forget_detection(&self) {
        *self.detected.lock().expect("detection lock") = None;
        self.forget_global();
    }

    /// Name the global account again on the next call.
    fn forget_global(&self) {
        *self.global.lock().expect("global lock") = None;
    }

    fn unavailable(&self, function_id: &str, error: Error) -> AccountError {
        if is_function_not_found(&error) {
            self.forget_detection();
        }
        AccountError::Unavailable(format!("{function_id}: {error}"))
    }
}

impl<E: Engine> AccountSource for TmuxAccounts<E> {
    fn holds_accounts(&self) -> BoxFuture<'_, Result<bool, AccountError>> {
        Box::pin(self.detect())
    }

    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, Result<AccountToken, AccountError>> {
        Box::pin(async move {
            let generation = {
                let cache = self.tokens.lock().expect("token lock");
                if let Some(cached) = cache.tokens.get(account)
                    && cached.fresh_at(crate::now_ms())
                {
                    return Ok(cached.clone());
                }
                cache.generation(account)
            };
            let raw = self
                .engine
                .invoke(
                    CREDENTIAL_FN,
                    json!({ "account": account }),
                    CREDENTIAL_TIMEOUT_MS,
                )
                .await
                .map_err(|error| match tmux_error_code(&error) {
                    Some(code) => {
                        let error = AccountError::from_code(code, account)
                            .expect("tmux_error_code returns known codes");
                        if matches!(error, AccountError::UnknownAccount(_)) {
                            // The global account's id may be the one gone.
                            self.forget_global();
                        }
                        error
                    }
                    None => self.unavailable(CREDENTIAL_FN, error),
                })?;
            let token = parse_credential(&raw, account)?;
            let now = crate::now_ms();
            if token.expires_at_ms.is_some_and(|expires| expires <= now) {
                return Err(AccountError::TokenExpired(account.to_string()));
            }
            let mut cache = self.tokens.lock().expect("token lock");
            // Invalidated while tmux answered: used this once, never cached.
            if cache.generation(account) == generation {
                if token.fresh_at(now) {
                    cache.tokens.insert(account.to_string(), token.clone());
                } else {
                    cache.tokens.remove(account);
                }
            }
            Ok(token)
        })
    }

    fn invalidate(&self, account: &str) {
        {
            let mut cache = self.tokens.lock().expect("token lock");
            cache.tokens.remove(account);
            *cache.generations.entry(account.to_string()).or_default() += 1;
        }
        // A refused token may mean the global account changed hands.
        self.forget_global();
    }

    fn global(&self) -> BoxFuture<'_, Result<String, AccountError>> {
        Box::pin(async move {
            if let Some((at, id)) = self.global.lock().expect("global lock").as_ref()
                && at.elapsed() < self.ttl
            {
                return Ok(id.clone());
            }
            let raw = self
                .engine
                .invoke(ACCOUNTS_LIST_FN, json!({}), LIST_TIMEOUT_MS)
                .await
                .map_err(|error| self.unavailable(ACCOUNTS_LIST_FN, error))?;
            let id = global_id(&raw)?;
            *self.global.lock().expect("global lock") = Some((Instant::now(), id.clone()));
            Ok(id)
        })
    }
}

/// `tmux::accounts::list`'s `global`: the global account's id. `null` or
/// empty means no account is signed in as the global one.
fn global_id(raw: &Value) -> Result<String, AccountError> {
    match raw.get("global") {
        Some(Value::Null) => Err(AccountError::NotSignedIn(GLOBAL.into())),
        Some(Value::String(id)) if id.trim().is_empty() => {
            Err(AccountError::NotSignedIn(GLOBAL.into()))
        }
        Some(Value::String(id)) if id.trim() != GLOBAL => Ok(id.trim().to_string()),
        _ => Err(AccountError::Unavailable(format!(
            "{ACCOUNTS_LIST_FN} named no global account"
        ))),
    }
}

/// `{account, access_token, expires_at_ms, email?, plan?}` → the token.
/// When tmux names the account it answered for, it must be the one asked
/// for: another account's token never stands in for it.
fn parse_credential(raw: &Value, account: &str) -> Result<AccountToken, AccountError> {
    match raw.get("account") {
        None | Some(Value::Null) => {}
        Some(answered) if answered.as_str().map(str::trim) == Some(account) => {}
        Some(answered) => {
            return Err(AccountError::Unavailable(format!(
                "{CREDENTIAL_FN} answered for the account {answered}, not \"{account}\""
            )));
        }
    }
    let access_token = raw
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            AccountError::Unavailable(format!("{CREDENTIAL_FN} answered no access_token"))
        })?;
    Ok(AccountToken {
        account: account.to_string(),
        access_token: access_token.to_string(),
        expires_at_ms: raw.get("expires_at_ms").and_then(Value::as_i64),
    })
}

/// The code of a `tmux::accounts::*` failure. tmux answers one JSON object
/// `{code, message}` as its handler error, which reaches a caller as
/// `invocation_failed` with that object in the message
/// (`handler error: {"code":"busy",…}`); a structured remote code (bare or
/// prefixed, `tmux/busy`) is read as well.
fn tmux_error_code(error: &Error) -> Option<&'static str> {
    let (code, message) = match error {
        Error::Remote { code, message, .. } => (code.as_str(), message.as_str()),
        Error::Handler(message) => ("", message.as_str()),
        _ => return None,
    };
    let known = |candidate: &str| TMUX_CODES.iter().copied().find(|code| *code == candidate);
    if let Some(code) = known(code.rsplit('/').next().unwrap_or(code)) {
        return Some(code);
    }
    let start = message.find('{')?;
    let object = serde_json::Deserializer::from_str(&message[start..])
        .into_iter::<Value>()
        .next()?
        .ok()?;
    known(object.get("code")?.as_str()?)
}

/// The account a stream asks for: `provider_options.account`, the concrete
/// id the harness froze on the turn. Absent (an older harness, a direct
/// `router::chat`) means the global account.
pub fn requested_account(provider_options: Option<&Value>) -> Option<&str> {
    provider_options?
        .get("account")?
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Where a call's credential came from. Its `Debug` leaves the tokens out.
#[derive(Clone, PartialEq)]
pub enum Credential {
    /// The account source holds the accounts: this account's token.
    Account(AccountToken),
    /// No account source: today's vault / `~/.claude` credential, if any.
    Legacy(Option<Value>),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Account(token) => f.debug_tuple("Account").field(token).finish(),
            Self::Legacy(credential) => f
                .debug_tuple("Legacy")
                .field(&credential.as_ref().map(|_| REDACTED))
                .finish(),
        }
    }
}

/// The credential of a call, from `source` while it holds the accounts:
/// `requested`'s token, or for no account (or `"global"`) the global
/// account's, named by its id first.
///
/// `legacy` (the vault / `~/.claude`) is the only path that may touch the
/// vault. It runs only for a call on the global account, and only when the
/// source is known not to hold the accounts. A pinned account never falls
/// back to it, since that token is another account's (and its quota): without
/// its source, and whenever detection could not tell, the call fails as
/// transient.
pub async fn credential<F, Fut>(
    source: &dyn AccountSource,
    requested: Option<&str>,
    legacy: F,
) -> Result<Credential, AccountError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<Value>>,
{
    let pinned = requested.filter(|id| *id != GLOBAL);
    match (pinned, source.holds_accounts().await?) {
        (Some(id), true) => source.token(id).await.map(Credential::Account),
        (Some(id), false) => Err(AccountError::Unavailable(format!(
            "the tmux worker that holds the account \"{id}\" is not running"
        ))),
        (None, true) => {
            let id = source.global().await?;
            source.token(&id).await.map(Credential::Account)
        }
        (None, false) => Ok(Credential::Legacy(legacy().await)),
    }
}

/// After a 401: drop the refused token and ask once more.
/// - `Ok(Some)`: a different token, the one worth another request.
/// - `Ok(None)`: the same token again, so the 401 stands. It is dropped as
///   well, so no later call is served it from the cache.
/// - `Err`: the source gave none. Its kind stands: `busy` or an unreadable
///   source is transient, not a sign-in to redo.
///
/// A caller whose second request is refused too drops that token
/// ([`AccountSource::invalidate`]).
pub async fn ask_again(
    source: &dyn AccountSource,
    refused: &AccountToken,
) -> Result<Option<AccountToken>, AccountError> {
    source.invalidate(&refused.account);
    let fresh = source.token(&refused.account).await?;
    if fresh.access_token == refused.access_token {
        source.invalidate(&refused.account);
        return Ok(None);
    }
    Ok(Some(fresh))
}

#[cfg(test)]
pub(crate) mod testing {
    //! A fake engine for the account code: answers by function id, records
    //! every call.
    use super::*;
    use std::sync::Arc;

    type Answer = Box<dyn Fn(&str, &Value) -> Result<Value, Error> + Send + Sync>;

    #[derive(Clone)]
    pub struct FakeEngine {
        pub calls: Arc<Mutex<Vec<(String, Value)>>>,
        answer: Arc<Answer>,
    }

    impl FakeEngine {
        pub fn new(
            answer: impl Fn(&str, &Value) -> Result<Value, Error> + Send + Sync + 'static,
        ) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                answer: Arc::new(Box::new(answer)),
            }
        }

        /// How many times `function_id` was called.
        pub fn count(&self, function_id: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, _)| id == function_id)
                .count()
        }

        pub fn called_ids(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    impl Engine for FakeEngine {
        fn invoke(
            &self,
            function_id: &str,
            payload: Value,
            _timeout_ms: u64,
        ) -> BoxFuture<'static, Result<Value, Error>> {
            self.calls
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload.clone()));
            let answer = (self.answer)(function_id, &payload);
            Box::pin(async move { answer })
        }

        fn worker_namespace(&self) -> String {
            "default".into()
        }
    }

    /// tmux's function list, with or without the credential function.
    pub fn function_list(with_credential: bool) -> Value {
        let mut functions = vec![json!({ "function_id": ACCOUNTS_LIST_FN })];
        if with_credential {
            functions.push(json!({ "function_id": CREDENTIAL_FN }));
        }
        json!({ "functions": functions })
    }

    /// tmux's handler error, as it reaches a caller.
    pub fn tmux_error(code: &str) -> Error {
        Error::Remote {
            code: "invocation_failed".into(),
            message: format!("handler error: {{\"code\":\"{code}\",\"message\":\"one sentence\"}}"),
            stacktrace: None,
        }
    }

    pub fn in_one_hour() -> i64 {
        crate::now_ms() + 3_600_000
    }

    /// The vault / `~/.claude` path, for a test where it must not run.
    pub async fn no_legacy() -> Option<Value> {
        panic!("the legacy credential path must not run while tmux manages the accounts")
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// tmux with two accounts, `work` global; tokens `<account>-<n>` where n
    /// counts the credential calls.
    fn tmux(expires_at_ms: impl Fn() -> i64 + Send + Sync + 'static) -> FakeEngine {
        let asked = AtomicUsize::new(0);
        FakeEngine::new(move |function_id, payload| match function_id {
            "engine::functions::list" => Ok(function_list(true)),
            ACCOUNTS_LIST_FN => Ok(json!({ "global": "work", "accounts": [] })),
            CREDENTIAL_FN => {
                let account = payload["account"].as_str().unwrap().to_string();
                let n = asked.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(json!({
                    "account": account,
                    "access_token": format!("{account}-{n}"),
                    "expires_at_ms": expires_at_ms(),
                    "email": "a@example.com",
                    "plan": "max"
                }))
            }
            other => panic!("unexpected call {other}"),
        })
    }

    #[test]
    fn the_requested_account_is_the_trimmed_provider_option() {
        let options = json!({ "account": " personal ", "other": 1 });
        assert_eq!(requested_account(Some(&options)), Some("personal"));
        assert_eq!(requested_account(Some(&json!({ "account": "" }))), None);
        assert_eq!(requested_account(Some(&json!({ "account": 3 }))), None);
        assert_eq!(requested_account(Some(&json!({}))), None);
        assert_eq!(requested_account(None), None);
    }

    #[tokio::test]
    async fn a_stream_gets_the_token_of_its_account() {
        let engine = tmux(in_one_hour);
        let source = TmuxAccounts::new(engine.clone());
        let got = credential(&source, Some("personal"), no_legacy)
            .await
            .unwrap();
        let Credential::Account(token) = got else {
            panic!("want an account token, got {got:?}")
        };
        assert_eq!(token.account, "personal");
        assert_eq!(token.access_token, "personal-1");
        let asked = engine.calls.lock().unwrap().clone();
        let (_, payload) = asked.iter().find(|(id, _)| id == CREDENTIAL_FN).unwrap();
        assert_eq!(payload, &json!({ "account": "personal" }));
        // A concrete id: the global account's id is never asked for.
        assert_eq!(engine.count(ACCOUNTS_LIST_FN), 0);
        let detection = asked
            .iter()
            .find(|(id, _)| id == "engine::functions::list")
            .unwrap();
        assert_eq!(detection.1["include_internal"], true);
    }

    #[tokio::test]
    async fn no_account_or_global_resolves_to_the_global_id_first() {
        for requested in [None, Some(GLOBAL)] {
            let engine = tmux(in_one_hour);
            let source = TmuxAccounts::new(engine.clone());
            let Credential::Account(token) =
                credential(&source, requested, no_legacy).await.unwrap()
            else {
                panic!("want an account token")
            };
            assert_eq!(token.account, "work", "{requested:?}");
            let asked = engine.calls.lock().unwrap().clone();
            assert!(
                asked
                    .iter()
                    .all(|(id, payload)| id != CREDENTIAL_FN || payload["account"] == "work"),
                "never asks tmux for \"global\": {asked:?}"
            );
        }
    }

    #[tokio::test]
    async fn without_tmux_the_global_account_takes_the_legacy_path_and_tmux_is_not_asked() {
        for requested in [None, Some(GLOBAL)] {
            let engine = FakeEngine::new(|function_id, _| match function_id {
                "engine::functions::list" => Ok(function_list(false)),
                other => panic!("unexpected call {other}"),
            });
            let source = TmuxAccounts::new(engine.clone());
            let legacy = json!({ "type": "oauth", "access_token": "vault-token" });
            let expected = legacy.clone();
            let got = credential(&source, requested, || async move { Some(legacy) })
                .await
                .unwrap();
            assert_eq!(got, Credential::Legacy(Some(expected)), "{requested:?}");
            assert_eq!(engine.called_ids(), ["engine::functions::list"]);
        }
    }

    #[tokio::test]
    async fn a_pinned_account_never_falls_back_to_the_vault() {
        // tmux confirmed absent, and a listing that never answers: the turn
        // pinned to "personal" fails as transient instead of running on the
        // ~/.claude token (another account, and its quota).
        let absent = FakeEngine::new(|function_id, _| match function_id {
            "engine::functions::list" => Ok(function_list(false)),
            other => panic!("unexpected call {other}"),
        });
        let unreadable = FakeEngine::new(|function_id, _| match function_id {
            "engine::functions::list" => Err(Error::Timeout),
            other => panic!("unexpected call {other}"),
        });
        for engine in [absent, unreadable] {
            let source = TmuxAccounts::new(engine).without_retry_delay();
            let error = credential(&source, Some("personal"), no_legacy)
                .await
                .unwrap_err();
            assert!(matches!(error, AccountError::Unavailable(_)), "{error:?}");
            assert_eq!(error.kind(), ErrorKind::Transient);
        }
    }

    #[tokio::test]
    async fn an_unreadable_listing_never_sends_the_global_account_to_the_vault() {
        let engine = FakeEngine::new(|function_id, _| match function_id {
            "engine::functions::list" => Err(Error::Timeout),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::new(engine).without_retry_delay();
        let error = credential(&source, None, no_legacy).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Transient);
    }

    #[tokio::test]
    async fn while_tmux_manages_the_accounts_the_vault_is_untouched() {
        let engine = tmux(in_one_hour);
        let source = TmuxAccounts::new(engine.clone());
        let legacy_ran = Arc::new(AtomicUsize::new(0));
        for requested in [Some("personal"), None] {
            let ran = legacy_ran.clone();
            credential(&source, requested, || async move {
                ran.fetch_add(1, Ordering::SeqCst);
                None
            })
            .await
            .unwrap();
        }
        assert_eq!(legacy_ran.load(Ordering::SeqCst), 0);
        assert!(
            engine
                .called_ids()
                .iter()
                .all(|id| !id.starts_with("auth::") && !id.starts_with("oauth::")),
            "{:?}",
            engine.called_ids()
        );
    }

    #[tokio::test]
    async fn detection_is_kept_for_the_ttl_and_asked_again_after_it() {
        let engine = tmux(in_one_hour);
        let source = TmuxAccounts::new(engine.clone());
        for _ in 0..3 {
            assert_eq!(source.holds_accounts().await, Ok(true));
        }
        assert_eq!(engine.count("engine::functions::list"), 1);

        let expiring = TmuxAccounts::with_ttl(engine.clone(), Duration::ZERO);
        assert_eq!(expiring.holds_accounts().await, Ok(true));
        assert_eq!(expiring.holds_accounts().await, Ok(true));
        assert_eq!(engine.count("engine::functions::list"), 3);
    }

    #[tokio::test]
    async fn an_unanswered_detection_keeps_what_was_known() {
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let failing = fail.clone();
        let engine = FakeEngine::new(move |function_id, _| match function_id {
            "engine::functions::list" if failing.load(Ordering::SeqCst) => Err(Error::Timeout),
            "engine::functions::list" => Ok(function_list(true)),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::with_ttl(engine.clone(), Duration::ZERO);
        assert_eq!(source.holds_accounts().await, Ok(true));
        fail.store(true, Ordering::SeqCst);
        assert_eq!(
            source.holds_accounts().await,
            Ok(true),
            "still managed through a hiccup"
        );

        // Nothing known: tried DETECT_ATTEMPTS times, then an error, which is
        // not cached as absence (the next call lists again).
        let never_known =
            TmuxAccounts::with_ttl(engine.clone(), Duration::from_secs(60)).without_retry_delay();
        let before = engine.count("engine::functions::list");
        let error = never_known.holds_accounts().await.unwrap_err();
        assert!(matches!(error, AccountError::Unavailable(_)), "{error:?}");
        assert_eq!(
            engine.count("engine::functions::list") - before,
            DETECT_ATTEMPTS as usize
        );
        fail.store(false, Ordering::SeqCst);
        assert_eq!(never_known.holds_accounts().await, Ok(true));
    }

    #[tokio::test]
    async fn a_listing_that_fails_once_is_asked_again_before_deciding() {
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let engine = FakeEngine::new(move |function_id, _| match function_id {
            "engine::functions::list" if counted.fetch_add(1, Ordering::SeqCst) == 0 => {
                Err(Error::Timeout)
            }
            "engine::functions::list" => Ok(function_list(false)),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::new(engine).without_retry_delay();
        assert_eq!(source.holds_accounts().await, Ok(false));
        assert_eq!(tries.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_token_is_cached_per_account_until_a_minute_before_it_expires() {
        let engine = tmux(in_one_hour);
        let source = TmuxAccounts::new(engine.clone());
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-1"
        );
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-1"
        );
        assert_eq!(source.token("work").await.unwrap().access_token, "work-2");
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-1"
        );
        assert_eq!(engine.count(CREDENTIAL_FN), 2);

        // Within the last minute: used once, never cached.
        let engine = tmux(|| crate::now_ms() + 30_000);
        let source = TmuxAccounts::new(engine.clone());
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-1"
        );
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-2"
        );
        assert_eq!(engine.count(CREDENTIAL_FN), 2);
    }

    #[tokio::test]
    async fn an_expired_token_from_tmux_is_never_used() {
        let engine = tmux(|| crate::now_ms() - 1);
        let source = TmuxAccounts::new(engine);
        assert_eq!(
            source.token("personal").await,
            Err(AccountError::TokenExpired("personal".into()))
        );
    }

    #[tokio::test]
    async fn a_401_drops_the_cached_token_and_asks_once_more() {
        let engine = tmux(in_one_hour);
        let source = TmuxAccounts::new(engine.clone());
        let refused = source.token("personal").await.unwrap();
        let fresh = ask_again(&source, &refused)
            .await
            .unwrap()
            .expect("a new token");
        assert_eq!(fresh.access_token, "personal-2");
        assert_eq!(engine.count(CREDENTIAL_FN), 2);
        // The new one is what the cache serves now.
        assert_eq!(source.token("personal").await.unwrap(), fresh);
        assert_eq!(engine.count(CREDENTIAL_FN), 2);
    }

    #[tokio::test]
    async fn asking_again_for_the_same_token_is_not_worth_a_request() {
        let engine = FakeEngine::new(|function_id, _| match function_id {
            CREDENTIAL_FN => Ok(json!({
                "account": "personal", "access_token": "same", "expires_at_ms": in_one_hour()
            })),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::new(engine.clone());
        let refused = source.token("personal").await.unwrap();
        assert_eq!(ask_again(&source, &refused).await, Ok(None));
        assert_eq!(engine.count(CREDENTIAL_FN), 2, "it did ask");
        // The refused token is not served from the cache afterwards.
        source.token("personal").await.unwrap();
        assert_eq!(engine.count(CREDENTIAL_FN), 3);
    }

    #[tokio::test]
    async fn asking_again_keeps_the_sources_own_error() {
        // A busy or unreadable source is a moment to wait, never a sign-in
        // to redo; a signed-out account still is one.
        for (second, want, kind) in [
            (
                tmux_error("busy"),
                AccountError::Busy("personal".into()),
                ErrorKind::Transient,
            ),
            (
                tmux_error("not_signed_in"),
                AccountError::NotSignedIn("personal".into()),
                ErrorKind::AuthExpired,
            ),
        ] {
            let asked = AtomicUsize::new(0);
            let second = Mutex::new(Some(second));
            let engine = FakeEngine::new(move |_, _| {
                if asked.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(json!({
                        "account": "personal", "access_token": "first", "expires_at_ms": in_one_hour()
                    }));
                }
                Err(second.lock().unwrap().take().expect("asked twice only"))
            });
            let source = TmuxAccounts::new(engine);
            let refused = source.token("personal").await.unwrap();
            let error = ask_again(&source, &refused).await.unwrap_err();
            assert_eq!(error, want);
            assert_eq!(error.kind(), kind);
        }
        let unreadable = FakeEngine::new(|_, _| Err(Error::Timeout));
        let source = TmuxAccounts::new(unreadable);
        let refused = AccountToken {
            account: "personal".into(),
            access_token: "first".into(),
            expires_at_ms: Some(in_one_hour()),
        };
        let error = ask_again(&source, &refused).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Transient, "{error:?}");
    }

    #[tokio::test]
    async fn an_invalidate_during_the_ask_keeps_that_answer_out_of_the_cache() {
        let slot: Arc<std::sync::OnceLock<std::sync::Weak<TmuxAccounts<FakeEngine>>>> =
            Arc::new(std::sync::OnceLock::new());
        let reach = slot.clone();
        let asked = AtomicUsize::new(0);
        let engine = FakeEngine::new(move |function_id, _| {
            assert_eq!(function_id, CREDENTIAL_FN);
            let n = asked.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                // A 401 elsewhere drops this account while tmux answers.
                let source = reach.get().and_then(std::sync::Weak::upgrade).unwrap();
                source.invalidate("personal");
            }
            Ok(json!({
                "account": "personal",
                "access_token": format!("personal-{n}"),
                "expires_at_ms": in_one_hour()
            }))
        });
        let source = Arc::new(TmuxAccounts::new(engine.clone()));
        slot.set(Arc::downgrade(&source)).ok();
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-1"
        );
        // That answer was used once, not cached: the next call asks again,
        // and that one is cached.
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-2"
        );
        assert_eq!(
            source.token("personal").await.unwrap().access_token,
            "personal-2"
        );
        assert_eq!(engine.count(CREDENTIAL_FN), 2);
    }

    #[tokio::test]
    async fn a_token_answered_for_another_account_is_never_used() {
        let engine = FakeEngine::new(|_, payload| {
            let asked = payload["account"].as_str().unwrap();
            Ok(json!({
                "account": if asked == "personal" { "work" } else { asked },
                "access_token": "a-token",
                "expires_at_ms": in_one_hour()
            }))
        });
        let source = TmuxAccounts::new(engine);
        let error = source.token("personal").await.unwrap_err();
        assert!(matches!(error, AccountError::Unavailable(_)), "{error:?}");
        assert_eq!(source.token("work").await.unwrap().account, "work");
        // An answer that names no account is taken for the one asked.
        let nameless = FakeEngine::new(|_, _| {
            Ok(json!({ "access_token": "a-token", "expires_at_ms": in_one_hour() }))
        });
        let source = TmuxAccounts::new(nameless);
        assert_eq!(source.token("personal").await.unwrap().account, "personal");
    }

    #[test]
    fn debug_output_never_shows_a_token() {
        let token = AccountToken {
            account: "personal".into(),
            access_token: "sk-ant-oat01-secret".into(),
            expires_at_ms: Some(1),
        };
        let legacy = Credential::Legacy(Some(json!({
            "type": "oauth", "access_token": "sk-ant-oat01-secret", "refresh_token": "sk-ant-ort01-secret"
        })));
        for shown in [
            format!("{token:?}"),
            format!("{:?}", Credential::Account(token.clone())),
            format!("{legacy:?}"),
        ] {
            assert!(!shown.contains("secret"), "{shown}");
            assert!(shown.contains(REDACTED), "{shown}");
        }
        assert!(format!("{token:?}").contains("personal"));
        assert_eq!(format!("{:?}", Credential::Legacy(None)), "Legacy(None)");
    }

    #[tokio::test]
    async fn tmux_error_codes_become_account_errors() {
        for (code, want, kind) in [
            (
                "not_signed_in",
                AccountError::NotSignedIn("personal".into()),
                ErrorKind::AuthExpired,
            ),
            (
                "token_expired",
                AccountError::TokenExpired("personal".into()),
                ErrorKind::AuthExpired,
            ),
            (
                "busy",
                AccountError::Busy("personal".into()),
                ErrorKind::Transient,
            ),
            (
                "unknown_account",
                AccountError::UnknownAccount("personal".into()),
                ErrorKind::Permanent,
            ),
        ] {
            let engine = FakeEngine::new(move |function_id, _| match function_id {
                CREDENTIAL_FN => Err(tmux_error(code)),
                other => panic!("unexpected call {other}"),
            });
            let got = TmuxAccounts::new(engine).token("personal").await;
            assert_eq!(got, Err(want.clone()), "{code}");
            assert_eq!(want.kind(), kind, "{code}");
            assert!(want.message().contains("\"personal\""), "{code}");
        }
    }

    #[test]
    fn tmux_error_codes_are_read_from_every_shape() {
        assert_eq!(tmux_error_code(&tmux_error("busy")), Some("busy"));
        let remote = |code: &str| Error::Remote {
            code: code.into(),
            message: "m".into(),
            stacktrace: None,
        };
        assert_eq!(
            tmux_error_code(&remote("not_signed_in")),
            Some("not_signed_in")
        );
        assert_eq!(
            tmux_error_code(&remote("tmux/unknown_account")),
            Some("unknown_account")
        );
        assert_eq!(
            tmux_error_code(&Error::Handler(
                r#"{"code":"token_expired","message":"sign in again"} (trailing)"#.into()
            )),
            Some("token_expired")
        );
        assert_eq!(tmux_error_code(&remote("invocation_failed")), None);
        assert_eq!(tmux_error_code(&tmux_error("failed")), None);
        assert_eq!(tmux_error_code(&Error::Timeout), None);
    }

    #[tokio::test]
    async fn tmux_gone_between_calls_is_unavailable_and_detected_again() {
        let gone = FakeEngine::new(|function_id, _| match function_id {
            "engine::functions::list" => Ok(function_list(true)),
            CREDENTIAL_FN => Err(Error::Remote {
                code: "function_not_found".into(),
                message: "Function tmux::accounts::credential not found".into(),
                stacktrace: None,
            }),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::new(gone.clone());
        assert_eq!(source.holds_accounts().await, Ok(true));
        let error = source.token("personal").await.unwrap_err();
        assert!(matches!(error, AccountError::Unavailable(_)), "{error:?}");
        assert_eq!(error.kind(), ErrorKind::Transient);
        assert_eq!(source.holds_accounts().await, Ok(true));
        assert_eq!(
            gone.count("engine::functions::list"),
            2,
            "the detection was dropped"
        );
    }

    #[tokio::test]
    async fn the_global_id_is_kept_for_the_ttl() {
        let engine = tmux(in_one_hour);
        let source = TmuxAccounts::new(engine.clone());
        assert_eq!(source.global().await.unwrap(), "work");
        assert_eq!(source.global().await.unwrap(), "work");
        assert_eq!(engine.count(ACCOUNTS_LIST_FN), 1);

        for (answer, unreadable) in [
            (json!({ "global": "global" }), true),
            (json!({ "accounts": [] }), true),
            (json!({ "global": 3 }), true),
            (json!({ "global": null }), false),
            (json!({ "global": " " }), false),
        ] {
            let answered = answer.clone();
            let engine = FakeEngine::new(move |_, _| Ok(answered.clone()));
            let got = TmuxAccounts::new(engine).global().await;
            if unreadable {
                assert!(matches!(got, Err(AccountError::Unavailable(_))), "{answer}");
            } else {
                // No global sign-in: a sign-in to redo, not a moment to wait.
                let error = got.unwrap_err();
                assert_eq!(error, AccountError::NotSignedIn(GLOBAL.into()), "{answer}");
                assert_eq!(error.kind(), ErrorKind::AuthExpired);
                assert!(error.message().contains("global"), "{}", error.message());
            }
        }
    }

    #[tokio::test]
    async fn the_global_id_is_named_again_after_a_401_or_an_unknown_account() {
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_gone = gone.clone();
        let engine = FakeEngine::new(move |function_id, payload| match function_id {
            ACCOUNTS_LIST_FN => Ok(json!({ "global": "work" })),
            CREDENTIAL_FN if is_gone.load(Ordering::SeqCst) => Err(tmux_error("unknown_account")),
            CREDENTIAL_FN => Ok(json!({
                "account": payload["account"], "access_token": "t", "expires_at_ms": in_one_hour()
            })),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::new(engine.clone());
        assert_eq!(source.global().await.unwrap(), "work");
        assert_eq!(source.global().await.unwrap(), "work");
        assert_eq!(engine.count(ACCOUNTS_LIST_FN), 1);

        // A 401 (the token dropped) names the global account again.
        source.invalidate("work");
        assert_eq!(source.global().await.unwrap(), "work");
        assert_eq!(engine.count(ACCOUNTS_LIST_FN), 2);

        // So does an account tmux no longer knows.
        gone.store(true, Ordering::SeqCst);
        assert_eq!(
            source.token("work").await,
            Err(AccountError::UnknownAccount("work".into()))
        );
        assert_eq!(source.global().await.unwrap(), "work");
        assert_eq!(engine.count(ACCOUNTS_LIST_FN), 3);
    }

    #[test]
    fn the_credential_has_the_vault_shape_build_config_reads() {
        let token = AccountToken {
            account: "personal".into(),
            access_token: "sk-ant-oat01-x".into(),
            expires_at_ms: Some(1_900_000_000_123),
        };
        let credential = token.credential();
        assert_eq!(
            crate::config::extract_access_token(&credential).as_deref(),
            Some("sk-ant-oat01-x")
        );
        assert_eq!(credential["type"], "oauth");
        assert_eq!(credential["expires_at"], 1_900_000_000i64);
    }
}
