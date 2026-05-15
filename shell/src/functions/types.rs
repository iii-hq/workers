//! Typed request/response shapes for the 6 `shell::*` (non-fs) functions.
//! These mirror the inline `json!()` schemas that lived in `main.rs` before
//! the migration to `RegisterFunction::new_async`. The wire format is
//! unchanged from prior versions.
//!
//! `ExecRequest` and `ExecBgRequest` use per-field custom deserializers so the
//! published JSON schema (visible in the engine's tool listing) tells the
//! caller `command: string` while keeping the loose `timeout_ms` semantic
//! and the actionable error texts on type mismatches.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::exec::ExecOutcome;
use crate::jobs::{JobRecord, JobStatus};
use crate::target::Target;

/// Helpful error text for the recurring "argv-as-array in `command`"
/// mistake. Surfaced both from `deserialize_command` (typed-request path)
/// and from any remaining `Value` parsing sites so the LLM gets the same
/// recovery hint regardless of how the request was deserialized.
const COMMAND_ARRAY_HINT: &str =
    "'command' must be a string (got array). Pass the program name in \
     'command' and arguments in 'args', e.g. \
     {\"command\": \"sh\", \"args\": [\"-lc\", \"ls -la\"]}";

/// Reject anything that isn't a JSON string with the same actionable text the
/// loose `Value` handler used to produce. Keeps the LLM-recovery message
/// stable while letting the rest of the field be typed.
fn deserialize_command<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Array(_) => Err(D::Error::custom(COMMAND_ARRAY_HINT)),
        other => Err(D::Error::custom(format!(
            "'command' must be a string (got {})",
            other
        ))),
    }
}

/// Validate every element is a string with a per-index error. Returns
/// `Option<Vec<String>>` so the handler can distinguish `args: null` /
/// absent (`None`) from an empty array (`Some(vec![])`) — `parse_argv` uses
/// that distinction to decide whether to tokenize `command` via shell-words
/// (the legacy single-string path).
fn deserialize_args<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, el) in arr.into_iter().enumerate() {
                match el {
                    serde_json::Value::String(s) => out.push(s),
                    other => {
                        return Err(D::Error::custom(format!(
                            "'args[{}]' must be a string (got {})",
                            i, other
                        )));
                    }
                }
            }
            Ok(Some(out))
        }
        other => Err(D::Error::custom(format!(
            "'args' must be an array of strings (got {})",
            other
        ))),
    }
}

/// Permissive deserializer: negative integers and floats silently fall back
/// to `None` so the caller hits the default timeout. This preserves the
/// long-standing wire contract documented in `functions/exec.rs` (covered by
/// e2e: `cases-exec-sandbox.ts`).
fn deserialize_timeout_ms<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_u64())
}

/// Marker injected by `approval-gate` when re-invoking after user approval.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ApprovalMarker {
    pub call_id: String,
    pub session_id: String,
}

/// Request body for `shell::classify_argv`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClassifyArgvRequest {
    #[serde(deserialize_with = "deserialize_command")]
    pub command: String,
    #[serde(default, deserialize_with = "deserialize_args")]
    pub args: Option<Vec<String>>,
}

