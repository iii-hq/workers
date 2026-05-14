use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iii_sdk::TriggerRequest;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::io::IIITrigger;
use crate::{ClientRecord, KeySet, RefreshTokenRecord};

pub const CLIENTS_SCOPE: &str = "auth:clients";
pub const JWKS_SCOPE: &str = "auth:jwks";
pub const TOKENS_SCOPE: &str = "auth:tokens";

#[async_trait::async_trait]
pub trait AuthStore: Send + Sync {
    async fn get_client(&self, client_id: &str) -> anyhow::Result<Option<ClientRecord>>;
    async fn set_client(&self, client: ClientRecord) -> anyhow::Result<()>;
    async fn get_keyset(&self) -> anyhow::Result<Option<KeySet>>;
    async fn set_keyset(&self, keyset: KeySet) -> anyhow::Result<()>;
    async fn get_refresh_token(&self, token_id: &str)
        -> anyhow::Result<Option<RefreshTokenRecord>>;
    async fn set_refresh_token(&self, record: RefreshTokenRecord) -> anyhow::Result<()>;
    async fn is_revoked(&self, token_id: &str) -> anyhow::Result<bool>;
    async fn revoke(&self, token_id: &str) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryAuthStore {
    clients: Arc<RwLock<HashMap<String, ClientRecord>>>,
    keyset: Arc<RwLock<Option<KeySet>>>,
    refresh_tokens: Arc<RwLock<HashMap<String, RefreshTokenRecord>>>,
    revoked: Arc<RwLock<HashSet<String>>>,
}

impl InMemoryAuthStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl AuthStore for InMemoryAuthStore {
    async fn get_client(&self, client_id: &str) -> anyhow::Result<Option<ClientRecord>> {
        Ok(self.clients.read().await.get(client_id).cloned())
    }

    async fn set_client(&self, client: ClientRecord) -> anyhow::Result<()> {
        self.clients
            .write()
            .await
            .insert(client.client_id.clone(), client);
        Ok(())
    }

    async fn get_keyset(&self) -> anyhow::Result<Option<KeySet>> {
        Ok(self.keyset.read().await.clone())
    }

    async fn set_keyset(&self, keyset: KeySet) -> anyhow::Result<()> {
        *self.keyset.write().await = Some(keyset);
        Ok(())
    }

    async fn get_refresh_token(
        &self,
        token_id: &str,
    ) -> anyhow::Result<Option<RefreshTokenRecord>> {
        Ok(self.refresh_tokens.read().await.get(token_id).cloned())
    }

    async fn set_refresh_token(&self, record: RefreshTokenRecord) -> anyhow::Result<()> {
        self.refresh_tokens
            .write()
            .await
            .insert(record.token_id.clone(), record);
        Ok(())
    }

    async fn is_revoked(&self, token_id: &str) -> anyhow::Result<bool> {
        Ok(self.revoked.read().await.contains(token_id))
    }

    async fn revoke(&self, token_id: &str) -> anyhow::Result<()> {
        self.revoked.write().await.insert(token_id.to_string());
        Ok(())
    }
}

pub struct IiiStateAuthStore {
    iii: Arc<dyn IIITrigger>,
    timeout_ms: u64,
}

impl IiiStateAuthStore {
    pub fn new(iii: Arc<dyn IIITrigger>, timeout_ms: u64) -> Self {
        Self { iii, timeout_ms }
    }

    async fn get_value(&self, scope: &str, key: &str) -> anyhow::Result<Option<Value>> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::get failed: {e}"))?;
        if resp.is_null() {
            Ok(None)
        } else {
            Ok(Some(resp))
        }
    }

    async fn set_value(&self, scope: &str, key: &str, value: Value) -> anyhow::Result<()> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": scope, "key": key, "value": value }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::set failed: {e}"))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl AuthStore for IiiStateAuthStore {
    async fn get_client(&self, client_id: &str) -> anyhow::Result<Option<ClientRecord>> {
        self.get_value(CLIENTS_SCOPE, client_id)
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    async fn set_client(&self, client: ClientRecord) -> anyhow::Result<()> {
        let key = client.client_id.clone();
        self.set_value(CLIENTS_SCOPE, &key, serde_json::to_value(client)?)
            .await
    }

    async fn get_keyset(&self) -> anyhow::Result<Option<KeySet>> {
        self.get_value(JWKS_SCOPE, "keyset")
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    async fn set_keyset(&self, keyset: KeySet) -> anyhow::Result<()> {
        self.set_value(JWKS_SCOPE, "keyset", serde_json::to_value(keyset)?)
            .await
    }

    async fn get_refresh_token(
        &self,
        token_id: &str,
    ) -> anyhow::Result<Option<RefreshTokenRecord>> {
        self.get_value(TOKENS_SCOPE, &format!("refresh:{token_id}"))
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    async fn set_refresh_token(&self, record: RefreshTokenRecord) -> anyhow::Result<()> {
        self.set_value(
            TOKENS_SCOPE,
            &format!("refresh:{}", record.token_id),
            serde_json::to_value(record)?,
        )
        .await
    }

    async fn is_revoked(&self, token_id: &str) -> anyhow::Result<bool> {
        Ok(self
            .get_value(TOKENS_SCOPE, &format!("revoked:{token_id}"))
            .await?
            .is_some())
    }

    async fn revoke(&self, token_id: &str) -> anyhow::Result<()> {
        self.set_value(
            TOKENS_SCOPE,
            &format!("revoked:{token_id}"),
            json!({ "revoked": true }),
        )
        .await
    }
}
