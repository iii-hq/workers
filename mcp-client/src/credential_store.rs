//! One OAuth session per server in the vault secret `MCP_CLIENT_<SERVER>_OAUTH`:
//! `{version: 1, resource: <server url>, credentials: <StoredCredentials | null>}`,
//! written only by compare-and-set and read again on every load.

use std::sync::Arc;

use rmcp::transport::auth::{
    AuthError, CredentialRefreshGuard, CredentialStore, StoredCredentials,
};
use serde_json::{Value, json};

use crate::host::{Host, SecretError};
use crate::translate::ServerSecrets;

/// `MCP_CLIENT_<SERVER>_OAUTH`, the server name upper-cased with `-` as `_`.
pub fn oauth_secret_name(server: &str) -> String {
    format!(
        "MCP_CLIENT_{}_OAUTH",
        server.to_ascii_uppercase().replace('-', "_")
    )
}

pub fn encode(resource: &str, credentials: Option<&Value>) -> String {
    json!({"version": 1, "resource": resource, "credentials": credentials}).to_string()
}

/// The credentials of a version 1 envelope bound to `resource`; `None` for
/// anything else and for `credentials: null`.
pub fn decode(text: &str, resource: &str) -> Option<Value> {
    let mut envelope: Value = serde_json::from_str(text).ok()?;
    if envelope["version"] != 1 || envelope["resource"] != resource {
        return None;
    }
    envelope
        .as_object_mut()?
        .remove("credentials")
        .filter(|c| !c.is_null())
}

/// Adds every string under an `access_token`, `refresh_token` or `id_token`
/// key of `value`: rmcp keeps a token response's extra fields.
fn add_tokens(secrets: &ServerSecrets, value: &Value) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                match (key.as_str(), v) {
                    ("access_token" | "refresh_token" | "id_token", Value::String(token)) => {
                        secrets.add(token)
                    }
                    _ => add_tokens(secrets, v),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| add_tokens(secrets, v)),
        _ => {}
    }
}

/// Never carries the stored text, a token or a `SecretError`'s own text.
fn store_error(text: &str) -> AuthError {
    AuthError::CredentialStoreError(text.into())
}

/// Kept apart from "sign in again" by rmcp: an outage of the secrets worker
/// never signs anyone out.
fn secrets_down(_: SecretError) -> AuthError {
    store_error("the secrets worker did not answer")
}

/// The OAuth session of one server. Clones share one refresh mutex, so one
/// store per server, cloned into every rmcp manager, makes the guard per
/// server. No `Debug`: it reads and writes tokens.
#[derive(Clone)]
pub struct VaultCredentialStore {
    host: Arc<dyn Host>,
    name: String,
    reference: String,
    resource: String,
    on_rejected: Arc<dyn Fn() + Send + Sync>,
    refresh: Arc<tokio::sync::Mutex<()>>,
    secrets: Option<Arc<ServerSecrets>>,
}

impl VaultCredentialStore {
    pub fn new(
        host: Arc<dyn Host>,
        server: &str,
        resource: &str,
        on_rejected: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let name = oauth_secret_name(server);
        VaultCredentialStore {
            host,
            reference: format!("secret://{name}"),
            name,
            resource: resource.into(),
            on_rejected,
            refresh: Arc::default(),
            secrets: None,
        }
    }

