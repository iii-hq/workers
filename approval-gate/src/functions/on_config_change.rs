//! `approval::on-config-change` — bound to the engine's `configuration`
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
    use super::*;
    use crate::gate_config::snapshot;
    use crate::testkit::{with_stack, BootOpts};
    use crate::types::PermissionMode;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread")]
    async fn reloads_defaults_on_matching_entry() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let event: ConfigChangeEvent = serde_json::from_value(json!({
                "id": "approval-gate",
                "event_type": "configuration:updated",
                "new_value": { "default_mode": "auto", "pending_timeout_ms": 60000 }
            }))
            .unwrap();
            handle(&stack.deps, event).await.unwrap();
            let d = snapshot(&stack.defaults);
            assert_eq!(d.default_mode, PermissionMode::Auto);
            assert_eq!(d.pending_timeout_ms, 60_000);
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ignores_other_entries() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let event: ConfigChangeEvent = serde_json::from_value(json!({
                "id": "llm-router",
                "new_value": { "default_mode": "full" }
            }))
            .unwrap();
            handle(&stack.deps, event).await.unwrap();
            assert_eq!(
                snapshot(&stack.defaults).default_mode,
                PermissionMode::Manual
            );
        })
        .await;
    }
}
