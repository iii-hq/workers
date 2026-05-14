//! Bootstrap step: pre-populate `iii-directory`'s `skills_folder` with
//! the bundles referenced by `system_default_skills` on worker boot.
//!
//! The per-chat `provisioning` state already fetches each URI via
//! `directory::skills::get` — but those fetches return recovery stubs
//! if the files aren't on disk yet. This step front-loads the work
//! once at boot by deriving a target list from `system_default_skills`
//! (each `iii://<ns>/<rest>` → download bundle `<ns>`) and firing
//! `directory::skills::download` per namespace.
//!
//! The download function overwrites file-by-file
//! (`iii-directory/src/functions/download.rs`), so re-running on every
//! boot is safe; it always reconciles to the registry's latest. The
//! same code path is what the agent uses mid-chat to pull a new skill,
//! so there's exactly one mechanism for "make this skill available."
//!
//! Failures are logged per-namespace and never abort startup; missing
//! skills degrade to the existing soft-fail recovery stub in
//! [`crate::states::provisioning`].

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

use crate::config::TurnOrchestratorConfig;

/// Per-download trigger timeout. Wide enough for `git clone` over a
/// slow link; iii-directory enforces its own `download_timeout_ms`
/// internally as a tighter bound.
const DOWNLOAD_TIMEOUT_MS: u64 = 90_000;

/// How long to wait for `directory::skills::download` to register
/// before giving up. iii-directory normally registers in < 1s; the
/// long ceiling tolerates cold-start environments.
const READY_DEADLINE: Duration = Duration::from_secs(30);

/// Poll interval while waiting for the download function to register.
const READY_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Per-poll readiness probe timeout.
const PROBE_TIMEOUT_MS: u64 = 2_000;

/// Function we depend on. Hoisted so the readiness probe and the
/// actual call stay in sync.
const DOWNLOAD_FN: &str = "directory::skills::download";

/// Run the bootstrap. Never returns an error so a flaky registry
/// can't block turn-orchestrator from coming up.
pub async fn run(iii: Arc<III>, cfg: Arc<TurnOrchestratorConfig>) {
    let namespaces = derive_namespaces(&cfg.system_default_skills);
    if namespaces.is_empty() {
        tracing::debug!("bootstrap: no namespaces derivable from system_default_skills; skipping");
        return;
    }

    if !wait_for_download_fn(&iii).await {
        tracing::warn!(
            "bootstrap: {DOWNLOAD_FN} did not register within {}s; \
             system_default_skills will fall back to recovery stubs",
            READY_DEADLINE.as_secs(),
        );
        return;
    }

    let total = namespaces.len();
    let mut ok = 0usize;
    for (i, ns) in namespaces.iter().enumerate() {
        match iii
            .trigger(TriggerRequest {
                function_id: DOWNLOAD_FN.into(),
                payload: json!({ "worker": ns }),
                action: None,
                timeout_ms: Some(DOWNLOAD_TIMEOUT_MS),
            })
            .await
        {
            Ok(resp) => {
                ok += 1;
                let skills = resp
                    .get("skills_written")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                let prompts = resp
                    .get("prompts_written")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                tracing::info!(
                    target: "turn_orchestrator::bootstrap",
                    namespace = %ns,
                    skills,
                    prompts,
                    "bootstrap: downloaded ({} of {total})",
                    i + 1,
                );
            }
            Err(err) => {
                tracing::warn!(
                    target: "turn_orchestrator::bootstrap",
                    namespace = %ns,
                    %err,
                    "bootstrap: download failed; chats will see recovery stubs \
                     for this namespace until manually re-downloaded",
                );
            }
        }
    }
    tracing::info!(
        target: "turn_orchestrator::bootstrap",
        "bootstrap: {ok}/{total} namespaces populated from system_default_skills",
    );
}

/// Extract the leading namespace from each `iii://<ns>/<rest>` URI,
/// dedupe (preserving config order via [`BTreeSet`] insertion-by-key
/// is not what we want — we want insertion order). Use a `Vec` with a
/// seen-set to keep config order stable for logs.
fn derive_namespaces(uris: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for uri in uris {
        let Some(ns) = leading_namespace(uri) else {
            tracing::debug!(%uri, "bootstrap: URI has no derivable namespace; skipping");
            continue;
        };
        if seen.insert(ns.clone()) {
            out.push(ns);
        }
    }
    out
}

/// Return the first path segment of an `iii://<ns>/<rest>` URI.
/// Tolerates absent `iii://` prefix and empty rest.
fn leading_namespace(uri: &str) -> Option<String> {
    let body = uri.strip_prefix("iii://").unwrap_or(uri);
    let ns = body.split('/').next()?;
    if ns.is_empty() {
        return None;
    }
    Some(ns.to_string())
}

/// Poll `engine::functions::list` until [`DOWNLOAD_FN`] appears or
/// [`READY_DEADLINE`] elapses.
async fn wait_for_download_fn(iii: &III) -> bool {
    let deadline = Instant::now() + READY_DEADLINE;
    loop {
        if probe_once(iii).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }
}

async fn probe_once(iii: &III) -> bool {
    let Ok(value) = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({"include_internal": false}),
            action: None,
            timeout_ms: Some(PROBE_TIMEOUT_MS),
        })
        .await
    else {
        return false;
    };
    let Some(fns) = value.get("functions").and_then(Value::as_array) else {
        return false;
    };
    fns.iter()
        .any(|f| f.get("function_id").and_then(Value::as_str) == Some(DOWNLOAD_FN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_namespace_strips_iii_prefix() {
        assert_eq!(
            leading_namespace("iii://iii-directory/index"),
            Some("iii-directory".to_string())
        );
    }

    #[test]
    fn leading_namespace_handles_missing_prefix() {
        assert_eq!(
            leading_namespace("shell/index"),
            Some("shell".to_string())
        );
    }

    #[test]
    fn leading_namespace_single_segment() {
        assert_eq!(leading_namespace("iii://iii"), Some("iii".to_string()));
    }

    #[test]
    fn leading_namespace_rejects_empty() {
        assert_eq!(leading_namespace("iii://"), None);
        assert_eq!(leading_namespace(""), None);
    }

    #[test]
    fn leading_namespace_deep_path() {
        assert_eq!(
            leading_namespace("iii://iii-directory/directory/engine/functions/list"),
            Some("iii-directory".to_string())
        );
    }

    #[test]
    fn derive_namespaces_dedupes_preserving_first_occurrence() {
        let uris = vec![
            "iii://iii-directory/index".to_string(),
            "iii://shell/index".to_string(),
            "iii://iii-directory/directory/engine/functions/list".to_string(),
            "iii://shell/index".to_string(),
        ];
        assert_eq!(
            derive_namespaces(&uris),
            vec!["iii-directory".to_string(), "shell".to_string()]
        );
    }

    #[test]
    fn derive_namespaces_skips_malformed_uris() {
        let uris = vec![
            "iii://".to_string(),
            "iii://valid/x".to_string(),
            String::new(),
        ];
        assert_eq!(derive_namespaces(&uris), vec!["valid".to_string()]);
    }

    #[test]
    fn derive_namespaces_empty_input_returns_empty() {
        assert!(derive_namespaces(&[]).is_empty());
    }
}