/// Wire request for `shell::exec`. The schema is published to the engine's
/// tool listing so callers see field types up front instead of guessing
/// from the description.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecRequest {
    /// Program name (matched against the allowlist by basename or exact path).
    /// Must be a string — split arguments into `args`, do not pass argv as
    /// an array here.
    #[serde(deserialize_with = "deserialize_command")]
    pub command: String,
    /// Arguments passed to the program, in order. Every element must be a
    /// string; non-string elements are rejected by index.
    /// `None` (or `args: null` / absent) means "tokenize `command` via
    /// shell-words"; `Some(_)` (including the empty vec) means "use args
    /// verbatim, no shell-words." See `parse_argv` in `crate::exec::host`.
    #[serde(default, deserialize_with = "deserialize_args")]
    pub args: Option<Vec<String>>,
    /// Per-call timeout override, milliseconds. Capped at `cfg.max_timeout_ms`.
    /// Negative or fractional values silently fall back to
    /// `cfg.default_timeout_ms` (loose wire semantic, preserved on purpose).
    #[serde(default, deserialize_with = "deserialize_timeout_ms")]
    pub timeout_ms: Option<u64>,
    /// Where to run the command. Defaults to the host worker; pass
    /// `{ kind: "sandbox", sandbox_id }` to forward the call to a microVM.
    #[serde(default)]
    pub target: Target,
    /// Present only on gate-driven re-invocation after approval (§ 6.4).
    #[serde(default, rename = "__from_approval")]
    pub from_approval: Option<ApprovalMarker>,
}

/// Wire request for `shell::exec_bg`. Same shape as [`ExecRequest`]; documented
/// separately so the engine publishes a distinct schema per function.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecBgRequest {
    /// Program name. See [`ExecRequest::command`].
    #[serde(deserialize_with = "deserialize_command")]
    pub command: String,
    /// Arguments passed to the program. See [`ExecRequest::args`].
    /// `None` (or `args: null` / absent) means "tokenize `command` via
    /// shell-words"; `Some(_)` (including the empty vec) means "use args
    /// verbatim, no shell-words." See `parse_argv` in `crate::exec::host`.
    #[serde(default, deserialize_with = "deserialize_args")]
    pub args: Option<Vec<String>>,
    /// Per-call timeout. Host-targeted background jobs IGNORE `timeout_ms`;
    /// sandbox-targeted ones forward it through `cfg.resolve_timeout`.
    #[serde(default, deserialize_with = "deserialize_timeout_ms")]
    pub timeout_ms: Option<u64>,
    /// Where to run. See [`ExecRequest::target`].
    #[serde(default)]
    pub target: Target,
    /// Present only on gate-driven re-invocation after approval (§ 6.4).
    #[serde(default, rename = "__from_approval")]
    pub from_approval: Option<ApprovalMarker>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecResponse {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

// `ExecOutcome` (backend-side) and `ExecResponse` (wire-side) hold the
// same 7 fields; the conversion is a structural copy. Going through
// `From` instead of an inline manual copy in handlers means a future
// field added to one but not the other surfaces as a compile error
// rather than a silent drop.
impl From<ExecOutcome> for ExecResponse {
    fn from(o: ExecOutcome) -> Self {
        Self {
            exit_code: o.exit_code,
            stdout: o.stdout,
            stderr: o.stderr,
            duration_ms: o.duration_ms,
            timed_out: o.timed_out,
            stdout_truncated: o.stdout_truncated,
            stderr_truncated: o.stderr_truncated,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecBgResponse {
    pub job_id: String,
    pub argv: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KillRequest {
    pub job_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KillResponse {
    pub job_id: String,
    pub killed: bool,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusRequest {
    pub job_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub job: JobRecord,
}

/// `shell::list` returns one `JobSummary` per record. argv, stdout, and
/// stderr are deliberately omitted: the global JOBS map is process-wide and
/// has no per-caller scope, so any caller could otherwise read every other
/// caller's command line and captured output (which may embed credentials).
/// Full records remain reachable via `shell::status <job_id>` — the random
/// UUID acts as an unguessable capability for that record.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JobSummary {
    pub id: String,
    pub status: JobStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl From<&JobRecord> for JobSummary {
    fn from(r: &JobRecord) -> Self {
        Self {
            id: r.id.clone(),
            status: r.status.clone(),
            started_at_ms: r.started_at_ms,
            finished_at_ms: r.finished_at_ms,
            exit_code: r.exit_code,
            stdout_truncated: r.stdout_truncated,
            stderr_truncated: r.stderr_truncated,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub jobs: Vec<JobSummary>,
    pub count: usize,
}
