//! Integration coverage for the RoutingRequest wire envelope.
//!
//! These tests live alongside the unit tests in `src/router.rs` but exercise
//! the same serde boundary that real `iii.trigger` callers hit. Anything that
//! reaches `router::decide` over the bus carries iii-sdk-injected metadata
//! (today: `_caller_worker_id`), and operators occasionally add their own
//! routing hints (`tenant`, `feature`, `tags`) that we want to silently
//! tolerate. A regression here is silent — the function-level integration
//! test caught it the hard way.

use serde_json::json;

#[path = "../src/types.rs"]
#[allow(dead_code, clippy::all)]
mod types;

use types::RoutingRequest;

/// iii-sdk injects `_caller_worker_id` into every trigger payload. This test
/// pins the contract: `RoutingRequest` must accept and ignore that field.
/// Without it, every `iii.trigger("router::decide", ...)` call returns
/// `unknown field _caller_worker_id`, which silently breaks every consumer.
#[test]
fn accepts_iii_sdk_caller_worker_id() {
    let raw = json!({
        "_caller_worker_id": "worker-uuid-from-sdk",
        "prompt": "hello",
    });
    let parsed: RoutingRequest =
        serde_json::from_value(raw).expect("RoutingRequest must accept iii-sdk caller metadata");
    assert_eq!(parsed.prompt, "hello");
    assert!(parsed.tenant.is_none());
}

/// Harness-shaped routing hints round-trip cleanly when paired with
/// caller-injected metadata.
#[test]
fn accepts_full_harness_payload() {
    let raw = json!({
        "_caller_worker_id": "harness-runtime",
        "prompt": "summarise the workspace",
        "tenant": "acme",
        "feature": "code-review",
        "user": "u-42",
        "tags": ["coding", "long-context"],
        "budget_remaining_usd": 0.5,
        "latency_slo_ms": 2000,
        "min_quality": "high",
        "classifier_id": "default",
    });
    let parsed: RoutingRequest =
        serde_json::from_value(raw).expect("full harness payload deserialises");
    assert_eq!(parsed.tenant.as_deref(), Some("acme"));
    assert_eq!(parsed.feature.as_deref(), Some("code-review"));
    assert_eq!(parsed.tags.as_ref().map(|t| t.len()), Some(2));
    assert_eq!(parsed.classifier_id.as_deref(), Some("default"));
}

/// Empty payloads still deserialise — `prompt` is required so this should
/// fail explicitly rather than crash. Pins the contract that absent fields
/// produce a deserialise error, not a silent `String::default()`.
#[test]
fn rejects_payload_without_prompt() {
    let raw = json!({ "_caller_worker_id": "x" });
    let result: Result<RoutingRequest, _> = serde_json::from_value(raw);
    assert!(
        result.is_err(),
        "RoutingRequest with no prompt must fail to deserialise"
    );
}
