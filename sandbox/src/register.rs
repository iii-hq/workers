//! Registers the caller-facing `sandbox::*` function ids and forwards each
//! invocation to `sandbox::provider::<name>::<leaf>` via `iii.trigger`.
//!
//! Lifecycle floor (`create`, `exec`, `stop`, `list`) is always registered.
//! Optional surfaces (`snapshot`, `expose_port`, `branch`, `fs::read`,
//! `fs::write`) are also registered at the router layer; whether they
//! actually succeed depends on the chosen provider advertising the matching
//! capability in `create`'s `capabilities[]`.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, III};
use sandbox_abi::{ids, AbiError, SCode};
use serde_json::Value;

use crate::config::Config;

const FORWARD_TIMEOUT_MS: u64 = 300_000;

#[derive(Clone)]
pub struct Ctx {
    pub config: Arc<Config>,
}

impl Ctx {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

fn to_iii(err: AbiError) -> IIIError {
    IIIError::Handler(err.to_string())
}

fn resolve_provider(payload: &Value, default_provider: &str) -> String {
    payload
        .get("provider")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map_or_else(|| default_provider.to_string(), ToString::to_string)
}

async fn forward(
    iii: &III,
    provider: &str,
    leaf: &str,
    mut payload: Value,
) -> Result<Value, IIIError> {
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("provider");
    }
    let target = ids::provider(provider, leaf);
    iii.trigger(TriggerRequest {
        function_id: target.clone(),
        payload,
        action: None,
        timeout_ms: Some(FORWARD_TIMEOUT_MS),
    })
    .await
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("not registered") {
            to_iii(AbiError::UnknownProvider(format!(
                "{provider}: no adapter registered for {target}; run `iii worker add sandbox-{provider}`"
            )))
        } else {
            IIIError::Handler(format!("[{}] forward to {target} failed: {msg}", SCode::ProviderUnavailable.as_str()))
        }
    })
}

pub fn register_all(iii: &III, ctx: Ctx) {
    macro_rules! reg {
        ($id:expr, $desc:expr, $leaf:expr) => {{
            let iii_for_handler = iii.clone();
            let ctx = ctx.clone();
            iii.register_function_with(
                RegisterFunctionMessage {
                    id: $id.to_string(),
                    description: Some($desc.to_string()),
                    request_format: None,
                    response_format: None,
                    metadata: None,
                    invocation: None,
                },
                move |payload: Value| {
                    let iii = iii_for_handler.clone();
                    let provider = resolve_provider(&payload, &ctx.config.default_provider);
                    async move { forward(&iii, &provider, $leaf, payload).await }
                },
            );
        }};
    }

    reg!(ids::CREATE, "Boot a sandbox via the `provider` field (default = local).", "create");
    reg!(ids::EXEC, "Exec a command in a sandbox.", "exec");
    reg!(ids::STOP, "Tear down a sandbox. Idempotent.", "stop");
    reg!(ids::LIST, "List live sandboxes plus concurrency status.", "list");
    reg!(ids::SNAPSHOT, "Snapshot a sandbox. Capability-gated.", "snapshot");
    reg!(ids::EXPOSE_PORT, "Return a public URL for a port. Capability-gated.", "expose_port");
    reg!(ids::BRANCH, "Fan out N siblings from a sandbox. Capability-gated.", "branch");
    reg!(ids::FS_READ, "Read a file from a sandbox. Capability-gated.", "fs::read");
    reg!(ids::FS_WRITE, "Write a file into a sandbox. Capability-gated.", "fs::write");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_defaults_when_absent() {
        let cfg = Config::default();
        assert_eq!(resolve_provider(&json!({}), &cfg.default_provider), "local");
    }

    #[test]
    fn provider_defaults_when_empty_string() {
        let cfg = Config::default();
        assert_eq!(resolve_provider(&json!({"provider": ""}), &cfg.default_provider), "local");
    }

    #[test]
    fn provider_explicit_overrides_default() {
        let cfg = Config::default();
        assert_eq!(resolve_provider(&json!({"provider": "e2b"}), &cfg.default_provider), "e2b");
    }

    #[test]
    fn config_default_provider_can_be_overridden() {
        let cfg = Config { default_provider: "morph".into() };
        assert_eq!(resolve_provider(&json!({}), &cfg.default_provider), "morph");
    }
}
