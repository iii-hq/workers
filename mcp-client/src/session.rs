use anyhow::{anyhow, Result};
use dashmap::DashMap;
use iii_sdk::FunctionRef;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use crate::transport::Transport;
use crate::types::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, JsonRpcNotification,
    JsonRpcRequest, JsonRpcResponse, ListPromptsResult, ListResourcesResult, ListToolsResult,
    McpPrompt, McpResource, McpTool, ServerCapabilities,
};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub enum SessionSpec {
    Stdio {
        name: String,
        bin: String,
        args: Vec<String>,
    },
    Http {
        name: String,
        url: String,
    },
}

impl SessionSpec {
    pub fn parse(spec: &str) -> Result<SessionSpec> {
        let mut parts = spec.splitn(3, ':');
        let kind = parts
            .next()
            .ok_or_else(|| anyhow!("connect spec missing kind"))?;
        let name = parts
            .next()
            .ok_or_else(|| anyhow!("connect spec missing name"))?
            .to_string();
        let rest = parts
            .next()
            .ok_or_else(|| anyhow!("connect spec missing target"))?;

        match kind {
            "stdio" => {
                let mut tokens = rest.split(':');
                let bin = tokens
                    .next()
                    .ok_or_else(|| anyhow!("stdio spec missing binary"))?
                    .to_string();
                let args: Vec<String> = tokens.map(|s| s.to_string()).collect();
                Ok(SessionSpec::Stdio { name, bin, args })
            }
            "http" => Ok(SessionSpec::Http {
                name,
                url: rest.to_string(),
            }),
            other => Err(anyhow!("unknown connect kind: {}", other)),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            SessionSpec::Stdio { name, .. } => name,
            SessionSpec::Http { name, .. } => name,
        }
    }
}

pub struct Session {
    pub name: String,
    pub transport: Arc<Transport>,
    pub pending: DashMap<u64, oneshot::Sender<JsonRpcResponse>>,
    pub next_id: AtomicU64,
    pub capabilities: RwLock<Option<ServerCapabilities>>,
    pub registered: Mutex<HashMap<String, FunctionRef>>,
    pub notifications: Mutex<Option<mpsc::Receiver<JsonRpcNotification>>>,
    /// Serialises concurrent `reconcile()` passes against the same session
    /// so two simultaneous `tools/list_changed` listeners can't interleave
    /// the unregister/re-register window and corrupt `registered`.
    pub reconcile_lock: Arc<Mutex<()>>,
    notif_tx: mpsc::Sender<JsonRpcNotification>,
}

impl Session {
    fn new(name: String, transport: Arc<Transport>) -> (Arc<Session>, mpsc::Receiver<String>) {
        let reader = transport
            .take_reader()
            .expect("transport reader was already consumed");

        let (notif_tx, notif_rx) = mpsc::channel::<JsonRpcNotification>(32);
        let session = Arc::new(Session {
            name,
            transport,
            pending: DashMap::new(),
            next_id: AtomicU64::new(1),
            capabilities: RwLock::new(None),
            registered: Mutex::new(HashMap::new()),
            notifications: Mutex::new(Some(notif_rx)),
            reconcile_lock: Arc::new(Mutex::new(())),
            notif_tx,
        });
        (session, reader)
    }

    pub async fn connect(spec: SessionSpec) -> Result<Arc<Session>> {
        let (transport, name) = match spec.clone() {
            SessionSpec::Stdio { name, bin, args } => {
                let t = crate::transport::StdioTransport::spawn(&bin, &args).await?;
                (Arc::new(Transport::Stdio(t)), name)
            }
            SessionSpec::Http { name, url } => {
                let t = crate::transport::HttpTransport::new(url);
                (Arc::new(Transport::Http(t)), name)
            }
        };

        Session::start(name, transport).await
    }

    #[doc(hidden)]
    pub async fn connect_with_transport(
        name: impl Into<String>,
        transport: Arc<Transport>,
    ) -> Result<Arc<Session>> {
        Session::start(name.into(), transport).await
    }

