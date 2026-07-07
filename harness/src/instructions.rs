//! Worker-contributed prompt sections + operator-editable instructions.
//!
//! Two sources feed the system prompt beyond the identity prompt
//! (harness.md § System prompt):
//!
//! - **Worker-declared**: a worker publishes agent guidance as
//!   `metadata.agent_instructions` on the configuration entry it already
//!   registers (`configuration::register`). The section is injected only while
//!   the worker is live — some function id with prefix `<entry_id>::` exists in
//!   the cached registry snapshot — so installing the shell worker adds its
//!   coder guidance and stopping it removes it.
//! - **Operator-edited**: the harness registers a dedicated `instructions`
//!   configuration entry (`{ global, workers: { <name> } }`) so instructions
//!   stay reviewable in the console and out of the turn-loop tuning entry.
//!
//! Both are cached in one hot-swappable snapshot, seeded at boot and refreshed
//! by an *unfiltered* `configuration` trigger (any entry registering/updating
//! can change the section set). Mirrors the `configuration` / `discovery`
//! cell pattern. Best-effort throughout: instructions must never brick boot or
//! fail a send — a fetch failure keeps the previous snapshot.

use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::configuration::trigger_with_retry;
use crate::discovery::FunctionsSnapshot;
use crate::prompt::WorkerSection;

/// Reserved configuration entry id for operator-edited instructions.
pub const INSTRUCTIONS_CONFIG_ID: &str = "instructions";
const INSTRUCTIONS_FN_ID: &str = "harness::on-instructions-change";
/// Configuration-entry metadata key a worker declares its guidance under.
const METADATA_KEY: &str = "agent_instructions";
/// A section larger than this logs a warning (never truncated — a cut
/// instruction is worse than a long one).
const SECTION_WARN_BYTES: usize = 8 * 1024;

/// Operator-edited instructions (the `instructions` entry value).
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct UserInstructions {
    /// Appended to every agent turn, after all worker sections.
    #[serde(default)]
    pub global: Option<String>,
    /// Keyed by worker configuration-entry id; appended after that worker's
    /// declared section (or forms the section when the worker declares none).
    #[serde(default)]
    pub workers: HashMap<String, String>,
}

/// One worker's declared guidance (from configuration-entry metadata).
#[derive(Debug, Clone)]
pub struct DeclaredSection {
    pub worker_id: String,
    pub text: String,
}

/// The instructions snapshot: worker-declared sections (sorted by worker id)
/// plus the operator-edited entry value.
#[derive(Debug, Default)]
pub struct InstructionsSnapshot {
    pub declared: Vec<DeclaredSection>,
    pub user: UserInstructions,
}

/// Hot-swappable instructions snapshot shared with the turn entry points.
pub type InstructionsCell = Arc<RwLock<Arc<InstructionsSnapshot>>>;

pub fn new_cell() -> InstructionsCell {
    Arc::new(RwLock::new(Arc::new(InstructionsSnapshot::default())))
}

/// Register the `instructions` configuration entry. The empty default is
/// seeded as `initial_value` only when nothing is stored yet (safe to call
/// every boot). Best-effort: a failure warns and the harness boots without an
/// editable entry until the next restart.
pub async fn register_entry(iii: &IIIClient) {
    let mut payload = json!({
        "id": INSTRUCTIONS_CONFIG_ID,
        "name": "Agent instructions",
        "description": "Operator-edited agent instructions appended to every system prompt: \
                        `global` applies to all turns; `workers.<name>` is appended to that \
                        worker's prompt section while it is running.",
        "schema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "global": {
                    "type": "string",
                    "description": "Markdown appended to every agent turn."
                },
                "workers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Per-worker markdown, keyed by the worker's configuration \
                                    entry id (e.g. `shell`). Injected only while that worker \
                                    is running."
                }
            }
        },
    });
    match should_seed(iii).await {
        Ok(true) => payload["initial_value"] = json!({ "global": "", "workers": {} }),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(error = %e, "instructions entry pre-check failed; registering without a seed");
        }
    }
    if let Err(e) = trigger_with_retry(iii, "configuration::register", payload).await {
        tracing::warn!(error = %e, "registering the `instructions` configuration entry failed");
    }
}

