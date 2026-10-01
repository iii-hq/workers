use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use iii_sdk::RegisterFunction;
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::sessions::SessionManager;

type Handler = dyn Fn(Arc<SessionManager>, Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>
    + Send
    + Sync;

#[derive(Clone, Debug, Serialize)]
pub struct FunctionInfo {
    pub id: String,
    pub description: String,
    pub request_format: Value,
    pub response_format: Value,
}

struct Entry {
    info: FunctionInfo,
    handler: Arc<Handler>,
}

#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<String, Entry>,
}

impl Registry {
    pub fn add<Req, Resp, F, Fut>(&mut self, id: &str, description: &str, handler: F)
    where
        Req: DeserializeOwned + JsonSchema + Send + 'static,
        Resp: Serialize + JsonSchema + Send + 'static,
        F: Fn(Arc<SessionManager>, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp>> + Send + 'static,
    {
        assert!(
            id.starts_with("whatsapp::"),
            "function id must start with whatsapp::"
        );
        assert!(!self.entries.contains_key(id), "duplicate function id {id}");
        let handler = Arc::new(handler);
        self.entries.insert(
            id.to_owned(),
            Entry {
                info: FunctionInfo {
                    id: id.to_owned(),
                    description: description.to_owned(),
                    request_format: serde_json::to_value(schemars::schema_for!(Req))
                        .expect("request schema"),
                    response_format: serde_json::to_value(schemars::schema_for!(Resp))
                        .expect("response schema"),
                },
                handler: Arc::new(move |manager, mut input| {
                    let handler = handler.clone();
                    Box::pin(async move {
                        if let Some(fields) = input.as_object_mut() {
                            fields.remove("_caller_worker_id");
                        }
                        let request =
                            serde_json::from_value(input).context("invalid function request")?;
                        serde_json::to_value(handler(manager, request).await?)
                            .context("encoding function response")
                    })
                }),
            },
        );
    }

    pub fn catalog(&self) -> Vec<FunctionInfo> {
        self.entries
            .values()
            .map(|entry| entry.info.clone())
            .collect()
    }

    pub async fn invoke(
        &self,
        id: &str,
        manager: Arc<SessionManager>,
        input: Value,
    ) -> Result<Value> {
        let Some(entry) = self.entries.get(id) else {
            bail!("unknown function: {id}");
        };
        // Lifecycle operations own their cancellation and shutdown limits.
        if id.starts_with("whatsapp::sessions::") {
            return (entry.handler)(manager, input).await;
        }
        let timeout = Duration::from_secs(manager.config().operation_timeout_secs);
        tokio::time::timeout(timeout, (entry.handler)(manager, input))
            .await.context("operation timed out; a remote mutation may have completed; check state before retrying")?
    }

    pub fn install(self: Arc<Self>, iii: &iii_sdk::IIIClient, manager: Arc<SessionManager>) {
        for info in self.catalog() {
            let registry = self.clone();
            let manager = manager.clone();
            let id = info.id.clone();
            iii.register_function(
                &info.id,
                RegisterFunction::new_async(move |input: Value| {
                    let registry = registry.clone();
                    let manager = manager.clone();
                    let id = id.clone();
                    async move {
                        registry
                            .invoke(&id, manager, input)
                            .await
                            .map_err(|error| iii_sdk::Error::Handler(format!("{error:#}")))
                    }
                })
                .description(info.description)
                .request_format(info.request_format)
                .response_format(info.response_format),
            );
        }
    }
}