    async fn start(name: String, transport: Arc<Transport>) -> Result<Arc<Session>> {
        let (session, reader) = Session::new(name, transport);
        Session::spawn_reader(session.clone(), reader);

        let init: InitializeResult = session
            .call_typed(
                "initialize",
                Some(serde_json::to_value(InitializeParams {
                    protocol_version: PROTOCOL_VERSION.to_string(),
                    capabilities: ClientCapabilities::default(),
                    client_info: ClientInfo {
                        name: "iii-mcp-client".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                })?),
            )
            .await?;

        *session.capabilities.write().await = Some(init.capabilities.clone());

        session
            .send_notification("notifications/initialized", None)
            .await?;

        Ok(session)
    }

    pub fn transport_kind(&self) -> &'static str {
        self.transport.kind()
    }

    fn spawn_reader(session: Arc<Session>, mut reader: mpsc::Receiver<String>) {
        tokio::spawn(async move {
            while let Some(line) = reader.recv().await {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let value: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error = %e, raw = %line, "failed to parse JSON-RPC frame");
                        continue;
                    }
                };

                if value.get("id").is_some() && value.get("method").is_none() {
                    match serde_json::from_value::<JsonRpcResponse>(value) {
                        Ok(resp) => {
                            if let Some((_, sender)) = session.pending.remove(&resp.id) {
                                let _ = sender.send(resp);
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "invalid JSON-RPC response"),
                    }
                } else if value.get("method").is_some() && value.get("id").is_none() {
                    match serde_json::from_value::<JsonRpcNotification>(value) {
                        Ok(n) => {
                            let _ = session.notif_tx.send(n).await;
                        }
                        Err(e) => tracing::warn!(error = %e, "invalid JSON-RPC notification"),
                    }
                } else {
                    tracing::debug!(raw = %line, "ignoring non-response/notification frame");
                }
            }
            tracing::info!(session = %session.name, "transport reader closed");
            // Drain pending callers so they fail fast rather than wait the
            // full call timeout. Senders dropped here cause Session::call's
            // oneshot::Receiver to error with `RecvError`, which we map to
            // a clear "transport closed" message.
            let drained: Vec<u64> = session.pending.iter().map(|e| *e.key()).collect();
            for id in drained {
                if let Some((_, sender)) = session.pending.remove(&id) {
                    drop(sender);
                }
            }
        });
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&notif)?;
        self.transport.send_raw(&line).await
    }

    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);

        let line = serde_json::to_string(&req)?;
        if let Err(e) = self.transport.send_raw(&line).await {
            self.pending.remove(&id);
            return Err(e);
        }

        let resp = match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => {
                return Err(anyhow!(
                    "transport closed while awaiting response to {}",
                    method
                ));
            }
            Err(_) => {
                self.pending.remove(&id);
                return Err(anyhow!("timeout waiting for response to {}", method));
            }
        };

        if let Some(err) = resp.error {
            return Err(anyhow!("JSON-RPC error {}: {}", err.code, err.message));
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    async fn call_typed<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T> {
        let value = self.call(method, params).await?;
        serde_json::from_value(value).map_err(|e| anyhow!("decode {}: {}", method, e))
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let result: ListToolsResult = self.call_typed("tools/list", None).await?;
        Ok(result.tools)
    }

    pub async fn list_resources(&self) -> Result<Vec<McpResource>> {
        let result: ListResourcesResult = self.call_typed("resources/list", None).await?;
        Ok(result.resources)
    }

    pub async fn list_prompts(&self) -> Result<Vec<McpPrompt>> {
        let result: ListPromptsResult = self.call_typed("prompts/list", None).await?;
        Ok(result.prompts)
    }

    pub async fn tools_call(&self, name: &str, arguments: Value) -> Result<Value> {
        self.call(
            "tools/call",
            Some(json!({ "name": name, "arguments": arguments })),
        )
        .await
    }

    pub async fn resources_read(&self, uri: &str) -> Result<Value> {
        self.call("resources/read", Some(json!({ "uri": uri })))
            .await
    }

    pub async fn prompts_get(&self, name: &str, arguments: Value) -> Result<Value> {
        self.call(
            "prompts/get",
            Some(json!({ "name": name, "arguments": arguments })),
        )
        .await
    }
}
