//! The `browser::*` surface: scrapling's parse-only functions,
//! natively. `register_all` binds the 19 parse functions and the guidance hook
//! function on the iii bus; `apply_guidance` binds the guidance hook to
//! `harness::hook::pre-generate`.
//!
//! Wire contract: schemas byte-identical to scrapling/src/schemas.py modulo the
//! `browser::` id prefix (golden-pinned); behavior differential-pinned against
//! the frozen Scrapling 0.4.9 worker.
//!
//! Registration deliberately stays separate from `crate::functions`
//! (the 29 `browser::*` functions): that module pins its surface with
//! schemars-derived schemas and self-regenerating goldens, while this one
//! carries Python-mirrored literals checked against read-only goldens. Two
//! catalogs, two invariants, no interleaving.

pub mod adaptive;
mod browserforge;
pub mod cdp;
pub mod crawl;
pub mod dom;
pub mod egress_gate;
pub mod fetch;
pub mod inject_guidance;
pub mod markdown;
pub mod net;
pub mod ops;
pub mod page;
pub mod query;
pub mod raw_browser;
pub mod schemas;
pub mod selgen;
pub mod sessions;
pub mod similar;
pub mod text;
pub mod xpath;

use std::sync::Arc;

use serde_json::Value;

pub(crate) fn require_str<'a>(payload: &'a Value, key: &str) -> Result<&'a str, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("'{key}'"))
}

/// The 19 catalog function ids (in `schemas::catalog()` order) plus the
/// internal guidance hook, last. `register_all` asserts it registers exactly
/// this set, in this order.
pub const STATIC_IDS: &[&str] = &[
    "browser::fetch",
    "browser::stealthy-fetch",
    "browser::dynamic-fetch",
    "browser::screenshot-url",
    "browser::extract",
    "browser::css",
    "browser::xpath",
    "browser::regex",
    "browser::find-similar",
    "browser::find",
    "browser::find-by-text",
    "browser::find-by-regex",
    "browser::describe",
    "browser::to-markdown",
    "browser::session-open",
    "browser::session-fetch",
    "browser::session-close",
    "browser::session-list",
    "browser::crawl",
    inject_guidance::GUIDANCE_HOOK_ID,
];

/// An op's dispatch fn pointer (named alias: clippy::type_complexity on the
/// bare `fn(&Value) -> Result<Value, String>` spelled out at every use site).
type Op = fn(&Value) -> Result<Value, String>;

/// Look up the op fn for a catalog function id. `None` for anything not in
/// the catalog (including the guidance hook, which isn't dispatched here).
pub fn op_for(id: &str) -> Option<Op> {
    Some(match id {
        "browser::extract" => ops::extract::op,
        "browser::css" => ops::css_fn::op,
        "browser::xpath" => ops::xpath_fn::op,
        "browser::regex" => ops::regex_fn::op,
        "browser::find-similar" => ops::find_similar::op,
        "browser::find" => ops::find::op,
        "browser::find-by-text" => ops::find_by_text::op,
        "browser::find-by-regex" => ops::find_by_regex::op,
        "browser::describe" => ops::describe::op,
        "browser::to-markdown" => ops::to_markdown::op,
        _ => return None,
    })
}

pub fn dispatch_op(function_id: &str, payload: &Value) -> Result<Value, String> {
    match op_for(function_id) {
        Some(op) => op(payload),
        None => Err(format!("not implemented: {function_id}")),
    }
}

