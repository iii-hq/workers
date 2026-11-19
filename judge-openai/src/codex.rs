//! `auth: codex`: the bearer is the ChatGPT-login access token that
//! provider-openai-codex hands out through `provider::openai-codex::auth::token`.
//! This worker stores no token: each handler invocation asks for the current one,
//! and a 401 forces one refresh at most every five minutes, worker-wide.
use iii_sdk::{CallRequest, IIIClient, errors::Error};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};

const TOKEN_ID: &str = "provider::openai-codex::auth::token";
const TOKEN_TIMEOUT_MS: u64 = 10_000;
const FORCE_INTERVAL: Duration = Duration::from_secs(300);
pub const NOT_SIGNED_IN: &str =
    "not signed in to Codex; sign in with ChatGPT in the provider-openai-codex settings";

type FetchFuture = Pin<Box<dyn Future<Output = Result<Option<String>, String>> + Send>>;
/// One token request: the argument is the token OpenAI rejected, `Ok(None)` means
/// signed out, `Err` is the operator-facing reason.
pub type Fetch = Arc<dyn Fn(Option<String>) -> FetchFuture + Send + Sync>;

/// [`Fetch`] over the bus; provider-openai-codex checks the engine-stamped caller.
pub fn bus_fetch(iii: IIIClient) -> Fetch {
    Arc::new(move |rejected: Option<String>| {
        let iii = iii.clone();
        Box::pin(async move {
            let payload = match rejected {
                Some(rejected_token) => json!({ "rejected_token": rejected_token }),
                None => json!({}),
            };
            let response = iii
                .call(CallRequest {
                    function_id: TOKEN_ID.into(),
                    payload,
                    action: None,
                    timeout_ms: Some(TOKEN_TIMEOUT_MS),
                })
                .await
                .map_err(|error| describe(&error))?;
            match response {
                Value::Object(mut fields) => match fields.remove("access_token") {
                    Some(Value::String(token)) => Ok(Some(token)),
                    Some(Value::Null) => Ok(None),
                    _ => Err(unreadable("malformed_response")),
                },
                _ => Err(unreadable("malformed_response")),
            }
        })
    })
}

/// The fix for a failed token request, by code only: remote messages are not echoed.
fn describe(error: &Error) -> String {
    match error {
        Error::Remote { code, .. } => match code.as_str() {
            "function_not_found" => "provider-openai-codex is not running, or older than the version with auth::token; start or upgrade it, or set auth: api_key".into(),
            "provider/auth_expired" => "the Codex session expired; sign in with ChatGPT again".into(),
            "provider/forbidden" => "provider-openai-codex refused judge-openai (different namespace?)".into(),
            code => unreadable(code.strip_prefix("provider/").unwrap_or(code)),
        },
        Error::Timeout => unreadable("timeout"),
        _ => unreadable("invocation_failed"),
    }
}

fn unreadable(code: &str) -> String {
    format!("could not read the Codex session ({code}); retry")
}

/// Worker-wide Codex credential: `latest` is the bearer every HTTP attempt sends.
pub struct CodexSource {
    fetch: Fetch,
    latest: std::sync::Mutex<Option<Arc<str>>>,
    /// Held across a forced fetch so concurrent 401s wait and reuse its result.
    forced_at: Mutex<Option<Instant>>,
}
impl std::fmt::Debug for CodexSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodexSource([REDACTED])")
    }
}
impl CodexSource {
    pub fn new(fetch: Fetch) -> Self {
        Self {
            fetch,
            latest: Default::default(),
            forced_at: Mutex::default(),
        }
    }

    /// Ask for the current token; once per handler invocation, nothing cached across calls.
    pub async fn current(&self) -> Result<Arc<str>, String> {
        self.get(None).await
    }

    pub fn latest(&self) -> Option<Arc<str>> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// A token to retry with after OpenAI rejected `rejected`: one a sibling call
    /// already renewed, else a forced refresh at most once per [`FORCE_INTERVAL`].
    /// `Ok(None)` when there is nothing new to try.
    pub async fn renew(&self, rejected: &str) -> Result<Option<Arc<str>>, String> {
        let mut forced_at = self.forced_at.lock().await;
        if let Some(latest) = self.latest().filter(|latest| **latest != *rejected) {
            return Ok(Some(latest));
        }
        if forced_at.is_some_and(|at| at.elapsed() < FORCE_INTERVAL) {
            return Ok(None);
        }
        *forced_at = Some(Instant::now());
        let token = self.get(Some(rejected.to_owned())).await?;
        Ok((*token != *rejected).then_some(token))
    }

    async fn get(&self, rejected: Option<String>) -> Result<Arc<str>, String> {
        let token: Arc<str> = (self.fetch)(rejected)
            .await?
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| NOT_SIGNED_IN.to_owned())?
            .into();
        *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
        Ok(token)
    }
}
