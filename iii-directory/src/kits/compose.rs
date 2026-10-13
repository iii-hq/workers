//! Workers through Compose: what the project already declares (read from
//! `worker-compose.yaml` and `worker-compose.lock` on disk, so planning works
//! even while the daemon is down) and the `compose::*` calls an apply makes
//! (`add`, `remove`, polled through `compose::operation`).
//!
//! The control functions are registered by the Compose daemon only while
//! `iii compose --up` runs, in the daemon's namespace: like the existing
//! `compose::status` call (`functions::skills::compose_status_request`), the
//! request is routed to `III_COMPOSE_NAMESPACE` and names `III_COMPOSE_FILE`
//! when Compose started this worker.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

/// A worker the compose file declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredWorker {
    /// Container key.
    pub container: String,
    /// Registry package name (the last segment of the reference), or the
    /// container key for a path worker.
    pub package: String,
    /// `version:` as written (exact, tag or range); `None` when unpinned.
    pub selector: Option<String>,
    /// Version Compose resolved and locked, when known.
    pub resolved: Option<String>,
    /// A `path://` worker: the operator's local build, never managed by kits.
    pub path: bool,
}

/// Declared workers keyed by package name.
pub fn declared_workers(compose_file: &Path) -> BTreeMap<String, DeclaredWorker> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(compose_file) else {
        return out;
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return out;
    };
    let lock = read_compose_lock(&compose_file.with_extension("lock"));
    let Some(containers) = doc.get("containers").and_then(|c| c.as_mapping()) else {
        return out;
    };
    for (key, container) in containers {
        let Some(key) = key.as_str() else { continue };
        let worker = container
            .get("worker")
            .and_then(|w| w.as_str())
            .unwrap_or(key);
        let selector = container.get("version").and_then(|v| match v {
            serde_yaml::Value::String(s) => Some(s.clone()),
            serde_yaml::Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        let (package, path) = parse_worker_source(worker, key);
        let resolved = lock.get(key).cloned().or_else(|| {
            selector
                .as_deref()
                .filter(|s| semver::Version::parse(s).is_ok())
                .map(str::to_string)
        });
        out.insert(
            package.clone(),
            DeclaredWorker {
                container: key.to_string(),
                package,
                selector,
                resolved,
                path,
            },
        );
    }
    out
}

/// `(package name, is_path)` for a compose `worker:` value.
fn parse_worker_source(worker: &str, key: &str) -> (String, bool) {
    let worker = worker.trim();
    if worker.starts_with("path://") || worker.starts_with('.') || worker.starts_with('/') {
        return (key.to_string(), true);
    }
    let reference = worker.strip_prefix("package://").unwrap_or(worker);
    let name = reference.rsplit_once('@').map_or(reference, |(n, _)| n);
    let package = name.rsplit('/').next().unwrap_or(name);
    (package.to_string(), false)
}

/// Container key → resolved version, from `worker-compose.lock`.
fn read_compose_lock(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return out;
    };
    if let Some(containers) = doc.get("containers").and_then(|c| c.as_mapping()) {
        for (key, entry) in containers {
            let version = entry
                .get("resolved")
                .and_then(|r| r.get("version"))
                .and_then(|v| v.as_str());
            if let (Some(key), Some(version)) = (key.as_str(), version) {
                out.insert(key.to_string(), version.to_string());
            }
        }
    }
    out
}

/// Terminal state of a Compose operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationOutcome {
    Succeeded(String),
    Failed(String),
}

/// The `compose::*` calls kit flows make. [`EngineCompose`] is the real one.
#[async_trait]
pub trait ComposeControl: Send + Sync {
    /// `Ok` when the Compose daemon answers `compose::status`.
    async fn available(&self) -> Result<(), String>;
    /// `compose::add` with container objects; returns the operation id.
    async fn add(&self, workers: Vec<Value>, operation_id: &str) -> Result<String, String>;
    /// `compose::remove` by container key; returns the operation id.
    async fn remove(&self, containers: Vec<String>, operation_id: &str) -> Result<String, String>;
    /// `compose::operation` snapshot: `(status, last detail)`.
    async fn operation(&self, operation_id: &str) -> Result<(String, String), String>;
}

