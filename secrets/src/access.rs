//! Who may resolve a value.
//!
//! The engine overwrites `_caller_worker_id` on every invocation with the id
//! of the connection that made it, so a caller cannot claim another worker's
//! identity. That id maps to a worker name through `engine::workers::list`
//! (cached; refreshed on a miss, since a new connection means a new id). A
//! caller registered in another namespace than this worker never matches: a
//! second Compose project on the same engine with its own `llm-router` does
//! not get this project's keys.
use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::error::{codes, SecretsError};

const WORKERS_LIST: &str = "engine::workers::list";
const LOOKUP_TIMEOUT_MS: u64 = 5_000;
/// The engine's namespace for workers that declare none.
pub const DEFAULT_NAMESPACE: &str = "default";

/// The authorization rule, nothing else: an empty allowlist admits nobody,
/// an unidentified caller is never admitted, names match exactly.
pub fn is_authorized(consumers: &[String], caller: Option<&str>) -> bool {
    match caller {
        Some(caller) if !caller.is_empty() => consumers.iter().any(|consumer| consumer == caller),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerIdentity {
    pub name: String,
    /// `None` when the engine does not report one.
    pub namespace: Option<String>,
}

/// `{workers: [{id, name, namespace?}]}` as an id → identity map. Entries
/// without an id or a name are skipped.
pub fn parse_workers(list: &Value) -> HashMap<String, WorkerIdentity> {
    list.get("workers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|worker| {
            let id = worker.get("id")?.as_str()?;
            let name = worker.get("name")?.as_str()?.trim();
            if id.is_empty() || name.is_empty() {
                return None;
            }
            let namespace = worker
                .get("namespace")
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some((
                id.to_owned(),
                WorkerIdentity {
                    name: name.to_owned(),
                    namespace,
                },
            ))
        })
        .collect()
}

/// The name a caller is authorized as: its worker name, provided it lives in
/// `own_namespace` (an engine that reports no namespace is trusted as-is).
pub fn authorized_name<'a>(identity: &'a WorkerIdentity, own_namespace: &str) -> Option<&'a str> {
    match identity.namespace.as_deref() {
        Some(namespace) if namespace != own_namespace => None,
        _ => Some(identity.name.as_str()),
    }
}

/// id → identity cache over `engine::workers::list`.
pub struct CallerDirectory {
    iii: Arc<IIIClient>,
    own_namespace: String,
    cache: RwLock<HashMap<String, WorkerIdentity>>,
}

impl CallerDirectory {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        let own_namespace = iii
            .namespace()
            .filter(|namespace| !namespace.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_owned());
        Self {
            iii,
            own_namespace,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// The caller's worker name for authorization, `None` when the engine
    /// stamped no id, the id is not a connected worker, or it belongs to
    /// another namespace.
    pub async fn name_of(
        &self,
        caller_worker_id: Option<&str>,
    ) -> Result<Option<String>, SecretsError> {
        let Some(id) = caller_worker_id.map(str::trim).filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        if let Some(identity) = self.cache.read().await.get(id) {
            return Ok(authorized_name(identity, &self.own_namespace).map(str::to_owned));
        }
        let list = self
            .iii
            .trigger(TriggerRequest {
                function_id: WORKERS_LIST.to_owned(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(LOOKUP_TIMEOUT_MS),
            })
            .await
            .map_err(|_| {
                SecretsError::new(
                    codes::CALLER_LOOKUP_FAILED,
                    "could not identify the calling worker (engine::workers::list failed)",
                )
            })?;
        // Replacing (not merging) the map also forgets disconnected workers.
        let workers = parse_workers(&list);
        let name = workers
            .get(id)
            .and_then(|identity| authorized_name(identity, &self.own_namespace))
            .map(str::to_owned);
        *self.cache.write().await = workers;
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_consumers_admit_nobody() {
        assert!(!is_authorized(&[], Some("llm-router")));
        assert!(!is_authorized(&[], None));
    }

    #[test]
    fn only_listed_identified_callers_are_admitted() {
        let consumers = names(&["llm-router", "judge-typesafe"]);
        assert!(is_authorized(&consumers, Some("llm-router")));
        assert!(is_authorized(&consumers, Some("judge-typesafe")));
        assert!(!is_authorized(&consumers, Some("harness")));
        assert!(!is_authorized(&consumers, Some("LLM-ROUTER")));
        assert!(!is_authorized(&consumers, Some("llm-router ")));
        assert!(!is_authorized(&consumers, Some("")));
        assert!(!is_authorized(&consumers, None));
    }

    #[test]
    fn workers_list_maps_ids_to_names() {
        let list = json!({"workers":[
            {"id":"a1","name":"llm-router","namespace":"default"},
            {"id":"b2","name":"llm-router","namespace":"other-project"},
            {"id":"c3","name":"ade"},
            {"id":"d4"},
            {"name":"orphan"}
        ]});
        let workers = parse_workers(&list);
        assert_eq!(workers.len(), 3);
        assert_eq!(
            authorized_name(&workers["a1"], "default"),
            Some("llm-router")
        );
        // Same name, other namespace: not this project's router.
        assert_eq!(authorized_name(&workers["b2"], "default"), None);
        assert_eq!(
            authorized_name(&workers["b2"], "other-project"),
            Some("llm-router")
        );
        // No namespace reported: trusted as-is.
        assert_eq!(authorized_name(&workers["c3"], "default"), Some("ade"));
        assert!(parse_workers(&json!({})).is_empty());
    }
}
