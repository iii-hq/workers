pub mod context;
pub mod functions_mod;
pub mod registry;
pub mod stream;
pub mod workers;

/// Namespace prefixes filtered out of `introspection::functions::list` by
/// default. Each one is noise for an agent deciding what to call — they
/// belong to telemetry, internal probes, or skill/resource plumbing the
/// agent has no reason to invoke directly.
pub const DEFAULT_EXCLUDED_NAMESPACES: &[&str] = &[
    "skills::resources-",
    "skills::register",
    "skills::unregister",
    "skills::fetch_skill",
    "skill::register",
    "skill::unregister",
    "engine::workers::register",
    "engine::console::",
    "engine::telemetry::",
    "telemetry::",
    "iii-telemetry::",
    "iii-observability::",
    "hook-fanout::",
    "policy-denylist::",
    "auth::",
    "introspection::stream::",
];

/// Known engine-builtin workers (NOT separate iii-sdk worker processes).
/// They show up in `engine::workers::list` as `available` until the engine
/// config enables them via a config block.
pub const ENGINE_BUILTINS: &[(&str, &str)] = &[
    (
        "iii-sandbox",
        "Engine builtin. Enable by adding `iii-sandbox: { runtime: libkrun | docker, image: python, idle_timeout_secs: 300 }` to your engine config.yaml, then restart engine with `--config <path>` instead of `--use-default-config`.",
    ),
    (
        "iii-http",
        "Engine builtin HTTP trigger surface. Enabled via `iii-http:` config block.",
    ),
    (
        "iii-cron",
        "Engine builtin cron trigger surface. Enabled via `iii-cron:` config block.",
    ),
    (
        "iii-pubsub",
        "Engine builtin pub/sub topic surface. Always available.",
    ),
    (
        "iii-state",
        "Engine builtin key-value state. Always available.",
    ),
    (
        "iii-stream",
        "Engine builtin append-only streams. Always available.",
    ),
    (
        "iii-queue",
        "Engine builtin durable queues. Always available.",
    ),
    (
        "iii-engine-functions",
        "Engine builtin introspection (engine::functions::list etc).",
    ),
    (
        "iii-worker-manager",
        "Engine builtin worker lifecycle. `iii worker add` routes through here.",
    ),
    (
        "iii-console",
        "Engine builtin web console + queue publishers.",
    ),
];

use std::sync::Arc;

use iii_sdk::{protocol::TriggerRequest, IIIError, III};
use serde_json::Value;

pub async fn call(iii: &Arc<III>, function_id: &str, payload: Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

pub fn is_excluded(fn_id: &str, extra: &[String]) -> bool {
    DEFAULT_EXCLUDED_NAMESPACES
        .iter()
        .any(|p| fn_id.starts_with(p))
        || extra.iter().any(|p| fn_id.starts_with(p))
}

pub fn builtin_hint(worker_name: &str) -> Option<&'static str> {
    ENGINE_BUILTINS
        .iter()
        .find(|(n, _)| *n == worker_name)
        .map(|(_, h)| *h)
}