async fn should_seed(iii: &IIIClient) -> Result<bool, String> {
    match get_entry_value(iii, INSTRUCTIONS_CONFIG_ID).await? {
        None => Ok(true),
        Some(v) if v.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

/// `configuration::get` tolerant of a missing entry (codes vary in case).
async fn get_entry_value(iii: &IIIClient, id: &str) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": id })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Non-empty `metadata.agent_instructions` string of one `configuration::list`
/// item; anything else (absent, wrong type, blank) is `None`.
fn declared_of(item: &Value) -> Option<DeclaredSection> {
    let worker_id = item.get("id")?.as_str()?;
    if worker_id == INSTRUCTIONS_CONFIG_ID {
        return None;
    }
    let text = item.get("metadata")?.get(METADATA_KEY)?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if text.len() > SECTION_WARN_BYTES {
        tracing::warn!(
            worker_id,
            bytes = text.len(),
            "agent_instructions section is unusually large; it is injected verbatim into every \
             system prompt"
        );
    }
    Some(DeclaredSection {
        worker_id: worker_id.to_string(),
        text: text.to_string(),
    })
}

/// Fetch the authoritative snapshot: every entry's `agent_instructions`
/// metadata (one `configuration::list`) plus the operator entry value.
async fn fetch_snapshot(iii: &IIIClient) -> Result<InstructionsSnapshot, String> {
    let list = trigger_with_retry(iii, "configuration::list", json!({})).await?;
    let mut declared: Vec<DeclaredSection> = list
        .get("configurations")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(declared_of).collect())
        .unwrap_or_default();
    declared.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));

    let user = match get_entry_value(iii, INSTRUCTIONS_CONFIG_ID).await? {
        Some(v) if !v.is_null() => serde_json::from_value(v).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "malformed `instructions` entry value; ignoring it");
            UserInstructions::default()
        }),
        _ => UserInstructions::default(),
    };
    Ok(InstructionsSnapshot { declared, user })
}

/// Seed (or refresh) the snapshot. A failed fetch keeps the previous snapshot.
pub async fn reload(iii: &IIIClient, cell: &InstructionsCell) {
    match fetch_snapshot(iii).await {
        Ok(snapshot) => {
            let count = snapshot.declared.len();
            *cell.write().await = Arc::new(snapshot);
            tracing::debug!(count, "instructions snapshot refreshed");
        }
        Err(e) => {
            tracing::warn!(error = %e, "instructions fetch failed; keeping previous snapshot");
        }
    }
}

