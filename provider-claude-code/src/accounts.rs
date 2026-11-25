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
//! to function changes. That function also serves the global account (the
//! `~/.claude` sign-in) whether or not tmux has an accounts registry, and tmux
//! renews what it serves, so the registry flag (`claude.managed` in tmux's
//! usage) is not the signal: with the function there and no registry, a vault
//! copy of the global refresh token would still part ways with the one tmux
//! renews. While tmux manages the accounts the provider neither reads nor
//! writes the `auth-credentials` vault. Without tmux it keeps today's vault /
//! `~/.claude` path ([`Credential::Legacy`]).
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
/// The error codes `tmux::accounts::credential` answers with.
const TMUX_CODES: [&str; 4] = ["not_signed_in", "token_expired", "busy", "unknown_account"];

/// One account's token.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Whether this source holds the accounts now. False sends every call
    /// down today's vault / `~/.claude` path.
    fn available(&self) -> BoxFuture<'_, bool>;
    /// The token of `account`, a concrete id: the cached one while it has
    /// more than [`EXPIRY_MARGIN_MS`] left, else a newly asked one.
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, Result<AccountToken, AccountError>>;
    /// Drop `account`'s cached token (the upstream turned it down).
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

/// The tmux worker as the [`AccountSource`] (contract A of chat-projects
/// phase 4).
pub struct TmuxAccounts<E> {
    engine: E,
    ttl: Duration,
    detected: Mutex<Option<(Instant, bool)>>,
    global: Mutex<Option<(Instant, String)>>,
    tokens: Mutex<HashMap<String, AccountToken>>,
}

impl<E: Engine> TmuxAccounts<E> {
    pub fn new(engine: E) -> Self {
        Self::with_ttl(engine, DETECT_TTL)
    }

    pub fn with_ttl(engine: E, ttl: Duration) -> Self {
        Self {
            engine,
            ttl,
            detected: Mutex::new(None),
            global: Mutex::new(None),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    async fn detect(&self) -> bool {
        if let Some((at, present)) = *self.detected.lock().expect("detection lock")
            && at.elapsed() < self.ttl
        {
            return present;
        }
        let listed = self
            .engine
            .invoke(
                EngineFunctions::LIST_FUNCTIONS,
                json!({ "prefix": "tmux::accounts::", "include_internal": true }),
                LIST_TIMEOUT_MS,
            )
            .await;
        let namespace = self.engine.worker_namespace();
        let mut detected = self.detected.lock().expect("detection lock");
        let present = match listed {
            Ok(raw) => list_contains_function(&raw, CREDENTIAL_FN, &namespace),
            // Unanswered: keep what was known, so a hiccup never sends a
            // managed provider back to the vault; never known means absent.
            Err(_) => detected.is_some_and(|(_, present)| present),
        };
        *detected = Some((Instant::now(), present));
        present
    }

    /// tmux went away: ask the function list again on the next call.
    fn forget_detection(&self) {
        *self.detected.lock().expect("detection lock") = None;
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
    fn available(&self) -> BoxFuture<'_, bool> {
        Box::pin(self.detect())
    }

    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, Result<AccountToken, AccountError>> {
        Box::pin(async move {
            let now = crate::now_ms();
            if let Some(cached) = self.tokens.lock().expect("token lock").get(account)
                && cached.fresh_at(now)
            {
                return Ok(cached.clone());
            }
            let raw = self
                .engine
                .invoke(
                    CREDENTIAL_FN,
                    json!({ "account": account }),
                    CREDENTIAL_TIMEOUT_MS,
                )
                .await
                .map_err(|error| match tmux_error_code(&error) {
                    Some(code) => AccountError::from_code(code, account)
                        .expect("tmux_error_code returns known codes"),
                    None => self.unavailable(CREDENTIAL_FN, error),
                })?;
            let token = parse_credential(&raw, account).ok_or_else(|| {
                AccountError::Unavailable(format!("{CREDENTIAL_FN} answered no access_token"))
            })?;
            if token.expires_at_ms.is_some_and(|expires| expires <= now) {
                return Err(AccountError::TokenExpired(account.to_string()));
            }
            let mut tokens = self.tokens.lock().expect("token lock");
            if token.fresh_at(now) {
                tokens.insert(account.to_string(), token.clone());
            } else {
                tokens.remove(account);
            }
            Ok(token)
        })
    }

    fn invalidate(&self, account: &str) {
        self.tokens.lock().expect("token lock").remove(account);
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
            let id = raw
                .get("global")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty() && *id != GLOBAL)
                .ok_or_else(|| {
                    AccountError::Unavailable(format!("{ACCOUNTS_LIST_FN} named no global account"))
                })?
                .to_string();
            *self.global.lock().expect("global lock") = Some((Instant::now(), id.clone()));
            Ok(id)
        })
    }
}