/// Poll `compose::operation` until the operation leaves `running`.
pub async fn wait_operation(
    compose: &dyn ComposeControl,
    operation_id: &str,
    poll: Duration,
    deadline: Duration,
    mut on_progress: impl FnMut(&str) + Send,
) -> Result<OperationOutcome, String> {
    let started = std::time::Instant::now();
    let mut last = String::new();
    let mut misses = 0;
    loop {
        match compose.operation(operation_id).await {
            Ok((status, detail)) => {
                misses = 0;
                if detail != last && !detail.is_empty() {
                    on_progress(&detail);
                    last = detail.clone();
                }
                match status.as_str() {
                    "succeeded" => return Ok(OperationOutcome::Succeeded(detail)),
                    "failed" | "cancelled" => return Ok(OperationOutcome::Failed(detail)),
                    _ => {}
                }
            }
            // The operation may not be visible for a moment after acceptance.
            Err(e) => {
                misses += 1;
                if misses > 10 {
                    return Err(e);
                }
            }
        }
        if started.elapsed() > deadline {
            return Err(format!(
                "D515 compose_error: compose operation {operation_id} did not finish within {}s; \
                 check compose::operation and retry the apply.",
                deadline.as_secs()
            ));
        }
        tokio::time::sleep(poll).await;
    }
}

/// `compose::*` over the engine, routed like `compose::status`.
pub struct EngineCompose {
    iii: Arc<IIIClient>,
}

impl EngineCompose {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }

    async fn call(
        &self,
        function: &str,
        mut payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, String> {
        let nonempty = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let namespace = nonempty("III_COMPOSE_NAMESPACE");
        if let Some(file) = nonempty("III_COMPOSE_FILE") {
            payload["file"] = json!(file);
        }
        if let Some(ns) = &namespace {
            payload["namespace"] = json!(ns);
        }
        let request = TriggerRequest {
            function_id: function.to_string(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        };
        let result = match namespace {
            Some(ns) => self.iii.trigger(request.namespace(ns)).await,
            None => self.iii.trigger(request).await,
        };
        result.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl ComposeControl for EngineCompose {
    async fn available(&self) -> Result<(), String> {
        self.call("compose::status", json!({}), 5_000)
            .await
            .map(|_| ())
    }

    async fn add(&self, workers: Vec<Value>, operation_id: &str) -> Result<String, String> {
        let out = self
            .call(
                "compose::add",
                json!({ "workers": workers, "operation_id": operation_id }),
                60_000,
            )
            .await?;
        Ok(out
            .get("operation_id")
            .and_then(Value::as_str)
            .unwrap_or(operation_id)
            .to_string())
    }

    async fn remove(&self, containers: Vec<String>, operation_id: &str) -> Result<String, String> {
        let out = self
            .call(
                "compose::remove",
                json!({ "workers": containers, "operation_id": operation_id }),
                60_000,
            )
            .await?;
        Ok(out
            .get("operation_id")
            .and_then(Value::as_str)
            .unwrap_or(operation_id)
            .to_string())
    }

    async fn operation(&self, operation_id: &str) -> Result<(String, String), String> {
        let out = self
            .call(
                "compose::operation",
                json!({ "operation_id": operation_id }),
                10_000,
            )
            .await?;
        let status = out
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("running")
            .to_string();
        let detail = out
            .get("last_event")
            .and_then(|e| e.get("detail"))
            .and_then(Value::as_str)
            .or_else(|| out.get("phase").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        Ok((status, detail))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_workers_read_selectors_and_locked_versions() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("worker-compose.yaml");
        std::fs::write(
            &file,
            "containers:\n  kanban:\n    worker: kanban\n    version: ^1.4\n  \
             db:\n    worker: package://api.workers.iii.dev/state\n    version: 0.22.9\n  \
             iii-directory:\n    worker: path://../iii-directory\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("worker-compose.lock"),
            "version: 1\ncontainers:\n  kanban:\n    worker: package://api.workers.iii.dev/kanban\n    \
             requested: ^1.4\n    resolved:\n      name: kanban\n      version: 1.6.1\n",
        )
        .unwrap();
        let declared = declared_workers(&file);
        let kanban = &declared["kanban"];
        assert_eq!(kanban.selector.as_deref(), Some("^1.4"));
        assert_eq!(kanban.resolved.as_deref(), Some("1.6.1"));
        let state = &declared["state"];
        assert_eq!(state.container, "db");
        assert_eq!(state.resolved.as_deref(), Some("0.22.9"));
        assert!(declared["iii-directory"].path);
    }

    #[test]
    fn missing_compose_file_declares_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(declared_workers(&tmp.path().join("worker-compose.yaml")).is_empty());
    }
}