/// Internal `harness::on-instructions-change` payload (advisory; the handler
/// re-fetches the full snapshot).
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnInstructionsChangeEvent {
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `harness::on-instructions-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnInstructionsChangeResponse {
    pub ok: bool,
}

/// Register the internal refresh handler and bind an unfiltered
/// `configuration` trigger: ANY entry registering/updating/deleting can change
/// the section set (a new worker's metadata, an edited `instructions` value).
/// Best-effort: a failed bind warns — the boot seed still serves.
pub fn register_instructions_trigger(iii: &Arc<IIIClient>, cell: InstructionsCell) {
    let engine = iii.clone();
    iii.register_function(
        INSTRUCTIONS_FN_ID,
        RegisterFunction::new_async(move |_event: OnInstructionsChangeEvent| {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                reload(&engine, &cell).await;
                Ok::<OnInstructionsChangeResponse, iii_sdk::errors::Error>(
                    OnInstructionsChangeResponse { ok: true },
                )
            }
        })
        .description(
            "Internal: refresh the cached agent-instructions snapshot when any configuration \
             entry changes (worker `agent_instructions` metadata or the `instructions` entry).",
        )
        .metadata(json!({ "internal": true })),
    );

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: INSTRUCTIONS_FN_ID.to_string(),
        config: json!({
            "event_types": [
                "configuration:registered",
                "configuration:updated",
                "configuration:deleted",
            ],
        }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!("instructions-change trigger bound (all configuration entries)"),
        Err(e) => tracing::warn!(
            error = %e,
            "binding the instructions-change trigger failed; sections will not auto-refresh"
        ),
    }
}

/// The prompt-ready sections for the CURRENT registry: a worker contributes
/// while some `<worker_id>::` function is registered. Operator per-worker text
/// rides its worker's section (and forms one on its own for a live worker that
/// declares nothing). Sorted by worker id — deterministic across turns, so an
/// unchanged mesh keeps the prompt byte-stable.
pub fn live_sections(
    snapshot: &InstructionsSnapshot,
    functions: &FunctionsSnapshot,
) -> Vec<WorkerSection> {
    let is_live = |worker_id: &str| {
        let prefix = format!("{worker_id}::");
        functions
            .functions
            .iter()
            .any(|f| f.function_id.starts_with(&prefix))
    };

    let mut ids: Vec<&str> = snapshot
        .declared
        .iter()
        .map(|s| s.worker_id.as_str())
        .chain(snapshot.user.workers.keys().map(String::as_str))
        .collect();
    ids.sort_unstable();
    ids.dedup();

    ids.into_iter()
        .filter(|id| is_live(id))
        .filter_map(|id| {
            let declared = snapshot
                .declared
                .iter()
                .find(|s| s.worker_id == id)
                .map(|s| s.text.clone());
            let user = snapshot
                .user
                .workers
                .get(id)
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(str::to_string);
            if declared.is_none() && user.is_none() {
                return None;
            }
            Some(WorkerSection {
                worker: id.to_string(),
                declared,
                user,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::FunctionDescriptor;

    fn functions_of(ids: &[&str]) -> FunctionsSnapshot {
        FunctionsSnapshot::for_tests(
            ids.iter()
                .map(|id| FunctionDescriptor {
                    function_id: id.to_string(),
                    description: None,
                    parameters: None,
                })
                .collect(),
        )
    }

    fn declared(worker: &str, text: &str) -> DeclaredSection {
        DeclaredSection {
            worker_id: worker.into(),
            text: text.into(),
        }
    }

    #[test]
    fn declared_of_accepts_only_non_empty_strings() {
        let section = declared_of(&json!({
            "id": "shell",
            "metadata": { "agent_instructions": "Use coder::* for code files." }
        }))
        .expect("valid section");
        assert_eq!(section.worker_id, "shell");
        assert_eq!(section.text, "Use coder::* for code files.");

        for item in [
            json!({ "id": "shell" }),
            json!({ "id": "shell", "metadata": {} }),
            json!({ "id": "shell", "metadata": { "agent_instructions": "   " } }),
            json!({ "id": "shell", "metadata": { "agent_instructions": 42 } }),
            json!({ "id": "instructions", "metadata": { "agent_instructions": "self" } }),
            json!({ "metadata": { "agent_instructions": "no id" } }),
        ] {
            assert!(declared_of(&item).is_none(), "accepted: {item}");
        }
    }

    #[test]
    fn live_sections_gates_on_registered_function_prefix() {
        let snapshot = InstructionsSnapshot {
            declared: vec![
                declared("shell", "shell text"),
                declared("email", "email text"),
            ],
            user: UserInstructions::default(),
        };
        // Only shell is live; `shellac::run` must not match the `shell::` gate.
        let functions = functions_of(&["shell::exec", "shellac::run"]);
        let sections = live_sections(&snapshot, &functions);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].worker, "shell");
        assert_eq!(sections[0].declared.as_deref(), Some("shell text"));
    }

    #[test]
    fn live_sections_merges_user_text_and_emits_user_only_sections() {
        let snapshot = InstructionsSnapshot {
            declared: vec![declared("shell", "shell text")],
            user: UserInstructions {
                global: None,
                workers: [
                    ("shell".to_string(), "prefer rg".to_string()),
                    ("email".to_string(), "always cc ops".to_string()),
                    ("web".to_string(), "  ".to_string()),
                ]
                .into(),
            },
        };
        let functions = functions_of(&["shell::exec", "email::send", "web::fetch"]);
        let sections = live_sections(&snapshot, &functions);
        // Sorted by worker id; blank user text for `web` emits nothing.
        assert_eq!(
            sections
                .iter()
                .map(|s| s.worker.as_str())
                .collect::<Vec<_>>(),
            vec!["email", "shell"]
        );
        assert_eq!(sections[0].declared, None);
        assert_eq!(sections[0].user.as_deref(), Some("always cc ops"));
        assert_eq!(sections[1].declared.as_deref(), Some("shell text"));
        assert_eq!(sections[1].user.as_deref(), Some("prefer rg"));
    }

    #[test]
    fn live_sections_skips_stopped_workers_entirely() {
        let snapshot = InstructionsSnapshot {
            declared: vec![declared("shell", "shell text")],
            user: UserInstructions {
                global: None,
                workers: [("shell".to_string(), "prefer rg".to_string())].into(),
            },
        };
        assert!(live_sections(&snapshot, &functions_of(&[])).is_empty());
    }
}
