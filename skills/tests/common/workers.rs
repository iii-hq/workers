//! In-process registration of the `skills` surface against the
//! shared SDK handle. Re-uses the same entry point the production
//! binary does (`iii_skills::functions::register_all`), so the BDD
//! scenarios exercise identical code paths.
//!
//! The registered state is kept once-per-binary in `OnceCell`. The
//! cucumber `before` hook in `tests/bdd.rs` wipes the two state scopes
//! between scenarios so each scenario starts from a clean slate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use iii_skills::{
    config::SkillsConfig,
    functions,
    trigger_types::{self, RegisteredTriggerTypes},
};

pub struct Shared {
    pub cfg: Arc<SkillsConfig>,
    /// Kept so step defs needing to assert on subscriber-set state can
    /// probe it directly (the registered trigger types live here).
    #[allow(dead_code)]
    pub triggers: Arc<RegisteredTriggerTypes>,
    /// Root directory the fs-sources scenarios write fixtures into.
    /// Layout under this dir mirrors the configured globs:
    ///   `<fs_root>/fs-skills/...`  → matched by `fs-skills/**/*.md`
    ///   `<fs_root>/fs-prompts/...` → matched by `fs-prompts/**/*.md`
    pub fs_root: PathBuf,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<III>) -> Result<Arc<Shared>> {
    if let Some(s) = SHARED.get() {
        return Ok(s.clone());
    }

    // Build a leaked tempdir that lives for the test binary lifetime.
    // The handlers below clone the cfg's config_dir into their closures
    // so all subsequent fs scans resolve relative to this path.
    let tmp = tempfile::tempdir()?;
    let fs_root = tmp.keep();
    std::fs::create_dir_all(fs_root.join("fs-skills"))?;
    std::fs::create_dir_all(fs_root.join("fs-prompts"))?;

    let cfg = Arc::new(SkillsConfig {
        skills: vec!["fs-skills/**/*.md".into()],
        prompts: vec!["fs-prompts/**/*.md".into()],
        config_dir: Some(fs_root.clone()),
        ..SkillsConfig::default()
    });
    let registered = trigger_types::register_all(iii);
    functions::register_all(iii, &cfg, &registered);

    // Give the SDK a beat to publish the function registrations before
    // scenarios start triggering them.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let shared = Arc::new(Shared {
        cfg,
        triggers: Arc::new(registered),
        fs_root,
    });
    let _ = SHARED.set(shared.clone());
    Ok(shared)
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}

/// Wipe the fs-fixture directories between scenarios so leftover files
/// from one scenario don't pollute the next. Only touches the per-test
/// fs root — the rest of the file system is untouched.
pub fn reset_fs(fs_root: &Path) {
    for sub in ["fs-skills", "fs-prompts"] {
        let dir = fs_root.join(sub);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
    }
}

/// Wipe both state scopes between scenarios. Called from the cucumber
/// `before` hook. Iterates the current contents of each scope and
/// deletes each entry; `state::delete` is idempotent so a race with
/// another scenario's teardown is harmless.
pub async fn reset_state(iii: &III) {
    for scope in ["skills", "prompts"] {
        let listed = iii
            .trigger(TriggerRequest {
                function_id: "state::list".to_string(),
                payload: json!({ "scope": scope }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
            .ok();
        let Some(raw) = listed else {
            continue;
        };
        for value in extract_ids(raw, scope) {
            let _ = iii
                .trigger(TriggerRequest {
                    function_id: "state::delete".to_string(),
                    payload: json!({ "scope": scope, "key": value }),
                    action: None,
                    timeout_ms: Some(2_000),
                })
                .await;
        }
    }
}

/// `state::list` returns different shapes depending on the backend.
/// Extract the entries whose `.id` / `.name` looks like a key for our
/// scope and return those keys.
fn extract_ids(raw: Value, scope: &str) -> Vec<String> {
    let values: Vec<Value> = if let Value::Array(arr) = raw {
        arr
    } else if let Some(items) = raw.get("items").and_then(|v| v.as_array()) {
        items
            .iter()
            .filter_map(|item| item.get("value").cloned())
            .collect()
    } else if let Some(arr) = raw.get("value").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        return Vec::new();
    };
    values
        .into_iter()
        .filter_map(|v| match scope {
            "skills" => v.get("id").and_then(|x| x.as_str()).map(String::from),
            "prompts" => v.get("name").and_then(|x| x.as_str()).map(String::from),
            _ => None,
        })
        .collect()
}
