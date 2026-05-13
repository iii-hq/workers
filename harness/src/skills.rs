//! First-boot bootstrap of `skills_folder` via the `iii-directory` worker.
//!
//! On harness boot we want every bundled worker's skill to be present on disk
//! so `directory::skills::fetch-skill iii://<worker>/index` returns content immediately. The
//! `iii-directory` worker (formerly `skills`) is the source of truth: a single
//! `directory::skills::list` call returns every namespace currently materialised. For
//! each name in [`BOOTSTRAP_NAMES`] that isn't present yet we trigger
//! `directory::skills::download` with the version pinned by `iii.lock`.
//!
//! Bootstrap is best-effort. A missing registry or transient failure logs a
//! warning and continues; harness still comes up. Subsequent boots are
//! instant — `directory::skills::list` reports everything is present, no downloads.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::EXPECTED_WORKERS;

/// Workers the harness expects to have a skill on disk after first boot.
///
/// Order matches `iii.worker.yaml` for readability; ordering does not affect
/// bootstrap behaviour (downloads run concurrently).
///
/// Infra workers (`iii-state`, `iii-queue`, `iii-stream`, `iii-bridge`,
/// `iii-http`, `iii-sandbox`, provider-*, oauth-*) are intentionally not in
/// the bootstrap list — they don't ship a `<worker>/skill.md`.
pub const BOOTSTRAP_NAMES: &[&str] = &[
    "harness",
    "iii-directory",
    "auth-credentials",
    "turn-orchestrator",
    "approval-gate",
    "shell",
    "models-catalog",
    "llm-budget",
    "session",
];

/// Default per-download timeout. Matches `iii-directory`'s default
/// `download_timeout_ms`. Public so tests can compose tighter bounds.
pub const DOWNLOAD_TIMEOUT_MS: u64 = 60_000;

/// Default `directory::skills::list` timeout. Cheap call (no network), so short.
pub const LIST_TIMEOUT_MS: u64 = 5_000;

/// Bounded concurrency for parallel downloads. Higher values just queue on the
/// registry side and risk rate limiting; lower values stretch first-boot
/// wall-clock unnecessarily.
pub const BOOTSTRAP_CONCURRENCY: usize = 4;

/// The harness worker name. The harness is its own bundled worker (publishes
/// `harness/skill.md`) but never appears as a dependency in its own
/// `iii.worker.yaml`, so it's exempt from the drift assert below.
const SELF_WORKER: &str = "harness";

/// Boot-time guard: every [`BOOTSTRAP_NAMES`] entry must appear in
/// [`EXPECTED_WORKERS`] (the manifest the engine spawns). Drift between the
/// two would mean we either skip a bundled worker or try to download a
/// stranger. `harness` is exempt — see [`SELF_WORKER`].
fn assert_bootstrap_subset_of_expected() {
    for name in BOOTSTRAP_NAMES {
        if *name == SELF_WORKER {
            continue;
        }
        assert!(
            EXPECTED_WORKERS.contains(name),
            "BOOTSTRAP_NAMES entry {name:?} is not in EXPECTED_WORKERS — \
             either add it to harness/iii.worker.yaml or remove it from BOOTSTRAP_NAMES",
        );
    }
}

