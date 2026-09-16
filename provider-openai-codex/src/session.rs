//! Owns the provider's session, refresh serialization, and cancellable device
//! login. The UI sees ceremony/status data only, never OAuth credentials.
use crate::credential_store::{PrivateStateStore, SessionRecord, SessionStore};
use crate::oauth::{Credential, DeviceCode, OAuthClient, OAuthError, PollOutcome};
use futures::future::BoxFuture;
use iii_sdk::{errors::Error, IIIClient};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{watch, Mutex};

fn now_seconds() -> i64 {
    crate::now_ms() / 1000
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionError {
    pub code: String,
    pub message: String,
}

impl SessionError {
    pub fn storage() -> Self {
        Self::new(
            "storage_unavailable",
            "Could not read or save the Codex session. Check the state worker and retry.",
        )
    }
    fn expired() -> Self {
        Self::new(
            "auth_expired",
            "Your Codex session has expired. Sign in with ChatGPT again.",
        )
    }
    fn changed() -> Self {
        Self::new(
            "session_changed",
            "The Codex session changed. Retry the operation.",
        )
    }
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn into_bus(self) -> Error {
        Error::Remote {
            code: format!("provider/{}", self.code),
            message: self.message,
            stacktrace: None,
        }
    }
    pub fn error_kind(&self) -> llm_router::types::events::ErrorKind {
        use llm_router::types::events::ErrorKind;
        if self.code == "auth_expired" {
            ErrorKind::AuthExpired
        } else {
            ErrorKind::Transient
        }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SessionError {}
impl From<OAuthError> for SessionError {
    fn from(e: OAuthError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    Managed,
    Vault,
    Local,
}

pub struct ResolvedCredential {
    pub value: Value,
    pub source: CredentialSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    SignedOut,
    Authenticated,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    Pending,
    Ok,
    Expired,
    Canceled,
    Error,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct LoginStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    pub user_code: String,
    /// Unix timestamp in seconds.
    pub expires_at: i64,
    /// Seconds between status requests from the UI.
    pub interval: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LoginPollResponse {
    pub status: LoginStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SessionError>,
}

impl LoginPollResponse {
    fn status(status: LoginStatus) -> Self {
        Self {
            status,
            error: None,
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub struct AuthStatusResponse {
    pub status: AuthStatus,
    pub source: Option<CredentialSource>,
    pub account_id: Option<String>,
    pub login: Option<LoginStartResponse>,
}

#[derive(Deserialize, JsonSchema)]
pub struct EmptyRequest {}
#[derive(Deserialize, JsonSchema)]
pub struct LoginIdRequest {
    pub login_id: String,
}
#[derive(Serialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

pub trait OAuthTransport: Send + Sync {
    fn start(&self) -> BoxFuture<'_, Result<DeviceCode, OAuthError>>;
    fn poll<'a>(&'a self, device: &'a DeviceCode)
        -> BoxFuture<'a, Result<PollOutcome, OAuthError>>;
    fn refresh<'a>(
        &'a self,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Credential, OAuthError>>;
}

impl OAuthTransport for OAuthClient {
    fn start(&self) -> BoxFuture<'_, Result<DeviceCode, OAuthError>> {
        Box::pin(self.start())
    }
    fn poll<'a>(
        &'a self,
        device: &'a DeviceCode,
    ) -> BoxFuture<'a, Result<PollOutcome, OAuthError>> {
        Box::pin(self.poll(device))
    }
    fn refresh<'a>(
        &'a self,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Credential, OAuthError>> {
        Box::pin(self.refresh(credential))
    }
}

pub trait LegacySource: Send + Sync {
    fn resolve(&self) -> BoxFuture<'_, Result<Option<ResolvedCredential>, SessionError>>;
}

struct LegacyCredentials(IIIClient);
impl LegacySource for LegacyCredentials {
    fn resolve(&self) -> BoxFuture<'_, Result<Option<ResolvedCredential>, SessionError>> {
        Box::pin(crate::auth::fetch_legacy_credential(&self.0))
    }
}

struct Attempt {
    public: LoginStartResponse,
    outcome: LoginPollResponse,
    cancel: watch::Sender<bool>,
}

struct PendingRefresh {
    expected: SessionRecord,
    next: SessionRecord,
}

pub struct AuthManager {
    store: Arc<dyn SessionStore>,
    oauth: Arc<dyn OAuthTransport>,
    legacy: Arc<dyn LegacySource>,
    // Login completion/logout must not interleave. Refresh uses storage CAS
    // instead, so logout never waits on a slow token endpoint.
    transition: Mutex<()>,
    refresh: Mutex<Option<PendingRefresh>>,
    attempt: StdMutex<Option<Attempt>>,
    changes: watch::Sender<u64>,
}

impl AuthManager {
    pub fn new(iii: IIIClient, http: reqwest::Client) -> Arc<Self> {
        Self::with_dependencies(
            Arc::new(PrivateStateStore::new(iii.clone())),
            Arc::new(OAuthClient::new(http)),
            Arc::new(LegacyCredentials(iii)),
        )
    }

    pub fn with_dependencies(
        store: Arc<dyn SessionStore>,
        oauth: Arc<dyn OAuthTransport>,
        legacy: Arc<dyn LegacySource>,
    ) -> Arc<Self> {
        let (changes, _) = watch::channel(0);
        Arc::new(Self {
            store,
            oauth,
            legacy,
            transition: Mutex::new(()),
            refresh: Mutex::new(None),
            attempt: StdMutex::new(None),
            changes,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// `rejected_token` forces one refresh after a 401, only if that exact
    /// credential is still current. Concurrent callers reuse the rotation.
    pub async fn resolve(
        self: &Arc<Self>,
        rejected_token: Option<&str>,
    ) -> Result<Option<ResolvedCredential>, SessionError> {
        // OAuth may consume a refresh token before returning its replacement.
        // Keep resolution alive if a stream or RPC caller is canceled, so the
        // replacement is still persisted (or retained for a storage retry).
        let manager = self.clone();
        let rejected_token = rejected_token.map(str::to_owned);
        tokio::spawn(async move { manager.resolve_inner(rejected_token.as_deref()).await })
            .await
            .map_err(|_| {
                SessionError::new(
                    "session_unavailable",
                    "Could not resolve the Codex session. Retry the operation.",
                )
            })?
    }

    async fn resolve_inner(
        &self,
        rejected_token: Option<&str>,
    ) -> Result<Option<ResolvedCredential>, SessionError> {
        let mut pending = self.refresh.lock().await;
        for _ in 0..3 {
            let stored = self.store.load().await?;
            // A successful refresh followed by a storage outage is retained
            // in memory. Retry its write, not the consumed refresh token.
            if let Some(write) = pending.as_ref() {
                if stored
                    .as_ref()
                    .is_some_and(|r| r.revision == write.expected.revision)
                {
                    let swapped = self
                        .store
                        .compare_and_set(Some(&write.expected), &write.next)
                        .await?;
                    if swapped {
                        let credential =
                            write.next.credential.as_ref().expect("refresh credential");
                        let resolved = ResolvedCredential {
                            value: serde_json::to_value(credential).expect("credential serializes"),
                            source: CredentialSource::Managed,
                        };
                        let expired = credential.expires_at <= now_seconds();
                        *pending = None;
                        // This call has already renewed the session, even if
                        // the server preserved its access token.
                        return if expired {
                            Err(SessionError::expired())
                        } else {
                            Ok(Some(resolved))
                        };
                    }
                    *pending = None;
                    continue;
                }
                *pending = None; // logout/replacement won
            }
            let Some(record) = stored else {
                let legacy = self.legacy.resolve().await?;
                if let Some(ref resolved) = legacy {
                    if crate::auth::credential_expires_at(&resolved.value)
                        .is_some_and(|exp| exp <= now_seconds())
                    {
                        return Err(SessionError::expired());
                    }
                }
                return Ok(legacy);
            };
            let Some(ref credential) = record.credential else {
                return Ok(None);
            };
            if record.needs_login {
                return Err(SessionError::expired());
            }
            let forced = rejected_token == Some(credential.access_token.as_str());
            if !forced && credential.expires_at > now_seconds() + 60 {
                return Ok(Some(ResolvedCredential {
                    value: serde_json::to_value(credential).expect("credential serializes"),
                    source: CredentialSource::Managed,
                }));
            }
            if credential
                .refresh_token
                .as_deref()
                .is_none_or(str::is_empty)
            {
                return Err(SessionError::expired());
            }
            match self.oauth.refresh(credential).await {
                Ok(fresh) => {
                    let write = PendingRefresh {
                        expected: record,
                        next: SessionRecord::connected(fresh),
                    };
                    *pending = Some(write);
                    // Next pass stores the rotated token before returning it.
                }
                Err(e) if e.invalidates_session() => {
                    let mut expired = record.clone();
                    expired.revision = uuid::Uuid::new_v4().to_string();
                    expired.needs_login = true;
                    if self.store.compare_and_set(Some(&record), &expired).await? {
                        return Err(SessionError::expired());
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(SessionError::changed())
    }

    pub async fn status(self: &Arc<Self>) -> Result<AuthStatusResponse, SessionError> {
        let (status, source, account_id) = match self.resolve(None).await {
            Ok(Some(resolved)) => (
                AuthStatus::Authenticated,
                Some(resolved.source),
                account_id(&resolved.value),
            ),
            Ok(None) => (AuthStatus::SignedOut, None, None),
            Err(e) if e.code == "auth_expired" => {
                let stored = self.store.load().await?;
                if let Some(record) = stored {
                    match record.credential {
                        Some(credential) => (
                            if !record.needs_login && credential.expires_at > now_seconds() + 60 {
                                AuthStatus::Authenticated
                            } else {
                                AuthStatus::Expired
                            },
                            Some(CredentialSource::Managed),
                            Some(credential.account_id),
                        ),
                        None => (AuthStatus::SignedOut, None, None),
                    }
                } else {
                    let legacy = self.legacy.resolve().await?;
                    (
                        AuthStatus::Expired,
                        legacy.as_ref().map(|r| r.source),
                        legacy.as_ref().and_then(|r| account_id(&r.value)),
                    )
                }
            }
            Err(e) => return Err(e),
        };
        let login = self
            .attempt
            .lock()
            .expect("attempt mutex")
            .as_ref()
            .filter(|a| a.outcome.status == LoginStatus::Pending)
            .map(|a| a.public.clone());
        Ok(AuthStatusResponse {
            status,
            source,
            account_id,
            login,
        })
    }

    pub async fn start(self: &Arc<Self>) -> Result<LoginStartResponse, SessionError> {
        let _transition = self.transition.lock().await;
        self.store.load().await?; // fail before starting OAuth if state is down
        self.cancel_current();
        let device = self.oauth.start().await?;
        let public = LoginStartResponse {
            login_id: uuid::Uuid::new_v4().to_string(),
            verification_uri: device.verification_uri.clone(),
            user_code: device.user_code.clone(),
            expires_at: device.expires_at,
            interval: device.interval.max(1),
        };
        let (cancel, mut canceled) = watch::channel(false);
        *self.attempt.lock().expect("attempt mutex") = Some(Attempt {
            public: public.clone(),
            outcome: LoginPollResponse::status(LoginStatus::Pending),
            cancel,
        });
        let manager = self.clone();
        let login_id = public.login_id.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = canceled.wait_for(|c| *c) => {},
                _ = manager.run_login(&login_id, device) => {},
            }
        });
        Ok(public)
    }

    pub fn poll(&self, login_id: &str) -> LoginPollResponse {
        self.attempt
            .lock()
            .expect("attempt mutex")
            .as_ref()
            .filter(|a| a.public.login_id == login_id)
            .map(|a| a.outcome.clone())
            .unwrap_or_else(|| LoginPollResponse::status(LoginStatus::Canceled))
    }

    pub async fn cancel(&self, login_id: &str) {
        let _transition = self.transition.lock().await;
        let mut attempt = self.attempt.lock().expect("attempt mutex");
        if let Some(a) = attempt
            .as_mut()
            .filter(|a| a.public.login_id == login_id && a.outcome.status == LoginStatus::Pending)
        {
            a.outcome = LoginPollResponse::status(LoginStatus::Canceled);
            a.cancel.send_replace(true);
        }
    }

    fn cancel_current(&self) {
        if let Some(a) = self.attempt.lock().expect("attempt mutex").as_mut() {
            a.cancel.send_replace(true);
            if a.outcome.status == LoginStatus::Pending {
                a.outcome = LoginPollResponse::status(LoginStatus::Canceled);
            }
        }
    }

    pub async fn logout(&self) -> Result<(), SessionError> {
        let _transition = self.transition.lock().await;
        self.cancel_current();
        for _ in 0..3 {
            let current = self.store.load().await?;
            if self
                .store
                .compare_and_set(current.as_ref(), &SessionRecord::logged_out())
                .await?
            {
                return Ok(());
            }
        }
        Err(SessionError::changed())
    }

    fn finish(&self, login_id: &str, outcome: LoginPollResponse) {
        if let Some(a) = self
            .attempt
            .lock()
            .expect("attempt mutex")
            .as_mut()
            .filter(|a| a.public.login_id == login_id && a.outcome.status == LoginStatus::Pending)
        {
            a.outcome = outcome;
        }
    }

    async fn run_login(&self, login_id: &str, device: DeviceCode) {
        let lifetime = device.expires_at.saturating_sub(now_seconds()).max(0) as u64;
        // Expiry bounds the OAuth ceremony, not an already-authorized durable
        // write. Dropping a remote CAS at its deadline could still commit it.
        let result = tokio::time::timeout(
            Duration::from_secs(lifetime),
            self.poll_until_authorized(device),
        )
        .await;
        let credential = match result {
            Err(_) => {
                self.finish(login_id, LoginPollResponse::status(LoginStatus::Expired));
                return;
            }
            Ok(Err(error)) => {
                self.finish(
                    login_id,
                    LoginPollResponse {
                        status: LoginStatus::Error,
                        error: Some(error),
                    },
                );
                return;
            }
            Ok(Ok(credential)) => credential,
        };
        let _transition = self.transition.lock().await;
        if self.poll(login_id).status != LoginStatus::Pending {
            return;
        }
        let result = async {
            let next = SessionRecord::connected(credential);
            // Logout, cancel, and replacement login are excluded by the
            // transition lock. A refresh of the old session can still win a
            // CAS race; retry without discarding the authorized new account.
            for _ in 0..3 {
                let current = self.store.load().await?;
                if self.store.compare_and_set(current.as_ref(), &next).await? {
                    return Ok(());
                }
            }
            Err(SessionError::changed())
        }
        .await;
        match result {
            Ok(()) => {
                self.finish(login_id, LoginPollResponse::status(LoginStatus::Ok));
                self.changes.send_modify(|n| *n = n.wrapping_add(1));
            }
            Err(error) => self.finish(
                login_id,
                LoginPollResponse {
                    status: LoginStatus::Error,
                    error: Some(error),
                },
            ),
        }
    }

    async fn poll_until_authorized(&self, device: DeviceCode) -> Result<Credential, SessionError> {
        let mut delay = device.interval.max(1);
        loop {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            match self.oauth.poll(&device).await {
                Ok(PollOutcome::Pending { retry_after_secs }) => {
                    delay = retry_after_secs.max(device.interval).max(1);
                }
                Err(e) if !e.permanent => {
                    delay = e
                        .retry_after_secs
                        .unwrap_or(delay.saturating_mul(2).min(30))
                        .max(device.interval)
                        .max(1);
                }
                Err(e) => return Err(e.into()),
                Ok(PollOutcome::Authorized(credential)) => return Ok(credential),
            }
        }
    }
}

fn account_id(value: &Value) -> Option<String> {
    let inner = value.get("credential").unwrap_or(value);
    inner
        .get("account_id")
        .and_then(Value::as_str)
        .or_else(|| {
            inner
                .pointer("/provider_extra/account_id")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .or_else(|| {
            inner
                .get("access_token")
                .and_then(Value::as_str)
                .and_then(crate::auth::account_id_from_access_token)
        })
}

#[cfg(test)]
mod tests;
