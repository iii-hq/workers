//! `iii-directory::inject-skills-index` — a `pre_generate` hook that appends a
//! compact per-worker skills index to the agent's system prompt, ONLY while
//! this worker is connected and `inject_index` is enabled in config. Mirrors
//! `fp::inject-guidance` (fp/src/guidance.rs): the binding is requested at
//! worker startup; the engine parks it as a pending intent until the harness
//! registers the `harness::hook::pre-generate` trigger type (recoverable
//! triggers), which also covers a harness restart. Bound `fail_open` so an
//! error or timeout here can never block a turn.
//!
//! The index is built from the on-disk skill set (`scan_skills_merged`), one
//! line per worker overview (single-segment skill id), teaser from the
//! frontmatter `description`. Engine-side install filtering is deliberately
//! skipped: the hook runs in the generation critical path, so it reads disk
//! through a short-lived cache and never calls back into the engine.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::config::SharedConfig;
use crate::fs_source;

pub const INDEX_HOOK_ID: &str = "iii-directory::inject-skills-index";
pub const INDEX_HOOK_DESC: &str =
    "Internal pre_generate hook: appends a compact installed-skills index to the agent \
     system prompt when `inject_index` is enabled. Bound to harness::hook::pre-generate \
     at worker startup; not called directly.";

/// The harness-PROVIDED trigger type this worker binds to (NOT an engine
/// built-in).
const PRE_GENERATE_TRIGGER_TYPE: &str = "harness::hook::pre-generate";

/// Rebuild the index from disk at most this often.
const CACHE_TTL: Duration = Duration::from_secs(10);

/// Keep the injected section token-light: teasers are clipped per line and
/// the whole section is capped, pointing at `directory::skills::list` for the
/// rest.
const TEASER_MAX_CHARS: usize = 140;
const INDEX_MAX_CHARS: usize = 2500;

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

/// One index row: worker overview id and its teaser text.
type Entry = (String, String);

/// Scan the merged skill folders and keep one row per worker overview: a
/// single-segment skill id whose file parses. Teaser precedence matches the
/// list surface: frontmatter `description`, else empty.
fn collect_entries(cfg: &crate::config::SkillsConfig) -> Vec<Entry> {
    let (skills, _skips) =
        fs_source::scan_skills_merged(&cfg.resolved_skills_folder(), &cfg.local_skills_folder());
    let mut entries: Vec<Entry> = skills
        .iter()
        .filter(|s| !s.id.contains('/'))
        .filter_map(|s| {
            let (fm, _body) = fs_source::read_skill_with_frontmatter(&s.abs_path).ok()?;
            let teaser = fm.description.unwrap_or_default();
            Some((s.id.clone(), teaser))
        })
        .collect();
    entries.sort();
    entries
}

/// Render the index section, or `None` when there is nothing to inject.
fn format_index(entries: &[Entry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Installed skill docs\n\nWorkers on this engine ship skill docs. Read one with \
         `directory::skills::get { id: \"<worker>\" }`; deeper docs are linked inside. \
         Full list: `directory::skills::list`.\n\n",
    );
    for (id, teaser) in entries {
        let teaser: String = teaser.chars().take(TEASER_MAX_CHARS).collect();
        let line = if teaser.is_empty() {
            format!("- {id}\n")
        } else {
            format!("- {id}: {teaser}\n")
        };
        if out.len() + line.len() > INDEX_MAX_CHARS {
            out.push_str("- (truncated; see directory::skills::list)\n");
            break;
        }
        out.push_str(&line);
    }
    Some(out)
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
    index: Option<String>,
}

/// Register the index hook function and bind it to the harness pre-generate
/// trigger type. The `inject_index` config gate is read on every fire, so
/// toggling it via the configuration worker takes effect without a rebind.
pub fn setup(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg = cfg.clone();
    let cache: Arc<Mutex<Option<IndexCache>>> = Arc::new(Mutex::new(None));

    iii.register_function(
        INDEX_HOOK_ID,
        RegisterFunction::new_async(move |event: PreGenerateEvent| {
            let cfg = cfg.clone();
            let cache = cache.clone();
            async move {
                let snapshot = cfg.load_full();
                if !snapshot.inject_index {
                    return Ok::<_, Error>(PreGenerateResponse {
                        mutations: PreGenerateMutations::default(),
                    });
                }
                let index = {
                    let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
                    let stale = guard
                        .as_ref()
                        .is_none_or(|c| c.built_at.elapsed() > CACHE_TTL);
                    if stale {
                        *guard = Some(IndexCache {
                            built_at: Instant::now(),
                            index: format_index(&collect_entries(&snapshot)),
                        });
                    }
                    guard.as_ref().and_then(|c| c.index.clone())
                };
                Ok(PreGenerateResponse {
                    mutations: mutations_for(&event.generate.system_prompt, index.as_deref()),
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

    fn entries(pairs: &[(&str, &str)]) -> Vec<Entry> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn empty_entries_yield_no_index() {
        assert_eq!(format_index(&[]), None);
    }

    #[test]
    fn index_lists_one_line_per_worker_with_teaser() {
        let out = format_index(&entries(&[
            ("browser", "Drive a real browser."),
            ("fp", ""),
        ]))
        .expect("index");
        assert!(out.contains("directory::skills::get"));
        assert!(out.contains("- browser: Drive a real browser.\n"));
        assert!(out.contains("- fp\n"));
    }

    #[test]
    fn oversized_index_truncates_with_pointer() {
        let big: Vec<Entry> = (0..500)
            .map(|i| (format!("worker-{i:03}"), "x".repeat(100)))
            .collect();
        let out = format_index(&big).expect("index");
        assert!(out.len() <= INDEX_MAX_CHARS + 100);
        assert!(out.contains("(truncated; see directory::skills::list)"));
    }

    #[test]
    fn empty_base_is_preserved_never_replaced() {
        let m = mutations_for("", Some("## Installed skill docs"));
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
}