/// Run first-boot bootstrap. Lists what is present on disk, then triggers
/// `directory::skills::download` for every [`BOOTSTRAP_NAMES`] entry that's missing.
///
/// Best-effort: any failure (registry down, version mismatch, timeout) is
/// logged and skipped. The harness still boots.
pub async fn bootstrap_run(iii: &III) -> anyhow::Result<()> {
    assert_bootstrap_subset_of_expected();

    let present = list_present_namespaces(iii).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "harness bootstrap: directory::skills::list failed, assuming nothing present");
        HashSet::new()
    });
    let lock = read_iii_lock_versions().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "harness bootstrap: could not read iii.lock; downloads will omit version");
        HashMap::new()
    });

    let sem = Arc::new(Semaphore::new(BOOTSTRAP_CONCURRENCY));
    let mut set: JoinSet<()> = JoinSet::new();
    for name in BOOTSTRAP_NAMES {
        if present.contains(*name) {
            tracing::debug!(
                worker = name,
                "harness bootstrap: skill already present, skipping"
            );
            continue;
        }
        let version = lock.get(*name).cloned();
        let iii = iii.clone();
        let sem = Arc::clone(&sem);
        let name = (*name).to_string();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            match run_one(&iii, &name, version.as_deref()).await {
                Ok(()) => tracing::info!(worker = name, "harness bootstrap: downloaded skill"),
                Err(e) => {
                    tracing::warn!(worker = name, error = %e, "harness bootstrap: download failed");
                }
            }
        });
    }
    while set.join_next().await.is_some() {}
    Ok(())
}

/// Trigger `directory::skills::download` for a single worker. Returns Ok on a non-error
/// response from `iii-directory`; the response body is otherwise ignored.
pub(crate) async fn run_one(
    iii: &III,
    worker: &str,
    version: Option<&str>,
) -> Result<(), iii_sdk::IIIError> {
    let payload = build_download_payload(worker, version);
    iii.trigger(TriggerRequest {
        function_id: "directory::skills::download".into(),
        payload,
        action: None,
        timeout_ms: Some(DOWNLOAD_TIMEOUT_MS),
    })
    .await
    .map(|_| ())
}

/// Build the `directory::skills::download` payload. Pure helper so the shape is
/// testable without a live engine.
pub(crate) fn build_download_payload(worker: &str, version: Option<&str>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("worker".into(), Value::String(worker.to_string()));
    if let Some(v) = version {
        obj.insert("version".into(), Value::String(v.to_string()));
    }
    Value::Object(obj)
}

/// Trigger `directory::skills::list` and extract the set of present namespaces (the
/// first segment of every returned skill id, e.g. `shell/index` →
/// `shell`).
pub(crate) async fn list_present_namespaces(
    iii: &III,
) -> Result<HashSet<String>, iii_sdk::IIIError> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(LIST_TIMEOUT_MS),
        })
        .await?;
    Ok(extract_namespaces(&resp))
}

/// Pull the set of top-level namespaces out of a `directory::skills::list` response.
/// Accepts both the canonical `{ skills: [{ id, .. }] }` envelope and a bare
/// array of `{ id }`.
pub(crate) fn extract_namespaces(resp: &Value) -> HashSet<String> {
    let arr = resp
        .get("skills")
        .and_then(|v| v.as_array())
        .or_else(|| resp.as_array());
    let Some(arr) = arr else {
        return HashSet::new();
    };
    let mut out = HashSet::new();
    for entry in arr {
        if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
            if let Some(ns) = id.split('/').next() {
                if !ns.is_empty() {
                    out.insert(ns.to_string());
                }
            }
        }
    }
    out
}

/// Parse `iii.lock` and return `worker -> version`. Searches the harness
/// crate dir and the workspace root for a lockfile.
pub(crate) fn read_iii_lock_versions() -> anyhow::Result<HashMap<String, String>> {
    let path = find_iii_lock().ok_or_else(|| anyhow::anyhow!("iii.lock not found"))?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    parse_iii_lock_versions(&text)
}

/// Pure parser for the `iii.lock` YAML body. Extracts the `version` of each
/// worker under `workers:`. Separated from the IO call so it can be unit
/// tested against a fixed string.
pub(crate) fn parse_iii_lock_versions(text: &str) -> anyhow::Result<HashMap<String, String>> {
    let v: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|e| anyhow::anyhow!("parse iii.lock as YAML: {e}"))?;
    let mut out = HashMap::new();
    let Some(workers) = v.get("workers").and_then(serde_yaml::Value::as_mapping) else {
        return Ok(out);
    };
    for (name, entry) in workers {
        let Some(name) = name.as_str() else { continue };
        if let Some(version) = entry.get("version").and_then(serde_yaml::Value::as_str) {
            out.insert(name.to_string(), version.to_string());
        }
    }
    Ok(out)
}

