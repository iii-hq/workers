//! Encrypted sessions with resumable migration from private state.
use super::{PrivateStateStore, SessionRecord, SessionStore};
use crate::session::SessionError;
use futures::future::BoxFuture;
use iii_sdk::{errors::Error, protocol::TriggerRequest, IIIClient};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const SECRET_NAME: &str = "PROVIDER_OPENAI_CODEX_SESSION";
const SECRET_REF: &str = "secret://PROVIDER_OPENAI_CODEX_SESSION";
const CONSUMER: &str = "provider-openai-codex";

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u8,
    session: Option<SessionRecord>,
    migration_complete: bool,
}

pub struct SecretsSessionStore {
    iii: IIIClient,
    legacy: PrivateStateStore,
    transition: Mutex<()>,
}

impl SecretsSessionStore {
    pub fn new(iii: IIIClient) -> Self {
        Self {
            legacy: PrivateStateStore::new(iii.clone()),
            iii,
            transition: Mutex::new(()),
        }
    }

    async fn call(&self, function: &str, payload: Value) -> Result<Value, Error> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function.into(),
                payload,
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
    }

    async fn read(&self) -> Result<Option<(String, Envelope)>, SessionError> {
        let response = match self
            .call("secrets::resolve", json!({"ref":SECRET_REF}))
            .await
        {
            Ok(response) => response,
            Err(Error::Remote { code, .. }) if code == "SECRET_NOT_FOUND" => return Ok(None),
            Err(_) => return Err(SessionError::storage()),
        };
        let raw = response
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(SessionError::storage)?;
        let value: Value = serde_json::from_str(raw).map_err(|_| SessionError::storage())?;
        // Missing Option fields must not silently become an empty session.
        let session = value.get("session").ok_or_else(SessionError::storage)?;
        if !session.is_null() && session.get("credential").is_none() {
            return Err(SessionError::storage());
        }
        let envelope: Envelope =
            serde_json::from_value(value).map_err(|_| SessionError::storage())?;
        if envelope.version != 1 {
            return Err(SessionError::storage());
        }
        Ok(Some((raw.into(), envelope)))
    }

    async fn swap(
        &self,
        expected: Option<&str>,
        envelope: &Envelope,
    ) -> Result<bool, SessionError> {
        let value = serde_json::to_string(envelope).map_err(|_| SessionError::storage())?;
        let mut payload = json!({"name":SECRET_NAME,"expected":expected,"value":value});
        if expected.is_none() {
            payload["consumers"] = json!([CONSUMER]);
        }
        let response = self
            .call("secrets::compare-and-set", payload)
            .await
            .map_err(|_| SessionError::storage())?;
        response
            .get("swapped")
            .and_then(Value::as_bool)
            .ok_or_else(SessionError::storage)
    }

    /// Caller holds transition. Read back each write before progressing, so a
    /// lost response is safely resolved by the next call or process restart.
    async fn ready(&self) -> Result<(String, Envelope), SessionError> {
        for _ in 0..8 {
            match self.read().await? {
                Some((raw, envelope)) if envelope.migration_complete => return Ok((raw, envelope)),
                Some((raw, mut envelope)) => {
                    if let Some(current) = self.legacy.clear(envelope.session.as_ref()).await? {
                        envelope.session = Some(current);
                        self.swap(Some(&raw), &envelope).await?;
                        continue;
                    }
                    envelope.migration_complete = true;
                    self.swap(Some(&raw), &envelope).await?;
                }
                None => {
                    let envelope = Envelope {
                        version: 1,
                        session: self.legacy.load().await?,
                        migration_complete: false,
                    };
                    self.swap(None, &envelope).await?;
                }
            }
        }
        Err(SessionError::changed())
    }
}

impl SessionStore for SecretsSessionStore {
    fn load(&self) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionError>> {
        Box::pin(async {
            let _transition = self.transition.lock().await;
            Ok(self.ready().await?.1.session)
        })
    }

    fn compare_and_set<'a>(
        &'a self,
        expected: Option<&'a SessionRecord>,
        next: &'a SessionRecord,
    ) -> BoxFuture<'a, Result<bool, SessionError>> {
        Box::pin(async move {
            let _transition = self.transition.lock().await;
            let (raw, envelope) = self.ready().await?;
            if envelope.session.as_ref() != expected {
                return Ok(false);
            }
            self.swap(
                Some(&raw),
                &Envelope {
                    version: 1,
                    session: Some(next.clone()),
                    migration_complete: true,
                },
            )
            .await
        })
    }
}