/// `{account, access_token, expires_at_ms, email?, plan?}` → the token.
fn parse_credential(raw: &Value, account: &str) -> Option<AccountToken> {
    let access_token = raw
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())?;
    Some(AccountToken {
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

/// Where a call's credential came from.
#[derive(Debug, Clone, PartialEq)]
pub enum Credential {
    /// The account source holds the accounts: this account's token.
    Account(AccountToken),
    /// No account source: today's vault / `~/.claude` credential, if any.
    Legacy(Option<Value>),
}

/// The credential of a call: `requested`'s token from `source` while it holds
/// the accounts (absent or `"global"` means the global account, named by its
/// id first), else what `legacy` finds. `legacy` is the only path that may
/// touch the vault, and it runs only when the source is not available.
pub async fn credential<F, Fut>(
    source: &dyn AccountSource,
    requested: Option<&str>,
    legacy: F,
) -> Result<Credential, AccountError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<Value>>,
{
    if !source.available().await {
        return Ok(Credential::Legacy(legacy().await));
    }
    let account = match requested {
        Some(id) if id != GLOBAL => id.to_string(),
        _ => source.global().await?,
    };
    source.token(&account).await.map(Credential::Account)
}

/// After a 401: drop the refused token and ask once more. `Some` only for a
/// different token, the one worth another request; the same token again is
/// dropped as well, so no later call is served the refused one from the
/// cache. A caller whose second request is refused too drops that token
/// ([`AccountSource::invalidate`]).
pub async fn ask_again(source: &dyn AccountSource, refused: &AccountToken) -> Option<AccountToken> {
    source.invalidate(&refused.account);
    match source.token(&refused.account).await {
        Ok(fresh) if fresh.access_token != refused.access_token => Some(fresh),
        _ => {
            source.invalidate(&refused.account);
            None
        }
    }
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
    async fn without_tmux_the_legacy_path_runs_and_tmux_is_not_asked() {
        let engine = FakeEngine::new(|function_id, _| match function_id {
            "engine::functions::list" => Ok(function_list(false)),
            other => panic!("unexpected call {other}"),
        });
        let source = TmuxAccounts::new(engine.clone());
        let legacy = json!({ "type": "oauth", "access_token": "vault-token" });
        let expected = legacy.clone();
        let got = credential(&source, Some("personal"), || async move { Some(legacy) })
            .await
            .unwrap();
        assert_eq!(got, Credential::Legacy(Some(expected)));
        assert_eq!(engine.called_ids(), ["engine::functions::list"]);
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
            assert!(source.available().await);
        }
        assert_eq!(engine.count("engine::functions::list"), 1);

        let expiring = TmuxAccounts::with_ttl(engine.clone(), Duration::ZERO);
        assert!(expiring.available().await);
        assert!(expiring.available().await);
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
        assert!(source.available().await);
        fail.store(true, Ordering::SeqCst);
        assert!(source.available().await, "still managed through a hiccup");

        let never_known = TmuxAccounts::with_ttl(engine, Duration::ZERO);
        assert!(!never_known.available().await);
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
        let fresh = ask_again(&source, &refused).await.expect("a new token");
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
        assert_eq!(ask_again(&source, &refused).await, None);
        assert_eq!(engine.count(CREDENTIAL_FN), 2, "it did ask");
        // The refused token is not served from the cache afterwards.
        source.token("personal").await.unwrap();
        assert_eq!(engine.count(CREDENTIAL_FN), 3);
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
        assert!(source.available().await);
        let error = source.token("personal").await.unwrap_err();
        assert!(matches!(error, AccountError::Unavailable(_)), "{error:?}");
        assert_eq!(error.kind(), ErrorKind::Transient);
        assert!(source.available().await);
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

        let nameless = FakeEngine::new(|_, _| Ok(json!({ "global": "global" })));
        assert!(matches!(
            TmuxAccounts::new(nameless).global().await,
            Err(AccountError::Unavailable(_))
        ));
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