fn find_iii_lock() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("iii.lock"),
        PathBuf::from("../iii.lock"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("iii.lock"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("../iii.lock"), |p| p.join("iii.lock")),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_names_subset_of_expected_workers() {
        // Mirrors the runtime assert so cargo test catches drift even when
        // bootstrap_run is never invoked. `harness` itself is exempt — it's
        // the meta-worker doing the bootstrap, not a dependency.
        for name in BOOTSTRAP_NAMES {
            if *name == SELF_WORKER {
                continue;
            }
            assert!(
                EXPECTED_WORKERS.contains(name),
                "BOOTSTRAP_NAMES entry {name:?} missing from EXPECTED_WORKERS",
            );
        }
    }

    #[test]
    fn extract_namespaces_handles_skills_envelope() {
        let v = json!({
            "skills": [
                { "id": "shell/index", "bytes": 100, "modified_at": "2026-01-01" },
                { "id": "harness/index", "bytes": 50, "modified_at": "2026-01-01" },
                { "id": "shell/fs/read", "bytes": 20, "modified_at": "2026-01-01" },
            ]
        });
        let ns = extract_namespaces(&v);
        assert!(ns.contains("shell"));
        assert!(ns.contains("harness"));
        assert_eq!(ns.len(), 2);
    }

    #[test]
    fn extract_namespaces_handles_bare_array() {
        let v = json!([
            { "id": "approval-gate/index" },
            { "id": "approval-gate/resolve" },
        ]);
        let ns = extract_namespaces(&v);
        assert_eq!(ns.len(), 1);
        assert!(ns.contains("approval-gate"));
    }

    #[test]
    fn extract_namespaces_empty_on_malformed() {
        assert!(extract_namespaces(&json!({})).is_empty());
        assert!(extract_namespaces(&Value::Null).is_empty());
        assert!(extract_namespaces(&json!({"skills": "not-an-array"})).is_empty());
    }

    #[test]
    fn extract_namespaces_ignores_entries_without_id() {
        let v = json!({
            "skills": [
                { "no_id_here": true },
                { "id": "shell/index" },
                { "id": "" },
            ]
        });
        let ns = extract_namespaces(&v);
        assert_eq!(ns.len(), 1);
        assert!(ns.contains("shell"));
    }

    #[test]
    fn build_download_payload_includes_version_when_present() {
        let p = build_download_payload("shell", Some("0.3.3"));
        assert_eq!(p["worker"], json!("shell"));
        assert_eq!(p["version"], json!("0.3.3"));
    }

    #[test]
    fn build_download_payload_omits_version_when_absent() {
        let p = build_download_payload("shell", None);
        assert_eq!(p["worker"], json!("shell"));
        assert!(p.get("version").is_none());
    }

    #[test]
    fn parse_iii_lock_extracts_versions() {
        let lock = r"
version: 1
workers:
  harness:
    version: 0.1.6
    type: binary
  shell:
    version: 0.3.3
    type: binary
  iii-bridge:
    version: 0.11.6
    type: engine
";
        let map = parse_iii_lock_versions(lock).expect("parse");
        assert_eq!(map.get("harness").map(String::as_str), Some("0.1.6"));
        assert_eq!(map.get("shell").map(String::as_str), Some("0.3.3"));
        assert_eq!(map.get("iii-bridge").map(String::as_str), Some("0.11.6"));
    }

    #[test]
    fn parse_iii_lock_empty_when_no_workers_key() {
        let map = parse_iii_lock_versions("version: 1\n").expect("parse");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_iii_lock_rejects_garbage() {
        // Tab + colon at column 0 — not parseable as YAML.
        assert!(parse_iii_lock_versions("\t:\n\tbad").is_err());
    }
}
