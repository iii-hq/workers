//! `iii-directory::inject-skills-index` — a `pre_generate` hook that appends
//! the canonical skills index to the agent's system prompt, ONLY while this
//! worker is connected and `inject_index` is enabled in config. Mirrors
//! `fp::inject-guidance` (fp/src/guidance.rs): the binding is requested at
//! worker startup; the engine parks it as a pending intent until the harness
//! registers the `harness::hook::pre-generate` trigger type (recoverable
//! triggers), which also covers a harness restart. Bound `fail_open` so an
//! error or timeout here can never block a turn.
//!
//! The injected text IS `directory::skills::index` — built by the same
//! `skills::build_index` the function handler uses, so overview
//! classification, teaser resolution, and the never-drop-a-worker budget
//! policy cannot drift between the two surfaces. The hook wraps it in a
//! short TTL cache and rebuilds OUTSIDE the cache lock, so concurrent
//! generations are never serialized behind a rebuild; the worker-list
//! filter runs through the stale-tolerant `RegisteredWorkersCache`
//! (`fresh: false`), so a rebuild touches the engine at most once per its
//! TTL.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::config::SharedConfig;
use crate::functions::skills::{self, RegisteredWorkersCache};

pub const INDEX_HOOK_ID: &str = "iii-directory::inject-skills-index";
pub const INDEX_HOOK_DESC: &str =
    "Internal pre_generate hook: appends the directory::skills::index markdown to the agent \
     system prompt when `inject_index` is enabled. Bound to harness::hook::pre-generate at \
     worker startup; not called directly.";

/// The harness-PROVIDED trigger type this worker binds to (NOT an engine
/// built-in).
const PRE_GENERATE_TRIGGER_TYPE: &str = "harness::hook::pre-generate";

/// Rebuild the index at most this often.
const CACHE_TTL: Duration = Duration::from_secs(10);

/// The slice of the `pre_generate` hook envelope we read (lenient: ignores
/// every other field the harness sends).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateEvent {
    #[serde(default)]
    pub generate: GenerateContext,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateContext {
    /// The system prompt assembled so far (base + any prior hook's mutation).
    #[serde(default)]
    pub system_prompt: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreGenerateResponse {
    pub mutations: PreGenerateMutations,
}

/// The harness applies `system_prompt` only when the key is present, so
/// `None` serializes to an empty object: the safe no-op that preserves the
/// harness's assembled prompt.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct PreGenerateMutations {
    /// Full replacement system prompt (base + appended index). The harness
    /// overwrites, it does not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// Returns NO `system_prompt` when `base` is empty (schema drift must never
/// replace the assembled prompt with the index alone) or when there is no
/// index to add. For a real base we append and return the FULL prompt.
fn mutations_for(base: &str, index: Option<&str>) -> PreGenerateMutations {
    match (base.is_empty(), index) {
        (false, Some(index)) => PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{index}")),
        },
        _ => PreGenerateMutations::default(),
    }
}

struct IndexCache {
    built_at: Instant,
    /// `None` when the last build found no worker overviews (nothing to
    /// inject).
    index: Option<Arc<String>>,
}

/// Fresh-cache fast path: `Ok` with the cached index while it is younger
/// than [`CACHE_TTL`], else `Err` (rebuild needed). Extracted so the lock
/// scope stays a single expression.
fn cached_index(cache: &Mutex<Option<IndexCache>>) -> Result<Option<Arc<String>>, ()> {
    let guard = cache.lock().unwrap_or_else(|p| p.into_inner());
    match guard.as_ref() {
        Some(c) if c.built_at.elapsed() <= CACHE_TTL => Ok(c.index.clone()),
        _ => Err(()),
    }
}

