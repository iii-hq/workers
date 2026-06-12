//! `approval::on_config_change` — bound to the engine's `configuration`
//! trigger on `configuration_id: "approval-gate"`. Reactive reload, no
//! polling: swaps the in-memory deployment defaults.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::Deps;
use crate::error::ApprovalError;
use crate::gate_config::{parse_config_value, replace, ENTRY_ID};

/// The configuration trigger payload (only the fields we read; the
/// engine owns the shape).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConfigChangeEvent {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub new_value: Value,
}

pub async fn handle(deps: &Deps, event: ConfigChangeEvent) -> Result<Value, ApprovalError> {
    if event.id.as_deref() != Some(ENTRY_ID) {
        return Ok(Value::Null);
    }
    let next = parse_config_value(&event.new_value);
    tracing::info!(?next, "approval-gate configuration reloaded");
    replace(&deps.defaults, next);
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::{shared_defaults, snapshot};
    use crate::testkit::FakeBus;
    use crate::types::PermissionMode;

    fn deps() -> Arc<Deps> {
        Arc::new(Deps {
            bus: Arc::new(FakeBus::new()),
            sink: Arc::new(RecordingSink::new()),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        })
    }

    #[tokio::test]
    async fn reloads_defaults_on_matching_entry() {
        let deps = deps();
        let event: ConfigChangeEvent = serde_json::from_value(json!({
            "id": "approval-gate",
            "event_type": "configuration:updated",
            "new_value": { "default_mode": "auto", "pending_timeout_ms": 60000 }
        }))
        .unwrap();
        handle(&deps, event).await.unwrap();
        let d = snapshot(&deps.defaults);
        assert_eq!(d.default_mode, PermissionMode::Auto);
        assert_eq!(d.pending_timeout_ms, 60_000);
    }

    #[tokio::test]
    async fn ignores_other_entries() {
        let deps = deps();
        let event: ConfigChangeEvent = serde_json::from_value(json!({
            "id": "llm-router",
            "new_value": { "default_mode": "full" }
        }))
        .unwrap();
        handle(&deps, event).await.unwrap();
        assert_eq!(
            snapshot(&deps.defaults).default_mode,
            PermissionMode::Manual
        );
    }
}
