//! `context-clm::assemble` — the CLM strategy behind the harness's
//! `context::assemble` contract (spec §5.2), and the per-session plumbing it
//! shares with the `session::deleted` cleanup.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::Value;

use crate::config::ClmConfig;
use crate::configuration::ConfigCell;
use crate::store::Store;
use crate::types::{AssembleRequest, AssembleResponse};

pub const ASSEMBLE_ID: &str = "context-clm::assemble";
/// Every step is delegated here for the model limits and the final fit.
pub const CONTEXT_ASSEMBLE_ID: &str = "context::assemble";
/// The spike's ceiling for the delegated hop (a long window is serialized twice).
const DELEGATE_TIMEOUT_MS: u64 = 300_000;

/// The `context::assemble` hop, behind a trait so the flow runs without an engine.
pub trait Delegate: Send + Sync {
    fn assemble(&self, payload: Value) -> impl Future<Output = Result<Value, String>> + Send;
}

/// Production delegate: `context::assemble` over the bus.
pub struct IiiDelegate(pub Arc<IIIClient>);

impl Delegate for IiiDelegate {
    async fn assemble(&self, payload: Value) -> Result<Value, String> {
        self.0
            .trigger(TriggerRequest {
                function_id: CONTEXT_ASSEMBLE_ID.into(),
                payload,
                action: None,
                timeout_ms: Some(DELEGATE_TIMEOUT_MS),
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// One async mutex per session id: steps of one session never interleave.
// NOTE: entries live until session::deleted; add idle eviction if the map ever shows up.
#[derive(Default)]
pub struct Locks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl Locks {
    pub fn get(&self, sid: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(sid.to_owned())
            .or_default()
            .clone()
    }

    pub fn forget(&self, sid: &str) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).remove(sid);
    }
}

pub struct Deps<D> {
    pub delegate: D,
    pub store: Store,
    pub config: ConfigCell<ClmConfig>,
    pub locks: Locks,
}

/// One harness step. Without a filesystem root (or a session id to key state
/// by) the call is context-manager's, unchanged.
pub async fn assemble<D: Delegate>(
    deps: &Deps<D>,
    req: AssembleRequest,
) -> Result<AssembleResponse, Error> {
    forward(&deps.delegate, &req).await
}

async fn forward<D: Delegate>(
    delegate: &D,
    req: &AssembleRequest,
) -> Result<AssembleResponse, Error> {
    let payload = serde_json::to_value(req).map_err(|e| Error::Serde(e.to_string()))?;
    let value = delegate
        .assemble(payload)
        .await
        .map_err(|e| Error::Handler(format!("{CONTEXT_ASSEMBLE_ID}: {e}")))?;
    serde_json::from_value(value)
        .map_err(|e| Error::Serde(format!("{CONTEXT_ASSEMBLE_ID} response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    const SID: &str = "sess/1";

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<Value>>,
    }

    impl Fake {
        fn last(&self) -> Value {
            self.calls
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("delegate called")
        }
    }

    impl Delegate for Fake {
        async fn assemble(&self, payload: Value) -> Result<Value, String> {
            self.calls.lock().unwrap().push(payload.clone());
            Ok(json!({
                "system_prompt": payload["system_prompt"].as_str().unwrap_or_default(),
                "messages": payload["messages"],
                "token_count": 10,
                "usable": 1_000_000,
                "effective_max_output_tokens": 4096,
                "applied": { "compacted": false },
            }))
        }
    }

    fn deps(dir: &Path, cfg: ClmConfig) -> Deps<Fake> {
        Deps {
            delegate: Fake::default(),
            store: Store::new(dir.join("state")),
            config: crate::configuration::new_cell(cfg),
            locks: Locks::default(),
        }
    }

    fn request(raw: &[Value], root: Option<&Path>, step: u64) -> AssembleRequest {
        serde_json::from_value(json!({
            "messages": raw,
            "model": { "id": "claude-opus-5-5", "provider": "anthropic" },
            "system_prompt": "BASE",
            "tools": [],
            "options": {
                "lease_key": SID,
                "session_id": SID,
                "turn_id": "turn-1",
                "step": step,
                "filesystem_root": root.map(|p| p.to_string_lossy().into_owned()),
                "request_overhead_tokens": 0,
            },
        }))
        .unwrap()
    }

    fn user(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 })
    }

    #[tokio::test]
    async fn passthrough_without_filesystem_root() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path(), ClmConfig::default());
        let req = request(&[user("task")], None, 1);
        let expected = serde_json::to_value(&req).unwrap();
        let resp = assemble(&d, req).await.unwrap();
        assert_eq!(d.delegate.last(), expected);
        assert_eq!(resp.token_count, 10);
        assert!(d.store.load(SID).unwrap().is_none());
    }
}