/// Register the index hook function and bind it to the harness pre-generate
/// trigger type. The `inject_index` config gate is read on every fire, so
/// toggling it via the configuration worker takes effect without a rebind.
pub fn setup(iii: &Arc<IIIClient>, cfg: &SharedConfig, cache: &Arc<RegisteredWorkersCache>) {
    let cfg = cfg.clone();
    let workers_cache = cache.clone();
    let engine = iii.clone();
    let index_cache: Arc<Mutex<Option<IndexCache>>> = Arc::new(Mutex::new(None));

    iii.register_function(
        INDEX_HOOK_ID,
        RegisterFunction::new_async(move |event: PreGenerateEvent| {
            let cfg = cfg.clone();
            let workers_cache = workers_cache.clone();
            let engine = engine.clone();
            let index_cache = index_cache.clone();
            async move {
                let snapshot = cfg.load_full();
                if !snapshot.inject_index {
                    return Ok::<_, Error>(PreGenerateResponse {
                        mutations: PreGenerateMutations::default(),
                    });
                }
                let index = match cached_index(&index_cache) {
                    Ok(index) => index,
                    Err(()) => {
                        // Rebuild WITHOUT holding the cache lock: concurrent
                        // fires may race into a duplicate build (harmless,
                        // idempotent) but are never serialized behind one.
                        let (body, workers_count) =
                            skills::build_index(&snapshot, &workers_cache, &engine, false).await;
                        let index = (workers_count > 0).then(|| Arc::new(body));
                        let mut guard = index_cache.lock().unwrap_or_else(|p| p.into_inner());
                        *guard = Some(IndexCache {
                            built_at: Instant::now(),
                            index: index.clone(),
                        });
                        index
                    }
                };
                Ok(PreGenerateResponse {
                    mutations: mutations_for(
                        &event.generate.system_prompt,
                        index.as_deref().map(|s| s.as_str()),
                    ),
                })
            }
        })
        .description(INDEX_HOOK_DESC)
        .metadata(json!({ "internal": true })),
    );

    // `on_error: fail_open` is MANDATORY: pre_generate defaults fail-CLOSED,
    // which would abort generation if this hook ever errored or timed out.
    // If the harness is not up yet, the engine parks the binding as a pending
    // intent and activates it when the type registers.
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: PRE_GENERATE_TRIGGER_TYPE.to_string(),
        function_id: INDEX_HOOK_ID.to_string(),
        config: json!({ "on_error": "fail_open" }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(
            trigger_type = PRE_GENERATE_TRIGGER_TYPE,
            function_id = INDEX_HOOK_ID,
            "skills-index hook binding requested"
        ),
        Err(e) => tracing::warn!(
            trigger_type = PRE_GENERATE_TRIGGER_TYPE,
            function_id = INDEX_HOOK_ID,
            error = %e,
            "skills-index hook binding failed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_base_is_preserved_never_replaced() {
        let m = mutations_for("", Some("# Skills index"));
        assert!(m.system_prompt.is_none());
    }

    #[test]
    fn no_index_is_a_noop() {
        let m = mutations_for("BASE", None);
        assert!(m.system_prompt.is_none());
    }

    #[test]
    fn index_appends_after_the_base() {
        let m = mutations_for("BASE", Some("INDEX"));
        assert_eq!(m.system_prompt.as_deref(), Some("BASE\n\nINDEX"));
    }

    #[test]
    fn cache_serves_fresh_and_demands_rebuild_when_stale() {
        let cache = Mutex::new(None);
        assert!(cached_index(&cache).is_err());
        *cache.lock().unwrap() = Some(IndexCache {
            built_at: Instant::now(),
            index: Some(Arc::new("X".into())),
        });
        assert_eq!(
            cached_index(&cache).unwrap().as_deref().map(|s| s.as_str()),
            Some("X")
        );
        *cache.lock().unwrap() = Some(IndexCache {
            built_at: Instant::now() - CACHE_TTL - Duration::from_secs(1),
            index: Some(Arc::new("X".into())),
        });
        assert!(cached_index(&cache).is_err());
    }
}
