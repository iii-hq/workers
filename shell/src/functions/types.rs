//! Typed request/response shapes for the 5 `shell::*` (non-fs) functions.
//! These mirror the inline `json!()` schemas that lived in `main.rs` before
//! the transition to `RegisterFunction::new_async`. The wire format is
//! unchanged from prior versions.
//!
//! `ExecRequest` and `ExecBgRequest` use per-field custom deserializers so the
//! published JSON schema (visible in the engine's tool listing) tells the
//! caller `command: string` while keeping the loose `timeout_ms` semantic
//! and the actionable error texts on type mismatches.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::exec::ExecOutcome;
use crate::fs::FsScope;
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
    /// Optional working directory for this call (host target only). Confined to
    /// the fs jail exactly like `shell::fs::*` paths: jail-relative when
    /// `fs.host_roots` are set (else absolute), canonicalized, and must resolve
    /// inside a jail root and miss the denylist — a path that escapes returns
    /// S215. Must already exist and be a directory. Omitted, the call runs in
    /// the session working directory when it is session-scoped (the default for
    /// console chats), else the configured `working_dir`. Rejected (S210) on a
    /// sandbox target.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<FsScope>,
    /// Optional per-call environment values (host target only). A key may be
    /// set ONLY if the operator listed it in `env.allow`, and NEVER for an
    /// exec-hijacking key (PATH, IFS, HOME, LD_*/DYLD_*, and other loader/lookup
    /// and interpreter-startup keys — see DANGEROUS_ENV_KEYS) — those are
    /// rejected even if allowlisted. Supplying a key that is not in `env.allow`,
    /// or any dangerous key, rejects the WHOLE call (S210) naming the offending
    /// key; the env is never silently dropped. Permitted values override what
    /// would otherwise be forwarded for that key. Rejected (S210) on a sandbox target.
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    /// Optional bytes written to the program's standard input, followed by EOF.
    /// Use this to pipe content to a command that reads stdin (`tee`, `patch`,
    /// `cat`, a filter) instead of wrapping it in a shell heredoc. Omit to leave
    /// stdin closed (`/dev/null`, the default). HOST target only — rejected
    /// (S210) on a sandbox target, like cwd/env.
    #[serde(default)]
    pub stdin: Option<String>,
    /// Where to run the command. Defaults to the host worker; pass
    /// `{ kind: "sandbox", sandbox_id }` to forward the call to a microVM.
    #[serde(default)]
    pub target: Target,
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
    /// Optional working directory for this job (host target only). Same jail
    /// confinement and rules as [`ExecRequest::cwd`]: canonicalized, must
    /// resolve inside a jail root (S215 on escape) and be an existing
    /// directory. Rejected (S210) on a sandbox target.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<FsScope>,
    /// Optional per-call environment values (host target only). Same gating as
    /// [`ExecRequest::env`]: a key must be in `env.allow` and must not be an
    /// exec-hijacking key (PATH, IFS, HOME, LD_*/DYLD_*, and other loader/lookup
    /// and interpreter-startup keys — see DANGEROUS_ENV_KEYS); any violation
    /// rejects the whole call (S210). Rejected (S210) on a sandbox target.
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    /// Optional bytes written to the job's standard input, then EOF. See
    /// [`ExecRequest::stdin`]. HOST target only — rejected (S210) on a sandbox
    /// target.
    #[serde(default)]
    pub stdin: Option<String>,
    /// Where to run. See [`ExecRequest::target`].
    #[serde(default)]
    pub target: Target,
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

/// `shell::list` takes no arguments; this empty struct publishes an accurate
/// (empty-object) request schema.
///
/// NOTE: deliberately NOT `#[serde(deny_unknown_fields)]`. The engine injects
/// a `_caller_worker_id` field into every call's payload, so a strict no-arg
/// struct would reject EVERY call. The handler ignores the payload entirely
/// (`move |_req: Value|` in main.rs) for the same reason.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub jobs: Vec<JobSummary>,
    pub count: usize,
}

/// `shell::config-status` takes no arguments; this empty struct publishes an
/// accurate (empty-object) request schema, mirroring [`ListRequest`]. The
/// response is `configuration::ReloadStatus`. Like [`ListRequest`], it is NOT
/// `deny_unknown_fields` — the engine-injected `_caller_worker_id` would
/// otherwise be rejected; the handler ignores the payload.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConfigStatusRequest {}
