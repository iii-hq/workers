use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use iii_sdk::TriggerRequest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use crate::io::IIITrigger;
use crate::{ClientRecord, KeyRecord, KeySet, RefreshTokenRecord};

pub const CLIENTS_SCOPE: &str = "auth:clients";
pub const JWKS_SCOPE: &str = "auth:jwks";
pub const TOKENS_SCOPE: &str = "auth:tokens";
const KEYSET_KEY: &str = "keyset";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RevokedTokenRecord {
    token_id: String,
    created_at: i64,
}

#[async_trait::async_trait]
pub trait AuthStore: Send + Sync {
    async fn get_client(&self, client_id: &str) -> anyhow::Result<Option<ClientRecord>>;
    async fn set_client(&self, client: ClientRecord) -> anyhow::Result<()>;
    async fn get_keyset(&self) -> anyhow::Result<Option<KeySet>>;
    async fn set_keyset(&self, keyset: KeySet) -> anyhow::Result<()>;
    async fn create_keyset_if_absent(&self, keyset: KeySet) -> anyhow::Result<KeySet>;
    async fn rotate_keyset(
        &self,
        new_key: KeyRecord,
        current_time: i64,
        rotation_overlap_seconds: i64,
    ) -> anyhow::Result<KeySet>;
    async fn get_refresh_token(&self, token_id: &str)
        -> anyhow::Result<Option<RefreshTokenRecord>>;
    async fn set_refresh_token(&self, record: RefreshTokenRecord) -> anyhow::Result<()>;
    async fn rotate_refresh_token(
        &self,
        old_token_id: &str,
        new_record: RefreshTokenRecord,
    ) -> anyhow::Result<()>;
    async fn is_revoked(&self, token_id: &str) -> anyhow::Result<bool>;
    async fn revoke(&self, token_id: &str) -> anyhow::Result<()>;
    async fn cleanup_expired_tokens(
        &self,
        current_time: i64,
        revoked_retention_seconds: i64,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryAuthStore {
    clients: Arc<RwLock<HashMap<String, ClientRecord>>>,
    keyset: Arc<RwLock<Option<KeySet>>>,
    refresh_tokens: Arc<RwLock<HashMap<String, RefreshTokenRecord>>>,
    revoked: Arc<RwLock<HashMap<String, i64>>>,
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

    async fn create_keyset_if_absent(&self, keyset: KeySet) -> anyhow::Result<KeySet> {
        let mut current = self.keyset.write().await;
        if let Some(existing) = current.clone() {
            return Ok(existing);
        }
        *current = Some(keyset.clone());
        Ok(keyset)
    }

    async fn rotate_keyset(
        &self,
        new_key: KeyRecord,
        current_time: i64,
        rotation_overlap_seconds: i64,
    ) -> anyhow::Result<KeySet> {
        let mut current = self.keyset.write().await;
        let mut keyset = current.clone().unwrap_or_else(|| KeySet {
            current_kid: new_key.kid.clone(),
            keys: Vec::new(),
        });
        if !keyset.keys.is_empty() {
            for key in &mut keyset.keys {
                if key.kid == keyset.current_kid && key.retire_after.is_none() {
                    key.retire_after = Some(current_time + rotation_overlap_seconds);
                }
            }
            keyset
                .keys
                .retain(|key| key.retire_after.is_none_or(|retire| retire > current_time));
        }
        keyset.current_kid.clone_from(&new_key.kid);
        keyset.keys.push(new_key);
        *current = Some(keyset.clone());
        Ok(keyset)
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

    async fn rotate_refresh_token(
        &self,
        old_token_id: &str,
        new_record: RefreshTokenRecord,
    ) -> anyhow::Result<()> {
        let mut refresh_tokens = self.refresh_tokens.write().await;
        let mut revoked = self.revoked.write().await;
        refresh_tokens.insert(new_record.token_id.clone(), new_record);
        revoked.insert(old_token_id.to_string(), Utc::now().timestamp());
        Ok(())
    }

    async fn is_revoked(&self, token_id: &str) -> anyhow::Result<bool> {
        Ok(self.revoked.read().await.contains_key(token_id))
    }

    async fn revoke(&self, token_id: &str) -> anyhow::Result<()> {
        self.revoked
            .write()
            .await
            .insert(token_id.to_string(), Utc::now().timestamp());
        Ok(())
    }

    async fn cleanup_expired_tokens(
        &self,
        current_time: i64,
        revoked_retention_seconds: i64,
    ) -> anyhow::Result<()> {
        self.refresh_tokens
            .write()
            .await
            .retain(|_, record| record.expires_at > current_time);
        let revoked_cutoff = current_time.saturating_sub(revoked_retention_seconds);
        self.revoked
            .write()
            .await
            .retain(|_, created_at| *created_at >= revoked_cutoff);
        Ok(())
    }
}

pub struct IiiStateAuthStore {
    iii: Arc<dyn IIITrigger>,
    timeout_ms: u64,
    lock: Arc<Mutex<()>>,
}

impl IiiStateAuthStore {
    pub fn new(iii: Arc<dyn IIITrigger>, timeout_ms: u64) -> Self {
        Self {
            iii,
            timeout_ms,
            lock: Arc::new(Mutex::new(())),
        }
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

    async fn delete_value(&self, scope: &str, key: &str) -> anyhow::Result<()> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::delete".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::delete failed: {e}"))?;
        Ok(())
    }

    async fn list_values(&self, scope: &str) -> anyhow::Result<Vec<Value>> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": scope }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::list failed: {e}"))?;
        serde_json::from_value(resp).map_err(Into::into)
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
        self.get_value(JWKS_SCOPE, KEYSET_KEY)
            .await?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    async fn set_keyset(&self, keyset: KeySet) -> anyhow::Result<()> {
        self.set_value(JWKS_SCOPE, KEYSET_KEY, serde_json::to_value(keyset)?)
            .await
    }

    async fn create_keyset_if_absent(&self, keyset: KeySet) -> anyhow::Result<KeySet> {
        let _guard = self.lock.lock().await;
        if let Some(existing) = self.get_keyset().await? {
            return Ok(existing);
        }
        self.set_keyset(keyset.clone()).await?;
        Ok(keyset)
    }

    async fn rotate_keyset(
        &self,
        new_key: KeyRecord,
        current_time: i64,
        rotation_overlap_seconds: i64,
    ) -> anyhow::Result<KeySet> {
        let _guard = self.lock.lock().await;
        let mut keyset = self.get_keyset().await?.unwrap_or_else(|| KeySet {
            current_kid: new_key.kid.clone(),
            keys: Vec::new(),
        });
        if !keyset.keys.is_empty() {
            for key in &mut keyset.keys {
                if key.kid == keyset.current_kid && key.retire_after.is_none() {
                    key.retire_after = Some(current_time + rotation_overlap_seconds);
                }
            }
            keyset
                .keys
                .retain(|key| key.retire_after.is_none_or(|retire| retire > current_time));
        }
        keyset.current_kid.clone_from(&new_key.kid);
        keyset.keys.push(new_key);
        self.set_keyset(keyset.clone()).await?;
        Ok(keyset)
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

    async fn rotate_refresh_token(
        &self,
        old_token_id: &str,
        new_record: RefreshTokenRecord,
    ) -> anyhow::Result<()> {
        let _guard = self.lock.lock().await;
        self.set_refresh_token(new_record.clone()).await?;
        if let Err(err) = self.revoke(old_token_id).await {
            let _ = self
                .delete_value(TOKENS_SCOPE, &format!("refresh:{}", new_record.token_id))
                .await;
            return Err(err);
        }
        Ok(())
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
            serde_json::to_value(RevokedTokenRecord {
                token_id: token_id.to_string(),
                created_at: Utc::now().timestamp(),
            })?,
        )
        .await
    }

    async fn cleanup_expired_tokens(
        &self,
        current_time: i64,
        revoked_retention_seconds: i64,
    ) -> anyhow::Result<()> {
        let _guard = self.lock.lock().await;
        let revoked_cutoff = current_time.saturating_sub(revoked_retention_seconds);
        for value in self.list_values(TOKENS_SCOPE).await? {
            if let Ok(record) = serde_json::from_value::<RefreshTokenRecord>(value.clone()) {
                if record.expires_at <= current_time {
                    self.delete_value(TOKENS_SCOPE, &format!("refresh:{}", record.token_id))
                        .await?;
                }
                continue;
            }
            if let Ok(record) = serde_json::from_value::<RevokedTokenRecord>(value) {
                if record.created_at < revoked_cutoff {
                    self.delete_value(TOKENS_SCOPE, &format!("revoked:{}", record.token_id))
                        .await?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_cleanup_prunes_expired_tokens() -> anyhow::Result<()> {
        let store = InMemoryAuthStore::new();
        store
            .set_refresh_token(RefreshTokenRecord {
                token_id: "expired".to_string(),
                client_id: "client".to_string(),
                subject: "client".to_string(),
                scopes: vec!["mcp:tools".to_string()],
                expires_at: Utc::now().timestamp() - 1,
            })
            .await?;
        store.revoke("old-revoked").await?;
        store
            .cleanup_expired_tokens(Utc::now().timestamp() + 1, 0)
            .await?;
        assert!(store.get_refresh_token("expired").await?.is_none());
        assert!(!store.is_revoked("old-revoked").await?);
        Ok(())
    }
}
