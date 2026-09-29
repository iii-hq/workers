//! Provider-owned credential state. This scope is reserved before any read or
//! write: public state APIs and state-change subscriptions cannot expose it.
use crate::oauth::Credential;
use crate::session::SessionError;
use futures::future::BoxFuture;
use iii_sdk::{protocol::TriggerRequest, IIIClient};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const AUTH_SCOPE: &str = "provider-openai-codex-auth";
const PREFIX: &str = "provider-openai-codex";
const KEY: &str = "session";

/// None at the storage boundary means legacy credential resolution is allowed.
/// A record without a credential is an explicit logout and disables fallback.
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub revision: String,
    pub credential: Option<Credential>,
    pub needs_login: bool,
}

impl SessionRecord {
    pub fn connected(credential: Credential) -> Self {
        Self {
            revision: uuid::Uuid::new_v4().to_string(),
            credential: Some(credential),
            needs_login: false,
        }
    }

    pub fn logged_out() -> Self {
        Self {
            revision: uuid::Uuid::new_v4().to_string(),
            credential: None,
            needs_login: false,
        }
    }
}

pub trait SessionStore: Send + Sync {
    fn load(&self) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionError>>;
    fn compare_and_set<'a>(
        &'a self,
        expected: Option<&'a SessionRecord>,
        next: &'a SessionRecord,
    ) -> BoxFuture<'a, Result<bool, SessionError>>;
}

pub struct PrivateStateStore {
    iii: IIIClient,
}

impl PrivateStateStore {
    pub fn new(iii: IIIClient) -> Self {
        Self { iii }
    }

    async fn call(&self, id: &str, payload: Value) -> Result<Value, SessionError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: id.into(),
                payload,
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
            .map_err(|_| SessionError::storage())
    }

    async fn claim(&self) -> Result<(), SessionError> {
        // Idempotent and intentionally repeated: a restarted volatile state
        // worker forgets its claims. Do not treat an outage as an empty vault.
        self.call(
            "state::claim-namespace",
            json!({
                "functions_prefix": PREFIX, "scopes": [AUTH_SCOPE],
            }),
        )
        .await?;
        Ok(())
    }
}

impl SessionStore for PrivateStateStore {
    fn load(&self) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionError>> {
        Box::pin(async {
            self.claim().await?;
            let value = self
                .call(
                    "provider-openai-codex::state::get",
                    json!({ "scope": AUTH_SCOPE, "key": KEY }),
                )
                .await?;
            if value.is_null() {
                return Ok(None);
            }
            serde_json::from_value(value)
                .map(Some)
                .map_err(|_| SessionError::storage())
        })
    }

    fn compare_and_set<'a>(
        &'a self,
        expected: Option<&'a SessionRecord>,
        next: &'a SessionRecord,
    ) -> BoxFuture<'a, Result<bool, SessionError>> {
        Box::pin(async move {
            self.claim().await?;
            let response = self
                .call(
                    "provider-openai-codex::state::compare-and-set",
                    json!({
                        "scope": AUTH_SCOPE, "key": KEY, "expected": expected, "value": next,
                    }),
                )
                .await?;
            response
                .get("swapped")
                .and_then(Value::as_bool)
                .ok_or_else(SessionError::storage)
        })
    }
}