/// Register all 19 `browser::*` functions plus the internal
/// guidance hook on the iii bus.
///
/// The catalog is served by two handler families and every entry belongs to
/// exactly one: the ten parse ops are sync and stateless
/// (`fn(&Value) -> Result<Value>`, found via `op_for`), while the nine
/// outbound ones are async and need `net::Ctx`. Both take and return untyped
/// `serde_json::Value` — schemars would derive the permissive `AnyValue`
/// schema for that — so the `.request_format`/`.response_format` overrides
/// carrying `schemas::catalog()`'s Python-mirrored literals are mandatory.
/// The guidance hook uses real typed structs and relies on derivation.
pub fn register_all(iii: &Arc<iii_sdk::IIIClient>, ctx: &Arc<net::Ctx>) {
    let mut registered: Vec<&str> = Vec::new();
    for spec in crate::scrapling::schemas::catalog() {
        registered.push(spec.function_id);
        match op_for(spec.function_id) {
            Some(op) => {
                iii.register_function(
                    spec.function_id,
                    iii_sdk::RegisterFunction::new_async(
                        move |payload: serde_json::Value| async move {
                            // Parse ops are synchronous and CPU-bound; some
                            // (regex/find-by-regex on the vendored sre_engine,
                            // which has no backtracking limit) can run for a
                            // long time on adversarial input. Run them off the
                            // async runtime so one call can't peg a runtime
                            // worker and stall the interactive functions and
                            // outbound fetches that share it.
                            tokio::task::spawn_blocking(move || op(&payload))
                                .await
                                .map_err(|e| iii_sdk::Error::from(format!("parse op failed: {e}")))?
                                .map_err(iii_sdk::Error::from)
                        },
                    )
                    .description(spec.description)
                    .request_format(spec.request.clone())
                    .response_format(spec.response.clone()),
                );
            }
            None => {
                let id = spec.function_id;
                let cx = ctx.clone();
                iii.register_function(
                    id,
                    iii_sdk::RegisterFunction::new_async(move |payload: serde_json::Value| {
                        let cx = cx.clone();
                        async move {
                            net::dispatch(&cx, id, &payload)
                                .await
                                .map_err(iii_sdk::Error::from)
                        }
                    })
                    .description(spec.description)
                    .request_format(spec.request.clone())
                    .response_format(spec.response.clone()),
                );
            }
        }
    }
    registered.push(inject_guidance::GUIDANCE_HOOK_ID);
    iii.register_function(
        inject_guidance::GUIDANCE_HOOK_ID,
        iii_sdk::RegisterFunction::new_async(
            |event: inject_guidance::PreGenerateEvent| async move {
                inject_guidance::handle(event).await
            },
        )
        .description(inject_guidance::GUIDANCE_HOOK_DESC)
        .metadata(serde_json::json!({ "internal": true })),
    );
    assert_eq!(
        registered, STATIC_IDS,
        "register_all must register exactly STATIC_IDS"
    );
    tracing::info!("browser::* functions registered");
}

/// The live pre-generate guidance binding, if any. Shared with the
/// configuration change handler so a `browser.scrapling.inject_guidance`
/// flip binds/unbinds at runtime — no worker restart (mirrors
/// fp/src/guidance.rs).
pub type GuidanceState = iii_config_client::BindingSlot;

/// Reconcile the guidance binding with the configured `inject_guidance`
/// value: on → bind once; off → unregister and drop the handle. Idempotent
/// under repeated config events; a failed bind retries on the next event.
/// The hook FUNCTION stays registered either way (`register_all`) — an
/// unbound function is inert, and keeping it registered is what lets a
/// config flip enable injection without a restart.
///
/// `on_error: fail_open` is MANDATORY: pre_generate defaults fail-CLOSED,
/// and a missing guidance line must never abort a turn.
/// `metadata.inject_prompt` carries the guidance statically so the harness
/// appends it without an RPC per generate step; without it every model-call
/// step makes a live call to this worker and blocks generation on the answer
/// (and the guidance vanishes from the frozen-prompt preview).
pub fn apply_guidance(iii: &iii_sdk::IIIClient, state: &GuidanceState, enabled: bool) {
    state.reconcile(
        enabled,
        || {
            iii_config_client::try_bind(
                iii,
                iii_sdk::protocol::RegisterTriggerInput {
                    trigger_type: "harness::hook::pre-generate".to_string(),
                    function_id: inject_guidance::GUIDANCE_HOOK_ID.to_string(),
                    config: serde_json::json!({ "on_error": "fail_open" }),
                    metadata: Some(
                        serde_json::json!({ "inject_prompt": inject_guidance::GUIDANCE }),
                    ),
                },
            )
        },
        "inject_guidance on: appending browser::* scraping guidance to agent system prompts",
        "inject_guidance off: browser::* scraping guidance stays out of agent system prompts",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `register_all` picks a handler family per catalog entry. If an id
    /// belonged to neither (or both), registration would silently ship a
    /// function that always answers "not implemented" — so pin the partition
    /// here rather than discovering it on the bus.
    #[test]
    fn every_catalog_id_has_exactly_one_handler_family() {
        for spec in schemas::catalog() {
            let sync = op_for(spec.function_id).is_some();
            let net = net::NET_IDS.contains(&spec.function_id);
            assert!(
                sync ^ net,
                "{} must be handled by exactly one family (sync parse op: {sync}, net: {net})",
                spec.function_id
            );
        }
    }

    #[test]
    fn static_ids_are_the_catalog_in_order_plus_the_hook() {
        let mut expected: Vec<&str> = schemas::catalog().iter().map(|s| s.function_id).collect();
        expected.push(inject_guidance::GUIDANCE_HOOK_ID);
        assert_eq!(STATIC_IDS, expected.as_slice());
    }

    #[test]
    fn net_ids_covers_every_non_parse_catalog_entry() {
        let catalog_net: Vec<&str> = schemas::catalog()
            .iter()
            .map(|s| s.function_id)
            .filter(|id| op_for(id).is_none())
            .collect();
        assert_eq!(catalog_net.len(), net::NET_IDS.len());
        for id in &catalog_net {
            assert!(net::NET_IDS.contains(id), "{id} missing from NET_IDS");
        }
    }
}
