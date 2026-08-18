//! Provider re-discovery: find live providers from the ENGINE's function
//! registry rather than from the router's own persisted registry, and nudge
//! each one to re-declare.
//!
//! Why this exists. Providers re-declare (and re-push their catalog) when they
//! handle `provider::<id>::on_router_ready`. Two things can stop that from
//! happening, and both leave the router serving an empty catalog while the
//! providers themselves are healthy and connected:
//!
//! 1. The engine drops a trigger binding when the type's owner — this worker —
//!    disconnects, so a provider that outlived a router restart never hears the
//!    `router::ready` fan-out. `register.rs` already compensates by nudging
//!    `registry.ids()` directly, but that list comes from the persisted
//!    registry: on a stack whose state worker runs in-memory it is EMPTY after
//!    a restart, so the nudge reaches nobody.
//! 2. `router::ready` is a one-shot broadcast. A provider that re-binds its
//!    subscription after the router fired it simply misses it.
//!
//! In both cases the provider recovers only on its own periodic catalog timer
//! (three minutes for openai-codex), and until then every model it serves is
//! unresolvable — `router::models::budget` returns null, context-manager takes
//! its conservative 8k fallback, and healthy sessions die with a bewildering
//! `context/overflow`.
//!
//! The engine's function registry is the one source that is always current, so
//! we derive the provider set from it. `provider::<id>::on_router_ready` is the
//! deterministic per-provider handler (the same one the fan-out targets), which
//! makes it both the discovery key and the thing to call.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use std::collections::HashSet;

/// The handler suffix that identifies a provider and receives the nudge.
const READY_HANDLER_SUFFIX: &str = "::on_router_ready";
const PROVIDER_PREFIX: &str = "provider::";
/// A nudge is a provider round-trip (it re-declares and re-fetches its
/// catalog); bound it so one slow provider cannot stall the sweep.
const NUDGE_TIMEOUT_MS: u64 = 10_000;

/// Provider ids with a live `provider::<id>::on_router_ready` registration.
/// Errors and malformed rows yield an empty list — re-discovery is a repair
/// path and must never take the router down.
///
/// `include_internal` is required, not optional: most providers tag their
/// ready handler `metadata.internal = true` (it is invoked by id, never
/// discovered), and the engine's default list hides exactly those. Without
/// the flag this found only the one provider that happens not to tag it —
/// verified live, one of four.
pub async fn live_provider_ids(iii: &IIIClient) -> Vec<String> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({ "prefix": PROVIDER_PREFIX, "include_internal": true }),
            action: None,
            timeout_ms: Some(NUDGE_TIMEOUT_MS),
        })
        .await;
    let Ok(value) = response else {
        return Vec::new();
    };
    let mut ids: Vec<String> = value
        .get("functions")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("function_id").and_then(Value::as_str))
                .filter_map(provider_id_of)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    ids
}

/// The provider id inside `provider::<id>::on_router_ready`, if that is what
/// this function id is. Ids are matched whole — a function that merely starts
/// with the prefix, or one whose id contains the suffix mid-string, is not a
/// ready handler.
fn provider_id_of(function_id: &str) -> Option<&str> {
    let rest = function_id.strip_prefix(PROVIDER_PREFIX)?;
    let id = rest.strip_suffix(READY_HANDLER_SUFFIX)?;
    (!id.is_empty() && !id.contains("::")).then_some(id)
}

fn newly_live_provider_ids(known: &mut HashSet<String>, live: Vec<String>) -> Vec<String> {
    let added = live
        .iter()
        .filter(|id| !known.contains(*id))
        .cloned()
        .collect();
    *known = live.into_iter().collect();
    added
}

async fn nudge_provider_ids(iii: &IIIClient, ids: &[String]) {
    for id in ids {
        let _ = iii
            .trigger(TriggerRequest {
                function_id: format!("{PROVIDER_PREFIX}{id}{READY_HANDLER_SUFFIX}"),
                payload: json!({}),
                action: None,
                timeout_ms: Some(NUDGE_TIMEOUT_MS),
            })
            .await;
    }
}

/// Nudge every live provider to re-declare. Fire-and-forget per provider: a
/// dead one answers `function_not_found`, which is exactly the right answer,
/// and a slow one cannot hold up the rest.
pub async fn nudge_live_providers(iii: &IIIClient) -> usize {
    let ids = live_provider_ids(iii).await;
    nudge_provider_ids(iii, &ids).await;
    ids.len()
}

/// Coalescing handle for the re-discovery sweep. `engine::functions-available`
/// fires for every function registration change, including unrelated console
/// subscriptions. The sweep tracks provider membership and nudges only newly
/// live providers after the burst goes quiet.
#[derive(Clone)]
pub struct SweepHandle {
    tx: tokio::sync::mpsc::Sender<()>,
}

impl SweepHandle {
    /// Mark a sweep as due. Never blocks and never fails: a full channel
    /// already means a sweep is pending, which is the same outcome.
    pub fn request(&self) {
        let _ = self.tx.try_send(());
    }
}

/// Quiet period before a pending sweep runs.
const SWEEP_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(3);

/// Spawn the sweep task and return its handle.
pub fn spawn_debounced_sweep(iii: IIIClient) -> SweepHandle {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let mut known = HashSet::new();
        while rx.recv().await.is_some() {
            // Drain the rest of the burst, extending the quiet period each
            // time another request arrives.
            while tokio::time::timeout(SWEEP_DEBOUNCE, rx.recv())
                .await
                .is_ok_and(|received| received.is_some())
            {}
            let live = live_provider_ids(&iii).await;
            let added = newly_live_provider_ids(&mut known, live);
            let count = added.len();
            nudge_provider_ids(&iii, &added).await;
            tracing::debug!(
                providers = count,
                "registration change: nudged newly live providers to re-declare"
            );
        }
    });
    SweepHandle { tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_is_extracted_only_from_whole_ready_handler_ids() {
        assert_eq!(
            provider_id_of("provider::openai-codex::on_router_ready"),
            Some("openai-codex")
        );
        assert_eq!(
            provider_id_of("provider::anthropic::on_router_ready"),
            Some("anthropic")
        );
        // Other provider functions are not the ready handler.
        assert_eq!(provider_id_of("provider::anthropic::chat"), None);
        assert_eq!(provider_id_of("provider::anthropic::models::refresh"), None);
        // Not a provider function at all.
        assert_eq!(provider_id_of("router::models::get"), None);
        assert_eq!(provider_id_of("harness::send"), None);
        // Degenerate shapes resolve to nothing rather than a bogus id.
        assert_eq!(provider_id_of("provider::::on_router_ready"), None);
        assert_eq!(provider_id_of("provider::on_router_ready"), None);
        // A nested id would make `provider::<id>::on_router_ready` ambiguous.
        assert_eq!(provider_id_of("provider::a::b::on_router_ready"), None);
    }

    #[test]
    fn provider_set_diff_only_returns_new_ids() {
        let mut known = std::collections::HashSet::from(["anthropic".to_string()]);

        assert!(newly_live_provider_ids(&mut known, vec!["anthropic".into()]).is_empty());
        assert_eq!(
            newly_live_provider_ids(&mut known, vec!["anthropic".into(), "openai-codex".into()]),
            vec!["openai-codex"]
        );

        assert!(newly_live_provider_ids(&mut known, vec!["openai-codex".into()]).is_empty());
        assert_eq!(
            newly_live_provider_ids(&mut known, vec!["anthropic".into(), "openai-codex".into()]),
            vec!["anthropic"]
        );
    }
}