    /// Every token this store loads or saves joins `secrets`, so a server
    /// that echoes one has it scrubbed from error texts and log lines.
    pub fn with_secrets(mut self, secrets: Arc<ServerSecrets>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Clears the session without calling `on_rejected`. It waits for a
    /// refresh in flight, which would otherwise save its tokens over the
    /// sign-out; never call it while holding a refresh guard.
    pub async fn sign_out(&self) -> Result<(), String> {
        let _refresh = self.refresh.lock().await;
        self.wipe().await.map_err(|e| e.to_string())
    }

    /// The stored text; `None` when the secret is absent.
    async fn read(&self) -> Result<Option<String>, AuthError> {
        match self.host.resolve(&self.reference).await {
            Ok(text) => Ok(Some(text)),
            Err(SecretError::NotFound) => Ok(None),
            Err(e) => Err(secrets_down(e)),
        }
    }

    /// The credentials of `text` for this resource, their tokens remembered.
    fn session(&self, text: &str) -> Option<Value> {
        let credentials = decode(text, &self.resource)?;
        self.remember(&credentials);
        Some(credentials)
    }

    fn remember(&self, credentials: &Value) {
        if let Some(secrets) = &self.secrets {
            add_tokens(secrets, credentials);
        }
    }

    async fn swap(&self, expected: Option<&str>, value: &str) -> Result<bool, AuthError> {
        self.host
            .compare_and_set(&self.name, expected, value)
            .await
            .map_err(secrets_down)
    }

    /// Writes `credentials: null`. Nothing stored writes nothing; a lost swap
    /// is tried again, so a sign-out wins over a save that landed in between.
    async fn wipe(&self) -> Result<(), AuthError> {
        let empty = encode(&self.resource, None);
        for _ in 0..3 {
            let Some(text) = self.read().await? else {
                return Ok(());
            };
            if self.swap(Some(&text), &empty).await? {
                return Ok(());
            }
        }
        Err(store_error(
            "the stored session kept changing during the sign-out",
        ))
    }
}

#[async_trait::async_trait]
impl CredentialStore for VaultCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let Some(text) = self.read().await? else {
            return Ok(None);
        };
        // A session this version cannot read is no session: sign in again.
        Ok(self
            .session(&text)
            .and_then(|c| serde_json::from_value(c).ok()))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let credentials = serde_json::to_value(&credentials)
            .map_err(|_| store_error("the session could not be encoded"))?;
        self.remember(&credentials);
        let expected = self.read().await?;
        let text = encode(&self.resource, Some(&credentials));
        if !self.swap(expected.as_deref(), &text).await? {
            return Err(store_error("the stored session changed during the save"));
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.wipe().await
    }

    /// Clears the stored session only when it still holds the rejected
    /// refresh token: one replaced since must stay.
    async fn on_refresh_token_rejected(
        &self,
        rejected: &StoredCredentials,
    ) -> Result<(), AuthError> {
        const TOKEN: &str = "/token_response/refresh_token";
        let Some(text) = self.read().await? else {
            return Ok(());
        };
        let stored = self.session(&text);
        let stored = stored.as_ref().and_then(|c| c.pointer(TOKEN));
        let rejected = serde_json::to_value(rejected).ok();
        if stored.is_none() || stored != rejected.as_ref().and_then(|c| c.pointer(TOKEN)) {
            return Ok(());
        }
        if self
            .swap(Some(&text), &encode(&self.resource, None))
            .await?
        {
            (self.on_rejected)();
        }
        Ok(())
    }

    async fn acquire_refresh_guard(&self) -> Result<Option<CredentialRefreshGuard>, AuthError> {
        Ok(Some(CredentialRefreshGuard::new(
            self.refresh.clone().lock_owned().await,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = "https://mcp.example/mcp";

    #[test]
    fn the_secret_name_is_upper_case_with_underscores() {
        assert_eq!(
            oauth_secret_name("linear-auto"),
            "MCP_CLIENT_LINEAR_AUTO_OAUTH"
        );
        assert_eq!(oauth_secret_name("a1"), "MCP_CLIENT_A1_OAUTH");
    }

    #[test]
    fn the_envelope_round_trips_and_is_bound_to_the_url() {
        let creds = json!({"client_id": "c1", "token_response": null});
        let text = encode(RES, Some(&creds));
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"version": 1, "resource": RES, "credentials": creds})
        );
        assert_eq!(decode(&text, RES), Some(creds));
        assert_eq!(decode(&text, "https://other.example/mcp"), None);
        assert_eq!(decode(&encode(RES, None), RES), None);
        assert_eq!(
            decode(&text.replace("\"version\":1", "\"version\":2"), RES),
            None
        );
        assert_eq!(decode("not json", RES), None);
    }
}
