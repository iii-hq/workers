//! `fp::pipe` — a short worker-side pipeline: each step calls a
//! function and its result lands in the next step's payload (`into`, default
//! `/value`). Pure `fp::*` transforms run inline; every other step goes
//! over the bus via `iii.call`. Large values thread worker→worker and
//! never round-trip through the model; the chat gets a receipt with per-step
//! sizes and a preview.
//!
//! Steps run with THIS worker's authority, not the calling agent's dispatch
//! policy — the `fp::pipe` call itself is the policy/approval surface
//! (an approver sees the full step list). `forbidden_step` statically refuses
//! every call the root iii-permissions.yaml denies, less a short table of
//! written exceptions, plus fp's own extra refusals. Database writes are
//! refused only when the approval gate is present.

use aho_corasick::{AhoCorasick, MatchKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use iii_sdk::IIIClient;
use iii_sdk::protocol::{CallRequest, CallRequestWithMetadata};

use crate::util;

pub const PIPE_ID: &str = "fp::pipe";
pub const PIPE_DESC: &str = r#"Run a worker-side pipeline: each step calls a function and its result lands in the next step's payload at `into` (default "/value"); fp::* steps thread the bare value and a failing fp::when stops the pipe. The FIRST step gets no threaded value. Returns per-step sizes and a preview, not the value. A pipe is a chain, not a batch: fp::batch and fp::pipe cannot be steps (no nesting), and neither can engine::observers::dispose, configuration::, oauth::, secrets:: (but list/get/status/detect), mcp-client::, router:: (but its catalog reads router::models::list/get/budget/supports and router::provider::list), provider::, harness::, run::, workflow::start, workflow-result::submit, session writes, approval::, or any other call the agent policy denies (hooks, terminals, console UI) except state writes and kanban planning writes. For independent calls use fp::batch. A refusal lists every problem in the request."#;
// Worked example (fetch → get → take → state::set) lives in the crate README. shell::*/coder::*
// steps need the harness-stamped fs_scope and database writes are refused while
// `approval::gate` is registered — both surface as step errors.

/// ponytail: remaining ceilings — no nested pipes, no observer-control,
/// session-write or approval steps, 120s per bus step, and a held (approval) release
/// re-runs steps from scratch is unsupported: the pipe call itself is what
/// approvers review. shell::*/coder::* steps ride the harness-forwarded
/// fs_scope stamp (see `bus_step_args`) and are refused without one.
const MAX_STEPS: usize = 12;
const DEFAULT_PREVIEW_CHARS: usize = 400;
/// Hard cap on `preview_chars`: the receipt is a glimpse (or a slice to
/// reason over), never the bulk channel it exists to replace — an unbounded
/// u32 would let one call ship an entire fetched document back into the chat.
const MAX_PREVIEW_CHARS: usize = 8_000;
/// Per-step bus budget. The WHOLE pipe must also fit the caller's own
/// dispatch timeout on `fp::pipe`.
const STEP_TIMEOUT_MS: u64 = 120_000;
/// The read-only session-manager surface — every `session::*` id that
/// iii-permissions.yaml leaves at the needs_approval default instead of
/// hard-denying. The only `session::*` steps a pipe accepts.
const SESSION_READS: &[&str] = &[
    "session::get",
    "session::list",
    "session::messages",
    "session::messages-range",
    "session::messages-tail",
    "session::get-message",
    "session::get-attachment",
    "session::list-attachments",
];

/// The metadata reads of the secrets worker (masked hints, fingerprints,
/// never a value). The only `secrets::` steps a pipe accepts: the rest grant
/// access, resolve a value or change a credential.
const SECRETS_READS: &[&str] = &[
    "secrets::list",
    "secrets::get",
    "secrets::status",
    "secrets::detect",
];

/// The router's catalog reads: they spend nothing, return no credential
/// value, and iii-permissions.yaml does not deny them. The only `router::`
/// steps a pipe accepts.
const ROUTER_READS: &[&str] = &[
    "router::models::list",
    "router::models::get",
    "router::models::budget",
    "router::models::supports",
    "router::provider::list",
];

/// Calls the root iii-permissions.yaml denies that a step may still make, each
/// with its reason. The pipe or batch call is the approval surface a person
/// reviews. A test fails when an entry is no longer denied.
const ALLOWED_ON_PURPOSE: &[(&str, &str)] = &[
    (
        "state::set",
        "persisting the threaded value is what a pipe is for",
    ),
    (
        "state::update",
        "persisting the threaded value is what a pipe is for",
    ),
    (
        "state::delete",
        "clearing a stored value is part of persisting one",
    ),
    (
        "kanban::ticket::restore",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::project::update",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::project::archive",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::project::restore",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::release::create",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::release::update",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::release::ship",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::release::reopen",
        "a planning write an approved pipe may make",
    ),
    (
        "kanban::label::convert",
        "a planning write an approved pipe may make",
    ),
    // Gate-only: the database branch of `forbidden_unprefixed_step` refuses
    // these while the approval gate runs, before this table is read.
    (
        "database::on-secret-change",
        "gate-only: reloads stored connections; with no gate, no per-agent decision exists to bypass",
    ),
    (
        "database::saveConnection",
        "gate-only: writes a stored connection; with no gate, no per-agent decision exists to bypass",
    ),
    (
        "database::deleteConnection",
        "gate-only: deletes a stored connection; with no gate, no per-agent decision exists to bypass",
    ),
    (
        "database::getConnection",
        "gate-only: reads a stored connection; with no gate, no per-agent decision exists to bypass",
    ),
    (
        "database::testConnection",
        "gate-only: tests a stored connection; with no gate, no per-agent decision exists to bypass",
    ),
    (
        "database::mention::search",
        "gate-only: lists tables for chat mentions; with no gate, no per-agent decision exists to bypass",
    ),
    (
        "database::mention::get",
        "gate-only: reads a table for a chat mention; with no gate, no per-agent decision exists to bypass",
    ),
];

// NOTE: deliberately NOT `#[serde(deny_unknown_fields)]`. The engine injects
// a `_caller_worker_id` field into every dispatched call's payload (see
// ide/src/functions/types.rs for the same note), so a strict top-level
// struct would reject EVERY live call. `PipeStep` below stays strict — the
// engine never injects inside nested values, and a typo'd step field failing
// loudly is teachable. (A `//` comment on purpose: a `///` doc comment
// becomes the published schema description, leaking engine plumbing to every
// model that reads engine::functions::info.)
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PipeRequest {
    /// Steps run in order; step N+1 receives step N's threaded value.
    pub through: Vec<PipeStep>,
    /// Preview length of the final threaded value in the receipt (capped at
    /// 8000).
    #[serde(default)]
    pub preview_chars: Option<u32>,
    // Trusted filesystem scope stamped by the harness onto the pipe call
    // (harness/src/filesystem_scope.rs); forwarded opaquely onto each scoped
    // step by `bus_step_args`. Never model-supplied: the harness overwrites or
    // strips it before dispatch. (`//` on purpose — a `///` doc comment would
    // publish this plumbing field in the fp::pipe schema.)
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PipeStep {
    /// Function id to call.
    pub function: String,
    /// Base payload for the call.
    #[serde(default)]
    pub payload: Option<Value>,
    /// JSON pointer in this step's payload where the previous value lands (default
    /// `/value`); object keys only, no array indexing.
    // `~0`/`~1` escapes are decoded.
    #[serde(default)]
    pub into: Option<String>,
    /// Namespace of the target function (default: this worker's; say
    /// `default` for the engine's).
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StepReceipt {
    pub function: String,
    /// Size of the value threaded OUT of this step (chars of its string or
    /// serialized form).
    pub chars: usize,
    /// Set when the threaded value REPLACED a non-null literal already in this
    /// step's payload at the `into` pointer — almost always an omitted `into`
    /// on a write step clobbering the value it meant to write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PipeResponse {
    pub steps: Vec<StepReceipt>,
    pub value_preview: Option<String>,
    /// True when a `fp::when` guard failed and stopped the pipe: the receipts
    /// end at the guard and NO later step ran. Absent (false) on a full run.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub short_circuited: bool,
}

/// Mirror of harness `filesystem_scope::is_scoped_function` — keep
/// byte-identical, or a function the harness stamps but fp doesn't (or vice
/// versa) becomes a scope gap. `shell::filesystem::*` is the unscoped
/// control plane on both sides.
pub(crate) fn is_scoped_step(function_id: &str) -> bool {
    (function_id.starts_with("shell::") && !function_id.starts_with("shell::filesystem::"))
        || function_id.starts_with("coder::")
}

/// The namespace a call names. A blank one is no namespace: the SDK refuses
/// to send one, so validation and the call must both drop it.
pub(crate) fn named_namespace(namespace: Option<&str>) -> Option<&str> {
    namespace.filter(|ns| !ns.trim().is_empty())
}

/// The namespace a call names when it is not `own`.
pub(crate) fn other_namespace<'a>(namespace: Option<&'a str>, own: &str) -> Option<&'a str> {
    named_namespace(namespace).filter(|ns| *ns != own)
}

/// What `register_worker` resolves when, as in main.rs, no namespace option is set.
pub(crate) fn env_namespace() -> String {
    std::env::var("III_NAMESPACE")
        .ok()
        .filter(|ns| !ns.trim().is_empty())
        .unwrap_or_else(|| "default".into())
}

// shortcut: mesh ids carry no machine prefix since machines are namespaces, so
// this strip matches only in front of a mesh older than that; delete it and its
// branch in bus_step_args one release after the mesh release (follow-up ticket).
pub(crate) fn remote_scoped_step(function_id: &str) -> bool {
    !is_scoped_step(function_id)
        && function_id
            .match_indices("::")
            .any(|(index, _)| is_scoped_step(&function_id[index + 2..]))
}

/// Step functions a pipe or batch refuses. Steps run with THIS worker's
/// authority, so every call the root iii-permissions.yaml denies is refused
/// here too — an allowlisted or approve-always `fp::pipe` would otherwise be a
/// standing policy bypass. No process loads that file at runtime: it is the
/// shipped agent policy, and a test keeps this hand list in line with it.
/// `ALLOWED_ON_PURPOSE` lists the denied calls a step may still make, each
/// with its reason; the pipe call itself is the approval surface an approver
/// reviews. Operator rules added at runtime are not seen. Note that
/// the same reasoning makes allowlisting `fp::pipe` grant un-approved
/// shell::*/coder::* execution (scoped steps, gated in `validate`) — the
/// pipe must stay needs_approval. Residual, accepted: an rbac boundary that
/// exposes fp::pipe while denying shell::* could launder a scope through a
/// non-harness call; shell-side caller verification is tracked follow-up.
pub(crate) fn forbidden_step(
    function_id: &str,
    approval_gate_running: bool,
) -> Option<&'static str> {
    // Every `::` offset, overlapping ones too: in `a:::x::y` the tail `x::y`
    // starts after the second `::`, which `match_indices` would skip.
    let ids = std::iter::once(function_id).chain(
        function_id
            .as_bytes()
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| *pair == b"::")
            .map(|(index, _)| &function_id[index + 2..]),
    );
    for id in ids {
        // A catalog read ends the scan: the `provider::list` tail of
        // `router::provider::list` is not a provider call.
        if ROUTER_READS.contains(&id) {
            return None;
        }
        if let Some(reason) = forbidden_unprefixed_step(id, approval_gate_running) {
            return Some(reason);
        }
    }
    None
}

fn forbidden_unprefixed_step(
    function_id: &str,
    approval_gate_running: bool,
) -> Option<&'static str> {
    // Private accessors validate their scope, not the caller. A pipe must not
    // turn an agent's request into a privileged read or namespace claim.
    if function_id == "state::claim-namespace"
        || function_id.starts_with("provider-openai-codex::state::")
    {
        return Some("operator-only state functions are not supported in a pipe");
    }
    if function_id.starts_with("mcp-client::") {
        return Some(
            "operator-only MCP client functions are not supported in a pipe — steps run with \
             worker authority, which would bypass the agent-level deny on them",
        );
    }
    // Shell control-plane ids sit inside the scoped shell::* prefix but are
    // NOT session tools: config-status is agent-policy hard-denied (it can
    // surface operator paths), kill-session-jobs is the harness::stop sweep
    // (it reaps any session's jobs) and workspace::* is console picker
    // plumbing — all ignore fs_scope, so the stamp gate below would admit them.
    if function_id == "shell::config-status"
        || function_id == "shell::kill-session-jobs"
        || function_id.starts_with("shell::workspace::")
    {
        return Some(
            "shell control-plane functions are not supported in a pipe — steps run with \
             worker authority, which would bypass the agent-level deny on them",
        );
    }
    if function_id == crate::batch::BATCH_ID {
        return Some("a batch is not a pipe step — call fp::batch directly");
    }
    if function_id == PIPE_ID {
        return Some(
            "pipes do not nest — to start from a literal value, seed the FIRST transform \
             step's payload (\"value\": …) instead of wrapping the value in a pipe step",
        );
    }
    if function_id == "engine::observers::dispose" {
        return Some("observer control calls are not supported in a pipe — call them directly");
    }
    // Session READS are only needs_approval for agents (the policy hard-denies
    // the writes and the raw `session::store::*` protocol), and the pipe call
    // is that approval surface — so, like `database::query`, they may ride a
    // pipe. Writes and approval::* stay refused.
    if (function_id.starts_with("session::") && !SESSION_READS.contains(&function_id))
        || function_id.starts_with("approval::")
    {
        return Some(
            "only read-only session functions (session::get/list/messages*/get-message/\
             get-attachment/list-attachments) are supported in a pipe — session writes and \
             approval functions run with worker authority, which would bypass the agent-level \
             deny on them",
        );
    }
    if function_id.starts_with("secrets::") && !SECRETS_READS.contains(&function_id) {
        return Some(
            "only secrets::list/get/status/detect are supported in a pipe — other secrets \
             functions would run with worker authority, past the agent-level deny on them",
        );
    }
    if function_id.starts_with("configuration::")
        || function_id.starts_with("oauth::")
        || matches!(
            function_id,
            "email::config-status"
                | "claude::auth::status"
                | "pi::auth::status"
                | "storage::getBucket"
                | "storage::saveBucket"
                | "storage::deleteBucket"
                | "storage::testBucket"
        )
    {
        return Some(
            "credential/config functions are not supported in a pipe — steps run with worker \
             authority, which would bypass the agent-level deny on them",
        );
    }
    if function_id.starts_with("router::") || function_id.starts_with("provider::") {
        return Some(
            "LLM routing/provider functions are not supported in a pipe (model spend stays in \
             the harness loop); router::models::list/get/budget/supports and \
             router::provider::list are allowed",
        );
    }
    if function_id.starts_with("harness::")
        || function_id.starts_with("run::")
        || matches!(function_id, "context::assemble" | "context::compact")
    {
        return Some(
            "harness/turn-control functions are not supported in a pipe — steps run with \
             worker authority, which would bypass the agent-level deny on them",
        );
    }
    // The harness hook stamps the caller's session and folder onto a direct
    // workflow::start; a step skips it, so a forged folder would be trusted.
    if function_id == "workflow::start" {
        return Some(
            "workflow::start is not supported in a pipe — steps run with worker authority, \
             which would skip the caller stamp the pre-call hook puts on it",
        );
    }
    // The same stamp names the node a workflow-result::submit delivers for; a
    // step could deliver another node's result.
    if function_id == "workflow-result::submit" {
        return Some(
            "workflow-result::submit is not supported in a pipe — steps run with worker \
             authority, which would skip the caller stamp that names the submitting node",
        );
    }
    // `stream::*` is the deprecated iii-stream surface. Engines still host it,
    // so keep refusing it here until the builtin is removed.
    if function_id.starts_with("stream::")
        || function_id.starts_with("hook-fanout::")
        || function_id.starts_with("iii::")
        || function_id.starts_with("mesh-node::")
        || matches!(
            function_id,
            "mesh::auth"
                | "mesh::middleware"
                | "mesh::on-observer-registration"
                | "mesh::on-config-change"
                | "mesh::on-functions-change"
                | "mesh::on-node-changed"
                | "mesh::on-status-changed"
                | "mesh::on-tunnel-change"
                | "mesh::ui-content"
                | "mesh::configuration-id"
                // It calls any function_id later with the queue worker's
                // authority, so a step could reach what this list refuses.
                | "engine::queue::enqueue"
        )
    {
        return Some("bus-internal functions are not supported in a pipe");
    }
    // With an approval gate, only the read-only `database::query` may ride a
    // pipe. Every other `database::*` can WRITE with THIS worker's authority,
    // bypassing the gate's per-agent decision.
    if approval_gate_running
        && function_id.starts_with("database::")
        && function_id != "database::query"
    {
        return Some(
            "only database::query (read-only) is supported in a pipe — writes, transactions, \
             and prepared statements run with worker authority, which would bypass the \
             agent-level deny on them; call them directly",
        );
    }
    // After the database branch: its table entries are gate-only.
    if ALLOWED_ON_PURPOSE.iter().any(|(id, _)| *id == function_id) {
        return None;
    }
    // Event hooks: `on-config-change`, `on-secret-change`, `on_secret_changed`, …
    // Only after a `::`, or the bare tail of a gate-only database hook would
    // refuse it.
    if function_id
        .rsplit_once("::")
        .is_some_and(|(_, last)| last.starts_with("on-") || last.starts_with("on_"))
    {
        return Some("lifecycle hooks are engine-dispatched, never pipe steps");
    }
    if function_id.contains("::terminal::")
        || function_id.starts_with("shell::pty::")
        || function_id.starts_with("shell::ui-state::")
        || function_id.ends_with("::ui-content")
        || matches!(
            function_id,
            "stories::ui-file"
                | "browser::pick::start"
                | "browser::pick::resolve"
                | "browser::pick::stop"
                | "browser::pick::hint"
                | "browser::screencast::start"
                | "browser::screencast::stop"
                | "browser::frame"
                | "computer::screencast::start"
                | "computer::screencast::stop"
                | "computer::frame"
                | "ios-simulator::touch"
                | "ios-simulator::watch"
                | "ios-simulator::frame"
                | "a2ui::action"
                | "a2ui::binding::apply"
                | "a2ui::stamp-session"
                | "slack::interactions"
        )
    {
        return Some(
            "terminal and console UI functions are not supported in a pipe — steps run with \
             worker authority, which would bypass the agent-level deny on them",
        );
    }
    if function_id.starts_with("shell::scm::")
        || function_id.starts_with("github::webhooks::")
        || function_id.starts_with("ade::conversations::")
        || matches!(
            function_id,
            "directory::pre-generate"
                | "directory::hint-preview"
                | "kanban::ticket::delete"
                | "kanban::release::delete"
                | "eval::step"
                | "eval::sweep"
                | "security-scan::execute"
                | "sentinel::ingest"
                | "ios-simulator::tenants::list"
                | "worktree::land-step"
                | "slack::events"
                | "openwiki::cron::refresh-due"
        )
    {
        return Some(
            "the agent policy denies this function, so it is not supported in a pipe — steps \
             run with worker authority, which would bypass that deny",
        );
    }
    None
}

/// What a running gate refuses in a call that its absence admits: a database
/// write, or a call in another namespace (the approver sees `via` in this
/// worker's namespace only, so a call elsewhere would run unapproved there).
/// Empty when the gate's state decides nothing about the call.
pub(crate) fn gate_refusals(
    function: &str,
    namespace: Option<&str>,
    own: &str,
    via: &str,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if forbidden_step(function, false).is_none()
        && let Some(reason) = forbidden_step(function, true)
    {
        reasons.push(reason.to_string());
    }
    if let Some(ns) = other_namespace(namespace, own) {
        reasons.push(format!(
            "a call in namespace {ns} must be approved in that namespace — call {function} \
             directly with \"namespace\": \"{ns}\" instead of through {via}"
        ));
    }
    reasons
}

/// Static validation, separated from execution for testability. Every
/// problem in the request is collected before anything is refused, so one
/// corrected resend is enough.
fn validate_with_approval_gate(
    req: &PipeRequest,
    approval_gate_running: bool,
    own_namespace: &str,
) -> Result<(), String> {
    let mut problems = Vec::new();
    if req.through.is_empty() || req.through.len() > MAX_STEPS {
        problems.push(format!("a pipe takes 1..={MAX_STEPS} steps"));
    }
    for (i, step) in req.through.iter().enumerate() {
        let gated = if approval_gate_running {
            gate_refusals(
                &step.function,
                step.namespace.as_deref(),
                own_namespace,
                PIPE_ID,
            )
        } else {
            Vec::new()
        };
        for reason in forbidden_step(&step.function, false)
            .map(str::to_string)
            .into_iter()
            .chain(gated)
        {
            problems.push(format!("step {} ({}): {reason}", i + 1, step.function));
        }
        if is_scoped_step(&step.function) {
            if req.fs_scope.is_none() {
                problems.push(format!(
                    "step {} ({}): filesystem-scoped steps (shell::*/coder::*) run only under a \
                     harness-stamped filesystem scope, and this call carries none (no session \
                     working directory, or a non-harness caller) — call the function directly",
                    i + 1,
                    step.function
                ));
            } else {
                // Loud static refusal beats a silent overwrite by the stamp in
                // `bus_step_args` — the pointer can only be a mistake or an attack.
                let first_token = step
                    .into
                    .as_deref()
                    .and_then(|p| p.strip_prefix('/'))
                    .and_then(|p| p.split('/').next())
                    .map(|t| t.replace("~1", "/").replace("~0", "~"));
                if first_token.as_deref() == Some("fs_scope") {
                    problems.push(format!(
                        "step {} ({}): `into` must not target `fs_scope` — that field is \
                         harness-stamped, never threaded",
                        i + 1,
                        step.function
                    ));
                }
            }
        }
    }
    // Fail fast on an unseeded leading transform (live-tested trap: serde's
    // bare "missing field `value`" taught nothing). Only our own transforms
    // are statically checkable; a foreign first step just runs.
    if let Some(first) = req.through.first()
        && util::is_transform(&first.function)
        && first
            .payload
            .as_ref()
            .and_then(|p| p.get("value"))
            .is_none()
    {
        problems.push(format!(
            "step 1 ({}) receives no threaded input — seed it in the first step's payload \
                 ({{\"value\": …}}), or start the pipe with a producing function (a fetch, a \
                 state::get)",
            first.function
        ));
    }
    refused("pipe", problems)
}

const MAX_LISTED_PROBLEMS: usize = 32;

/// Longest text echoed for one listed problem. Problems carry caller-chosen
/// function ids, so each item is cut to this length to keep a refusal bounded.
const MAX_PROBLEM_CHARS: usize = 240;

/// The one refusal for a request: the count first, then the first problems, so
/// the caller can fix them in a single resend. The count covers every problem;
/// the listing stops at `MAX_LISTED_PROBLEMS` so an oversized request cannot
/// produce an unbounded refusal.
pub(crate) fn refused(kind: &str, problems: Vec<String>) -> Result<(), String> {
    if problems.is_empty() {
        return Ok(());
    }
    let total = problems.len();
    let mut listed: Vec<String> = problems
        .into_iter()
        .take(MAX_LISTED_PROBLEMS)
        .map(bounded_problem)
        .collect();
    if total > MAX_LISTED_PROBLEMS {
        listed.push(format!("and {} more", total - MAX_LISTED_PROBLEMS));
    }
    Err(format!(
        "{kind} refused, {total} problem(s): {}",
        listed.join("; ")
    ))
}

/// Cuts one listed problem to `MAX_PROBLEM_CHARS` characters, marking the cut.
fn bounded_problem(problem: String) -> String {
    if problem.chars().count() <= MAX_PROBLEM_CHARS {
        return problem;
    }
    let mut cut: String = problem.chars().take(MAX_PROBLEM_CHARS).collect();
    cut.push_str("...");
    cut
}

pub fn validate(req: &PipeRequest) -> Result<(), String> {
    validate_with_approval_gate(req, true, &env_namespace())
}

/// The gate state for a pipe only the gate's answer splits. An unknown state
/// refuses a step in another namespace, as `fp::batch` does; a pipe whose
/// only gated step is a database write keeps the lookup error.
fn split_gate(req: &PipeRequest, own: &str, lookup: Result<bool, String>) -> Result<bool, String> {
    match lookup {
        Err(_)
            if req
                .through
                .iter()
                .any(|s| other_namespace(s.namespace.as_deref(), own).is_some()) =>
        {
            Ok(true)
        }
        lookup => lookup,
    }
}

pub(crate) async fn approval_gate_running(iii: &IIIClient) -> Result<bool, String> {
    // Presence must come from a signal RBAC cannot turn into a false
    // "absent": `functions::list` rows are RBAC-filtered per session, so a
    // gate hidden from THIS worker's session reads as unregistered and
    // writes ride through. `functions::info` checks RBAC BEFORE existence —
    // NOT_FOUND is only reachable when the session may see the id.
    match iii
        .call(CallRequest {
            function_id: "engine::functions::info".into(),
            payload: json!({ "function_id": "approval::gate" }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
    {
        Ok(_) => Ok(true),
        Err(e) => gate_presence_from_error(&e),
    }
}

/// The engine's "no namespace holds it" answer is the ONLY one that unlocks
/// database writes. Every other NOT_FOUND (the bare id held by two or more
/// non-default namespaces and not by `default`: "is ambiguous ...") means a
/// gate runs somewhere, and fp cannot see its caller's namespace: present.
/// FORBIDDEN — the id RBAC-hidden, or the info call itself denied — means
/// presence is unknowable from here: treat the gate as present (fail closed).
/// Anything else is infrastructure trouble, surfaced rather than converted to
/// policy.
fn gate_presence_from_error(error: &iii_sdk::Error) -> Result<bool, String> {
    match error {
        iii_sdk::Error::Remote { code, message, .. } if code == "NOT_FOUND" => {
            Ok(message != "Function 'approval::gate' is not registered.")
        }
        iii_sdk::Error::Remote { code, .. } if code == "FORBIDDEN" => Ok(true),
        e => Err(format!("could not inspect approval-gate availability: {e}")),
    }
}

/// Set `value` at `pointer` inside `payload`, creating intermediate objects.
fn inject(payload: Value, pointer: &str, value: Value) -> Result<Value, String> {
    // RFC 6901 token unescape (`~1` → `/`, then `~0` → `~`). Object keys
    // only — a token that lands on an array errors below instead of indexing.
    let parts: Vec<String> = pointer
        .strip_prefix('/')
        .map(|p| {
            p.split('/')
                .map(|t| t.replace("~1", "/").replace("~0", "~"))
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(format!(
            "`into` must be a JSON pointer like \"/value\", got {pointer:?}"
        ));
    }
    let mut root = match payload {
        Value::Object(m) => Value::Object(m),
        Value::Null => json!({}),
        other => {
            return Err(format!(
                "step payload must be an object to receive `into`, got {}",
                util::kind_of(&other)
            ));
        }
    };
    let mut cursor = &mut root;
    for part in &parts[..parts.len() - 1] {
        let map = cursor.as_object_mut().expect("object cursor");
        cursor = map.entry(part.clone()).or_insert_with(|| json!({}));
        if !cursor.is_object() {
            return Err(format!(
                "`into` path {pointer:?} crosses a non-object at {part:?}"
            ));
        }
    }
    let map = cursor.as_object_mut().expect("object cursor");
    map.insert(parts[parts.len() - 1].clone(), value);
    Ok(root)
}

/// Dispatch-ready args for a bus step. The trusted scope stamps LAST — after
/// `into` threading — so nothing authored or threaded survives at `/fs_scope`
/// on a scoped step (full key replace: subkeys can't survive either).
/// Non-scoped steps pass through untouched — the scope must not leak to
/// arbitrary workers. Remote scoped steps lose any model-supplied scope;
/// the remote worker applies its own filesystem boundary.
pub(crate) fn bus_step_args(
    function_id: &str,
    args: Value,
    fs_scope: Option<&Value>,
) -> Result<Value, String> {
    if remote_scoped_step(function_id) {
        let Value::Object(mut map) = args else {
            return Ok(args);
        };
        map.remove("fs_scope");
        return Ok(Value::Object(map));
    }
    if !is_scoped_step(function_id) {
        return Ok(args);
    }
    // unreachable after validate(); fail closed anyway
    let Some(scope) = fs_scope else {
        return Err("filesystem-scoped step without a harness fs_scope stamp".into());
    };
    let Value::Object(mut map) = args else {
        return Err("a filesystem-scoped step's payload must be an object".into());
    };
    map.insert("fs_scope".into(), scope.clone());
    Ok(Value::Object(map))
}

/// Requested preview length, clamped to [0, MAX_PREVIEW_CHARS].
fn preview_len(requested: Option<u32>) -> usize {
    requested
        .map(|c| c as usize)
        .unwrap_or(DEFAULT_PREVIEW_CHARS)
        .min(MAX_PREVIEW_CHARS)
}

pub(crate) fn chars_of(value: &Value) -> usize {
    match value {
        Value::String(s) => s.chars().count(),
        other => other.to_string().chars().count(),
    }
}

/// What a stream channel handle's `access_key` becomes in text fp returns
/// (the harness's note for the handle objects it withholds, MOT-5827).
const WITHHELD_ACCESS_KEY: &str = "<withheld: a turn cannot read a stream channel. For a file's \
     text call coder::read-file, or in a sandbox cat it with sandbox::exec>";

/// Push the `access_key` of each stream channel handle (an object with string
/// `channel_id` and `access_key`, as `StreamChannelRef`) in `value`.
pub(crate) fn stream_access_keys(value: &Value, keys: &mut Vec<String>) {
    if value.get("channel_id").is_some_and(Value::is_string)
        && let Some(Value::String(key)) = value.get("access_key")
        && !key.is_empty()
    {
        keys.push(key.clone());
    }
    match value {
        Value::Object(map) => map.values().for_each(|v| stream_access_keys(v, keys)),
        Value::Array(items) => items.iter().for_each(|v| stream_access_keys(v, keys)),
        _ => {}
    }
}

/// `text` with each of `keys` replaced by [`WITHHELD_ACCESS_KEY`]. Run before
/// a value's text leaves fp: the harness withholds handle objects but cannot
/// look inside a string (MOT-5833). Matching the key's text, not the handle's
/// shape, also catches a key a step moved out of its handle (fp::get, omit).
/// shortcut: only the whole key is matched (engine keys are UUIDs, which JSON
/// never escapes), so a step that cuts a key (fp::take, fp::split) shows its
/// parts; withhold in those transforms if a pipe ever needs to cut one.
pub(crate) fn withhold_access_keys(text: String, keys: &[String]) -> String {
    if keys.is_empty() {
        return text;
    }
    // One pass, longest key first: a key that holds another is never cut into
    // pieces, and the inserted notes are never scanned again.
    match AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostLongest)
        .build(keys)
    {
        Ok(keys) => keys.replace_all(&text, &vec![WITHHELD_ACCESS_KEY; keys.patterns_len()]),
        // Fail closed: text that could not be searched never leaves fp.
        Err(_) => WITHHELD_ACCESS_KEY.to_string(),
    }
}

fn preview_of(value: &Value, max_chars: usize, access_keys: &[String]) -> String {
    let s = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    // Before the cut, so a cut never shows part of a key.
    let s = withhold_access_keys(s, access_keys);
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => format!("{}…", &s[..byte_idx]),
        None => s,
    }
}

/// A step failure carries the receipts trail so the caller sees how far the
/// pipe got and at what sizes, without the values themselves. `i` is the
/// 0-based loop index; the message is 1-based to match the console's step
/// list (live-tested: 0-based "failed at step 2" next to a 2-step completed
/// trail read as "the 2nd step failed" when it was the 3rd).
fn step_error(i: usize, function: &str, msg: &str, receipts: &[StepReceipt]) -> String {
    let n = i + 1;
    let trail = receipts
        .iter()
        .map(|r| format!("{}→{}ch", r.function, r.chars))
        .collect::<Vec<_>>()
        .join(" · ");
    if trail.is_empty() {
        format!("pipe failed at step {n} ({function}): {msg}")
    } else {
        format!("pipe failed at step {n} ({function}): {msg} · completed: {trail}")
    }
}

/// A bus step's call, in the namespace the step names (none: this worker's).
fn step_request(step: &PipeStep, args: Value) -> CallRequestWithMetadata {
    let request = CallRequest {
        function_id: step.function.clone(),
        payload: args,
        action: None,
        timeout_ms: Some(STEP_TIMEOUT_MS),
    };
    match named_namespace(step.namespace.as_deref()) {
        Some(namespace) => request.namespace(namespace),
        None => request.into(),
    }
}

/// Execute a pipe: transforms inline, everything else over the bus. The first
/// failing step stops the pipe with a teachable error carrying the trail.
pub async fn run(iii: &IIIClient, req: PipeRequest) -> Result<PipeResponse, String> {
    let own = iii.namespace().unwrap_or_else(|| "default".into());
    // Gate discovery is a bus round trip that couples every pipe to
    // engine::functions::info availability — pay it only when the gate
    // actually decides the verdict. Both static verdicts are computed first
    // (12 steps max, trivially cheap); agreeing verdicts are final as-is.
    match (
        validate_with_approval_gate(&req, false, &own),
        validate_with_approval_gate(&req, true, &own),
    ) {
        (Ok(()), Ok(())) => {}
        (Err(_), Err(_)) => {
            // The gate only decides database writes and steps in another
            // namespace, so the complete list needs its real state when the
            // pipe holds one. Pipes without one stay off the bus, and an
            // unknown state fails closed.
            let gate = if req.through.iter().any(|s| {
                (s.function.starts_with("database::") && s.function != "database::query")
                    || other_namespace(s.namespace.as_deref(), &own).is_some()
            }) {
                approval_gate_running(iii).await.unwrap_or(true)
            } else {
                false
            };
            validate_with_approval_gate(&req, gate, &own)?;
        }
        _ => {
            let gate = split_gate(&req, &own, approval_gate_running(iii).await)?;
            validate_with_approval_gate(&req, gate, &own)?
        }
    }
    let preview_chars = preview_len(req.preview_chars);

    let mut receipts: Vec<StepReceipt> = Vec::with_capacity(req.through.len());
    let mut piped: Option<Value> = None;
    let mut short_circuited = false;
    // Every handle key a step returned, so the preview withholds it however
    // later steps reshape the handle.
    let mut access_keys = Vec::new();

    for (i, step) in req.through.iter().enumerate() {
        let mut args = step.payload.clone().unwrap_or_else(|| json!({}));
        let mut clobber_note = None;
        if let Some(value) = piped.take() {
            let pointer = step.into.as_deref().unwrap_or("/value");
            // Injection REPLACES whatever sits at the pointer. Overwriting a
            // literal the author spelled out is almost always an omitted
            // `into` on a write step — flag it on the receipt so the clobber
            // is visible instead of silent. Empty containers don't count: a
            // `{}`/`[]` placeholder is padding, not an authored literal.
            let occupied = args.pointer(pointer).is_some_and(|v| match v {
                Value::Null => false,
                Value::Object(m) => !m.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            });
            if occupied {
                clobber_note = Some(format!(
                    "piped value REPLACED the literal at `{pointer}` — aim `into` at a \
                     subfield (e.g. `{pointer}/_piped`) to keep your literal payload"
                ));
            }
            args = inject(args, pointer, value)
                .map_err(|msg| step_error(i, &step.function, &msg, &receipts))?;
        }

        // fp::when is the pipe's guard, dispatched here rather than through
        // util::apply because a failing guard must STOP the pipe (that is its
        // whole purpose — a trailing write step must not run). A passing
        // guard threads the ORIGINAL value onward unchanged.
        if step.function == util::WHEN_ID {
            let (passed, value) = util::eval_when(&args)
                .map_err(|msg| step_error(i, &step.function, &msg, &receipts))?;
            receipts.push(StepReceipt {
                function: step.function.clone(),
                chars: chars_of(&value),
                note: clobber_note,
            });
            stream_access_keys(&value, &mut access_keys);
            piped = Some(value);
            if !passed {
                short_circuited = true;
                break;
            }
            continue;
        }

        let value = match util::apply(&step.function, &args) {
            Some(result) => result.map_err(|msg| step_error(i, &step.function, &msg, &receipts))?,
            None => {
                // The gate can register while an earlier step runs, so a step
                // its state decides reads it again, mapped as at admission
                // (`split_gate`); other steps skip the lookup.
                // shortcut: the gate can still register between this read and
                // the call; only the engine can close that, if it ever matters.
                let gated = gate_refusals(&step.function, step.namespace.as_deref(), &own, PIPE_ID);
                if !gated.is_empty()
                    && split_gate(&req, &own, approval_gate_running(iii).await)
                        .map_err(|e| step_error(i, &step.function, &e, &receipts))?
                {
                    return Err(step_error(i, &step.function, &gated.join("; "), &receipts));
                }
                // SECURITY: the stamp must be the LAST write to args before
                // dispatch — after `into` threading above — or a threaded
                // value at /fs_scope would reach the shell worker as trusted.
                let args = bus_step_args(&step.function, args, req.fs_scope.as_ref())
                    .map_err(|msg| step_error(i, &step.function, &msg, &receipts))?;
                iii.call(step_request(step, args))
                    .await
                    .map_err(|e| step_error(i, &step.function, &e.to_string(), &receipts))?
            }
        };

        receipts.push(StepReceipt {
            function: step.function.clone(),
            chars: chars_of(&value),
            note: clobber_note,
        });
        stream_access_keys(&value, &mut access_keys);
        piped = Some(value);
    }

    let final_value = piped.unwrap_or(Value::Null);
    // Receipts + preview only, matching PipeResponse — a step's raw result
    // (e.g. state::set echoing the stored value) must never ride along, or
    // the receipt grows to the size of the value it exists to avoid.
    Ok(PipeResponse {
        value_preview: Some(preview_of(&final_value, preview_chars, &access_keys)),
        steps: receipts,
        short_circuited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> Result<PipeRequest, String> {
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    #[test]
    fn tolerates_the_engine_injected_caller_worker_id() {
        // The engine stamps `_caller_worker_id` into every dispatched payload
        // (see ide/src/functions/types.rs) — this is the byte-shape of the
        // live request a stale deny_unknown_fields build rejected 4× in a row
        // (2026-07-16 research-pipeline session), forcing an 18KB
        // bulk-through-chat fallback:
        let req = parse(json!({
            "preview_chars": 300,
            "through": [
                { "function": "browser::fetch",
                  "payload": { "url": "https://en.wikipedia.org/wiki/Circuit_breaker_design_pattern",
                               "format": "markdown", "main_content_only": true } },
                { "function": "fp::get", "payload": { "path": "/content" } },
                { "function": "fp::take", "payload": { "n": 6000 } },
                { "function": "state::set", "into": "/value/content",
                  "payload": { "scope": "research", "key": "article",
                               "value": { "url": "https://en.wikipedia.org/wiki/Circuit_breaker_design_pattern",
                                          "title": "Circuit Breaker Design Pattern",
                                          "content": "" } } },
            ],
            "_caller_worker_id": "harness",
        }))
        .unwrap();
        assert_eq!(req.through.len(), 4);
        assert!(validate(&req).is_ok());

        // ...while STEP-level strictness stays: the engine never injects
        // inside nested values, so a typo'd step field still fails loudly.
        assert!(
            parse(json!({ "through": [{ "function": "a::b", "args": {} }] }))
                .unwrap_err()
                .contains("args")
        );
    }

    #[test]
    fn unseeded_leading_transform_fails_fast_and_teachably() {
        // The exact live shape that failed with serde's bare "missing field
        // `value`": a transform first, its seed nowhere.
        let req = parse(json!({ "through": [
            { "function": "fp::split", "payload": { "separator": "," } },
            { "function": "fp::uniq" },
        ]}))
        .unwrap();
        let err = validate(&req).unwrap_err();
        // step indices are 1-based in every message, matching the console
        assert!(err.contains("step 1") && err.contains("no threaded input"));
        assert!(err.contains("producing function"));

        // seeded leading transform passes (explicit null seed counts: the
        // transform itself decides whether null is workable)
        for seed in [json!("a,b"), Value::Null] {
            let req = parse(json!({ "through": [
                { "function": "fp::split", "payload": { "separator": ",", "value": seed } },
            ]}))
            .unwrap();
            assert!(validate(&req).is_ok());
        }

        // a producing first step is not ours to check
        let req = parse(json!({ "through": [
            { "function": "browser::fetch", "payload": { "url": "u" } },
            { "function": "fp::uniq" },
        ]}))
        .unwrap();
        assert!(validate(&req).is_ok());
    }

    #[test]
    fn validate_checks_shape_and_forbidden_steps() {
        // fetch → get → take → set: the motivating pipeline parses, zero `into`.
        let req = parse(json!({ "through": [
            { "function": "browser::fetch", "payload": { "url": "u", "format": "markdown" } },
            { "function": "fp::get", "payload": { "path": "/content" } },
            { "function": "fp::take", "payload": { "n": 20000 } },
            { "function": "state::set", "payload": { "scope": "s", "key": "k" } },
        ]}))
        .unwrap();
        assert_eq!(req.through.len(), 4);
        assert!(req.through.iter().all(|s| s.into.is_none()));
        assert!(validate(&req).is_ok());

        // unknown step fields fail loudly, not silently
        assert!(
            parse(json!({ "through": [
                { "function": "a::b", "pick": "/content" },
            ]}))
            .unwrap_err()
            .contains("pick")
        );

        // control / nested / worker-authority steps are refused with a
        // reason — mirroring every agent-policy hard-denied class
        // (iii-permissions.yaml), the ALLOWED_ON_PURPOSE ids excepted on purpose
        for (function, needle) in [
            ("shell::config-status", "control-plane"),
            ("shell::kill-session-jobs", "control-plane"),
            ("shell::workspace::roots", "control-plane"),
            ("fp::pipe", "nest"),
            ("engine::observers::dispose", "observer control"),
            ("session::append", "worker"),
            ("session::put-attachment", "worker"),
            ("session::store::get-attachment", "worker"),
            ("session::config-status", "worker"),
            ("approval::resolve", "worker"),
            ("configuration::get", "credential"),
            ("oauth::anthropic::login", "credential"),
            ("secrets::resolve", "secrets::list/get/status/detect"),
            ("secrets::on-changed", "secrets::list/get/status/detect"),
            ("router::chat", "routing"),
            ("provider::anthropic::chat", "routing"),
            ("harness::send", "turn-control"),
            ("run::start", "turn-control"),
            ("workflow::start", "caller stamp"),
            ("stream::set", "bus-internal"),
            ("iii::durable::publish", "bus-internal"),
            ("engine::queue::enqueue", "bus-internal"),
            ("storage::on-config-change", "lifecycle"),
            ("database::execute", "read-only"),
            ("database::transaction", "read-only"),
            ("database::transactionExecute", "read-only"),
            ("database::beginTransaction", "read-only"),
            ("database::runStatement", "read-only"),
            ("mcp-client::servers::approve", "operator-only"),
            ("mcp-client::login::start", "operator-only"),
            ("laptop::mcp-client::servers::approve", "operator-only"),
        ] {
            let req = parse(json!({ "through": [{ "function": function }] })).unwrap();
            assert!(validate(&req).unwrap_err().contains(needle), "{function}");
        }

        // …but the read-only query is the whole point of a validator pipe.
        let ok = parse(json!({ "through": [
            { "function": "database::query", "payload": { "db": "primary", "sql": "SELECT 1" } },
        ]}))
        .unwrap();
        assert!(validate(&ok).is_ok());

        // Only the operator namespace is refused: the tools are ordinary steps,
        // approved together with the pipe call that carries them.
        let mcp = parse(json!({ "through": [
            { "function": "mcp::linear::list_issues", "payload": {} },
        ]}))
        .unwrap();
        assert!(validate(&mcp).is_ok());

        // Only workflow::start is refused: reading or stopping a run is harmless.
        let wf = parse(json!({ "through": [
            { "function": "workflow::status", "payload": { "run_id": "r" } },
            { "function": "workflow::stop", "payload": { "run_id": "r" } },
        ]}))
        .unwrap();
        assert!(validate(&wf).is_ok());

        // Session READS are needs_approval, not hard-denied, so they ride a
        // pipe: pull an attachment out of the own session and thread it on.
        let reads = parse(json!({ "through": [
            { "function": "session::get-attachment",
              "payload": { "session_id": "s", "attachment_id": "a" } },
            { "function": "fp::get", "payload": { "path": "/data" } },
            { "function": "session::messages-tail", "payload": { "session_id": "s", "n": 5 } },
        ]}))
        .unwrap();
        assert!(validate(&reads).is_ok());

        let empty = parse(json!({ "through": [] })).unwrap();
        assert!(validate(&empty).is_err());
    }

    #[test]
    fn database_writes_are_rejected_only_with_an_approval_gate() {
        let req = parse(json!({ "through": [
            { "function": "database::execute",
              "payload": { "db": "primary", "sql": "INSERT INTO ledger VALUES (1)" } },
        ]}))
        .unwrap();

        assert!(validate_with_approval_gate(&req, true, "own").is_err());
        assert!(validate_with_approval_gate(&req, false, "own").is_ok());

        // `run` consults engine::functions::info only when the two verdicts
        // above differ — a pipe with no database write, even a read-only
        // database::query, validates identically both ways and never pays
        // the gate-discovery round trip.
        let query_only = parse(json!({ "through": [
            { "function": "database::query", "payload": { "db": "primary", "sql": "SELECT 1" } },
        ]}))
        .unwrap();
        assert!(validate_with_approval_gate(&query_only, true, "own").is_ok());
        assert!(validate_with_approval_gate(&query_only, false, "own").is_ok());

        // Only the engine's "registered nowhere" NOT_FOUND unlocks writes; a
        // NOT_FOUND naming the namespaces that hold the gate (two non-default
        // gates, none in `default`) and FORBIDDEN (RBAC hides the gate, or the
        // info call itself) fail closed as "present", and infrastructure
        // errors surface instead of deciding policy.
        let remote = |code: &str, message: &str| iii_sdk::Error::Remote {
            code: code.into(),
            message: message.into(),
            stacktrace: None,
        };
        let not_found = |message| gate_presence_from_error(&remote("NOT_FOUND", message));
        assert!(!not_found("Function 'approval::gate' is not registered.").unwrap());
        assert!(not_found(
            "Function 'approval::gate' is ambiguous: it is registered in namespace(s): proj, proj2. \
             Pass `namespace` to select one."
        )
        .unwrap());
        assert!(not_found("").unwrap());
        assert!(gate_presence_from_error(&remote("FORBIDDEN", "")).unwrap());
        // invoke-layer "target function missing" (old engine) is its own,
        // lowercase code — it must NOT read as gate absence
        assert!(gate_presence_from_error(&remote("function_not_found", "")).is_err());
        assert!(gate_presence_from_error(&iii_sdk::Error::Timeout).is_err());
    }

    fn step_in(namespace: Option<&str>) -> PipeRequest {
        parse(json!({ "through": [
            { "function": "state::get", "namespace": namespace,
              "payload": { "scope": "s", "key": "k" } },
        ]}))
        .unwrap()
    }

    #[test]
    fn a_step_in_another_namespace_is_refused_while_the_gate_runs_or_is_unknown() {
        let req = step_in(Some("laptop"));
        assert!(validate_with_approval_gate(&req, false, "own").is_ok());
        let lost = || Err("could not inspect approval-gate availability".to_string());
        for gate in [Ok(true), split_gate(&req, "own", lost())] {
            let err = validate_with_approval_gate(&req, gate.unwrap(), "own").unwrap_err();
            assert!(
                err.contains(
                    "step 1 (state::get): a call in namespace laptop must be approved in that \
                     namespace — call state::get directly with \"namespace\": \"laptop\" \
                     instead of through fp::pipe"
                ),
                "{err}"
            );
        }
        assert_eq!(split_gate(&req, "own", Ok(false)), Ok(false));
        // A database write alone keeps the lookup error: no write runs on a guess.
        let write = parse(json!({ "through": [{ "function": "database::execute" }] })).unwrap();
        assert!(split_gate(&write, "own", lost()).is_err());
    }

    #[test]
    fn a_step_in_its_own_namespace_or_without_one_still_runs_under_the_gate() {
        for namespace in [None, Some("own"), Some(" ")] {
            assert!(
                validate_with_approval_gate(&step_in(namespace), true, "own").is_ok(),
                "{namespace:?}"
            );
        }
    }

    #[test]
    fn a_step_the_gate_decides_is_refused_with_the_up_front_reason() {
        for (function, namespace) in [
            ("database::execute", None),
            ("state::get", Some("laptop")),
            ("database::execute", Some("laptop")),
        ] {
            let req =
                parse(json!({ "through": [{ "function": function, "namespace": namespace }] }))
                    .unwrap();
            let up_front = validate_with_approval_gate(&req, true, "own").unwrap_err();
            let reasons = gate_refusals(function, namespace, "own", PIPE_ID);
            assert!(!reasons.is_empty(), "{function}");
            for reason in reasons {
                let problem = format!("step 1 ({function}): {reason}");
                assert!(up_front.contains(&problem), "{up_front}");
            }
        }
        for (function, namespace) in [
            ("database::query", None),
            ("state::get", None),
            ("state::get", Some("own")),
            ("state::get", Some(" ")),
            // Refused with or without the gate: its state decides nothing.
            ("configuration::get", None),
        ] {
            let reasons = gate_refusals(function, namespace, "own", PIPE_ID);
            assert!(reasons.is_empty(), "{function}: {reasons:?}");
        }
    }

    #[test]
    fn a_step_calls_in_the_namespace_it_names() {
        for (namespace, expected) in [
            (Some("laptop"), r#"namespace: Some("laptop")"#),
            (None, "namespace: None"),
            (Some(" "), "namespace: None"),
        ] {
            let request = format!(
                "{:?}",
                step_request(&step_in(namespace).through[0], json!({}))
            );
            assert!(request.contains(expected), "{request}");
        }
    }

    fn scope() -> Value {
        json!({ "root": "/work/session-7", "grants": [], "boundary": "workspace" })
    }

    #[test]
    fn remote_scoped_step_matches_harness_table() {
        for (function_id, expected) in [
            ("laptop::coder::read-file", true),
            ("a::b::shell::exec", true),
            ("coder::read-file", false),
            ("laptop::shell::filesystem::read", false),
        ] {
            assert_eq!(remote_scoped_step(function_id), expected, "{function_id}");
        }
    }

    #[test]
    fn bus_step_args_strips_model_scope_on_node_prefixed_steps() {
        for function in ["laptop::coder::read-file", "a::b::shell::exec"] {
            for trusted in [Some(scope()), None] {
                let out = bus_step_args(
                    function,
                    json!({ "path": "a", "fs_scope": { "root": "/evil" } }),
                    trusted.as_ref(),
                )
                .unwrap();
                assert_eq!(out, json!({ "path": "a" }), "{function}");
                assert_eq!(
                    bus_step_args(function, json!({ "path": "a" }), trusted.as_ref()).unwrap(),
                    json!({ "path": "a" }),
                    "{function}"
                );
                for args in [Value::Null, json!("a"), json!([1])] {
                    assert_eq!(
                        bus_step_args(function, args.clone(), trusted.as_ref()).unwrap(),
                        args,
                        "{function}"
                    );
                }
            }
            let req = parse(json!({ "through": [{ "function": function }] })).unwrap();
            assert!(validate(&req).is_ok(), "{function}");
        }
        for function in ["laptop::browser::open", "laptop::shell::filesystem::read"] {
            let args = json!({ "fs_scope": 1 });
            assert_eq!(
                bus_step_args(function, args.clone(), Some(&scope())).unwrap(),
                args,
                "{function}"
            );
        }
    }

    #[test]
    fn forbidden_step_refuses_node_reserved_ids() {
        for function in [
            "shell::config-status",
            "shell::kill-session-jobs",
            "shell::workspace::reset",
            "mesh-node::workers::apply",
            "mesh::auth",
            "mesh::middleware",
            "mesh::on-observer-registration",
            "mesh::on-config-change",
            "mesh::on-functions-change",
            "mesh::on-node-changed",
            "mesh::on-status-changed",
            "mesh::on-tunnel-change",
            "mesh::ui-content",
            "mesh::configuration-id",
            "state::claim-namespace",
            "provider-openai-codex::state::get",
            "fp::pipe",
            "fp::batch",
            "engine::observers::dispose",
            "session::append",
            "session::store::get-attachment",
            "approval::resolve",
            "configuration::get",
            "oauth::anthropic::login",
            "secrets::resolve",
            "secrets::access",
            "secrets::set",
            "secrets::compare-and-set",
            "secrets::delete",
            "secrets::import",
            "secrets::on-config-change",
            "secrets::on-changed",
            "engine::queue::enqueue",
            "router::chat",
            "router::complete",
            "router::route",
            "router::count_tokens",
            "router::provider::resolve",
            "router::models::reconcile",
            "router::models::list::x",
            "harness::router::models::list",
            "provider::list",
            "provider::anthropic::chat",
            "harness::send",
            "run::start",
            "stream::set",
            "hook-fanout::dispatch",
            "iii::durable::publish",
            "storage::on-config-change",
        ] {
            for prefix in ["", "laptop::", "a::b::"] {
                let id = format!("{prefix}{function}");
                for gate in [false, true] {
                    assert!(forbidden_step(&id, gate).is_some(), "{id}, gate={gate}");
                }
            }
        }
    }

    #[test]
    fn prefixed_forbidden_rules_preserve_existing_exceptions() {
        for prefix in ["", "laptop::", "a::b::"] {
            for function in [
                "shell::exec",
                "shell::filesystem::read",
                "state::get",
                "state::set",
                "state::update",
                "state::delete",
                "kanban::ticket::restore",
                "kanban::project::update",
                "kanban::project::archive",
                "kanban::project::restore",
                "kanban::release::create",
                "kanban::release::update",
                "kanban::release::ship",
                "kanban::release::reopen",
                "kanban::label::convert",
                "state::claim-namespace-info",
                "provider-openai-codex::state-info",
                "mesh::status",
                "session::get",
                "session::list",
                "session::messages",
                "session::messages-range",
                "session::messages-tail",
                "session::get-message",
                "session::get-attachment",
                "session::list-attachments",
                "secrets::list",
                "secrets::get",
                "secrets::status",
                "secrets::detect",
                "router::models::list",
                "router::models::get",
                "router::models::budget",
                "router::models::supports",
                "router::provider::list",
            ] {
                let id = format!("{prefix}{function}");
                for gate in [false, true] {
                    assert!(forbidden_step(&id, gate).is_none(), "{id}, gate={gate}");
                }
            }
            let query = format!("{prefix}database::query");
            assert!(forbidden_step(&query, true).is_none(), "{query}");
            assert!(forbidden_step(&query, false).is_none(), "{query}");
            // Gate-only: the writes and the 7 policy-denied database ids run
            // only while no approval gate runs.
            for function in [
                "database::execute",
                "database::on-secret-change",
                "database::saveConnection",
                "database::deleteConnection",
                "database::getConnection",
                "database::testConnection",
                "database::mention::search",
                "database::mention::get",
            ] {
                let write = format!("{prefix}{function}");
                assert!(forbidden_step(&write, true).is_some(), "{write}");
                // The hook pattern sees the last segment of an old mesh spelling.
                let hook = !prefix.is_empty() && function == "database::on-secret-change";
                assert_eq!(forbidden_step(&write, false).is_some(), hook, "{write}");
            }
        }
    }

    #[test]
    fn calls_the_policy_denies_are_refused_in_every_spelling() {
        for (function, needle) in [
            ("storage::saveBucket", "credential"),
            ("storage::getBucket", "credential"),
            ("email::config-status", "credential"),
            ("claude::auth::status", "credential"),
            ("context::compact", "turn-control"),
            ("judge-openai::on-secret-change", "lifecycle"),
            ("telegram-bot::on-message-added", "lifecycle"),
            ("context::on_message_added", "lifecycle"),
            ("github::on-config-change", "lifecycle"),
            ("claude::terminal::describe", "terminal"),
            ("tmux::terminal::open", "terminal"),
            ("shell::pty::open", "terminal"),
            // No exception: the policy allows it bare but denies it prefixed.
            ("shell::pty::sessions", "terminal"),
            ("shell::ui-state::set", "console UI"),
            ("kanban::ui-content", "console UI"),
            ("stories::ui-file", "console UI"),
            ("browser::pick::start", "console UI"),
            ("computer::frame", "console UI"),
            ("a2ui::action", "console UI"),
            ("slack::interactions", "console UI"),
            ("shell::scm::commit-message", "policy denies"),
            ("github::webhooks::receive", "policy denies"),
            ("ade::conversations::import", "policy denies"),
            ("kanban::ticket::delete", "policy denies"),
            ("kanban::release::delete", "policy denies"),
            ("eval::step", "policy denies"),
            ("security-scan::execute", "policy denies"),
            ("ios-simulator::tenants::list", "policy denies"),
            ("slack::events", "policy denies"),
            ("openwiki::cron::refresh-due", "policy denies"),
        ] {
            for prefix in ["", "laptop::", "a::b::", "laptop:::"] {
                let id = format!("{prefix}{function}");
                for gate in [false, true] {
                    let reason = forbidden_step(&id, gate);
                    let reason = reason.unwrap_or_else(|| panic!("{id}, gate={gate}"));
                    assert!(reason.contains(needle), "{id}: {reason}");
                    let problem = format!("call 12 ({id}): {reason}");
                    assert!(problem.chars().count() <= MAX_PROBLEM_CHARS, "{problem}");
                }
            }
        }
    }

    #[test]
    fn scoped_steps_require_the_harness_stamp() {
        // no stamp (cron / worker-to-worker / no working directory) → refused,
        // teachably: names the stamp and points at direct calls
        for function in ["shell::exec", "coder::search", "shell::fs::grep"] {
            let req = parse(json!({ "through": [{ "function": function }] })).unwrap();
            let err = validate(&req).unwrap_err();
            assert!(err.contains("harness-stamped"), "{function}: {err}");
            assert!(err.contains("call the function directly"), "{function}");
        }

        // with the stamp, the motivating pipeline is accepted
        let req = parse(json!({
            "fs_scope": scope(),
            "through": [
                { "function": "coder::search", "payload": { "query": "TODO" } },
                { "function": "fp::map", "payload": { "path": "/path" } },
                { "function": "state::set", "payload": { "scope": "s", "key": "k" } },
            ],
        }))
        .unwrap();
        assert!(validate(&req).is_ok());

        // the shell::filesystem::* control plane stays unscoped on both sides
        // (harness is_scoped_function parity) — allowed with no stamp
        let req = parse(json!({ "through": [
            { "function": "shell::filesystem::validate", "payload": { "path": "/p" } },
        ]}))
        .unwrap();
        assert!(validate(&req).is_ok());

        // `into` aimed at /fs_scope (or below) is an attack or a mistake —
        // statically refused, never silently overwritten
        for into in ["/fs_scope", "/fs_scope/root"] {
            let req = parse(json!({
                "fs_scope": scope(),
                "through": [
                    { "function": "state::get", "payload": { "scope": "s", "key": "k" } },
                    { "function": "shell::exec", "into": into,
                      "payload": { "command": "ls" } },
                ],
            }))
            .unwrap();
            assert!(validate(&req).unwrap_err().contains("fs_scope"), "{into}");
        }
    }

    #[test]
    fn bus_step_args_stamps_last_and_overwrites_forgeries() {
        let trusted = scope();

        // scoped step gets the trusted scope
        let out = bus_step_args("shell::exec", json!({ "command": "ls" }), Some(&trusted)).unwrap();
        assert_eq!(out, json!({ "command": "ls", "fs_scope": trusted }));

        // a forged literal fs_scope in the authored payload is REPLACED whole
        let out = bus_step_args(
            "coder::search",
            json!({ "query": "q", "fs_scope": { "root": "/", "grants": ["/"] } }),
            Some(&trusted),
        )
        .unwrap();
        assert_eq!(out, json!({ "query": "q", "fs_scope": trusted }));

        // ordering proof: even a value threaded to /fs_scope by inject() (the
        // vector validate() also refuses) loses to the stamp, because the
        // stamp runs after threading
        let threaded = inject(
            json!({ "command": "ls" }),
            "/fs_scope",
            json!({ "root": "/", "boundary": "configured_roots" }),
        )
        .unwrap();
        let out = bus_step_args("shell::exec", threaded, Some(&trusted)).unwrap();
        assert_eq!(out, json!({ "command": "ls", "fs_scope": trusted }));

        // fail closed: no stamp, or a non-object payload
        assert!(
            bus_step_args("shell::exec", json!({}), None)
                .unwrap_err()
                .contains("without a harness fs_scope stamp")
        );
        assert!(
            bus_step_args("shell::exec", json!("ls"), Some(&trusted))
                .unwrap_err()
                .contains("must be an object")
        );

        // non-scoped steps pass through untouched — the scope must not leak
        let args = json!({ "scope": "s", "key": "k" });
        assert_eq!(
            bus_step_args("state::set", args.clone(), Some(&trusted)).unwrap(),
            args
        );
        assert_eq!(
            bus_step_args("state::set", args.clone(), None).unwrap(),
            args
        );

        // predicate mirror spot-checks (harness is_scoped_function parity)
        assert!(is_scoped_step("shell::exec"));
        assert!(is_scoped_step("coder::fs::read"));
        assert!(!is_scoped_step("shell::filesystem::validate"));
        assert!(!is_scoped_step("state::set"));
    }

    #[test]
    fn pipe_as_seed_step_redirects_to_the_seed_rule() {
        // Live-tested (haiku-4.5): five consecutive attempts used fp::pipe
        // {value} as step 1 to inject a literal starting value — the refusal
        // must teach the seed rule, not just state the nesting ban.
        let req = parse(json!({ "through": [
            { "function": "fp::pipe", "payload": { "value": [1, 2, 3] } },
            { "function": "fp::take", "payload": { "n": 3 } },
        ]}))
        .unwrap();
        let err = validate(&req).unwrap_err();
        assert!(err.contains("do not nest") && err.contains("seed the FIRST transform"));
    }

    #[test]
    fn inject_shapes_the_value() {
        // inject creates intermediate objects and rejects bad pointers
        assert_eq!(
            inject(json!({ "scope": "s" }), "/value", json!("x")).unwrap(),
            json!({ "scope": "s", "value": "x" })
        );
        assert_eq!(
            inject(json!({}), "/config/value", json!(1)).unwrap(),
            json!({ "config": { "value": 1 } })
        );
        // schema-literal models pad steps with a placeholder ({"value": {}}
        // — the published schemas mark `value` required); the threaded value
        // must overwrite it (live-tested: ornith-35b padded every join step)
        assert_eq!(
            inject(
                json!({ "separator": "-", "value": {} }),
                "/value",
                json!(["a"])
            )
            .unwrap(),
            json!({ "separator": "-", "value": ["a"] })
        );
        assert!(inject(json!({}), "value", json!(1)).is_err());
        assert!(inject(json!({ "a": 1 }), "/a/b", json!(1)).is_err());

        // RFC 6901 escapes decode per token; arrays refuse teachably
        assert_eq!(
            inject(json!({}), "/headers/x~1trace", json!(1)).unwrap(),
            json!({ "headers": { "x/trace": 1 } })
        );
        assert_eq!(
            inject(json!({}), "/a~0b", json!(1)).unwrap(),
            json!({ "a~b": 1 })
        );
        assert!(
            inject(json!({ "items": [] }), "/items/0", json!(1))
                .unwrap_err()
                .contains("non-object")
        );
    }

    #[test]
    fn previews_and_errors_stay_small() {
        assert_eq!(preview_of(&json!("abcdef"), 3, &[]), "abc…");
        assert_eq!(preview_of(&json!("ab"), 3, &[]), "ab");

        // an unbounded preview would defeat the receipt design — clamp it
        assert_eq!(preview_len(None), DEFAULT_PREVIEW_CHARS);
        assert_eq!(preview_len(Some(50)), 50);
        assert_eq!(preview_len(Some(u32::MAX)), MAX_PREVIEW_CHARS);

        let receipts = vec![
            StepReceipt {
                function: "browser::fetch".into(),
                chars: 184_232,
                note: None,
            },
            StepReceipt {
                function: "fp::get".into(),
                chars: 180_101,
                note: None,
            },
        ];
        // loop index 2 = the third step; the message is 1-based like the UI
        let msg = step_error(2, "state::set", "boom", &receipts);
        assert!(msg.contains("step 3 (state::set): boom"));
        assert!(msg.contains("browser::fetch→184232ch"));
    }

    #[test]
    fn desc_teaches_bare_value_threading() {
        // The live trap: a model that saw {value: X} from ten direct calls
        // composed `pick {paths:["value"]}` to unwrap between steps. The
        // published description must keep saying nothing threads wrapped.
        assert!(PIPE_DESC.contains("thread the bare value"));
    }

    #[test]
    fn every_problem_is_reported_in_one_refusal() {
        let req = parse(json!({ "through": [
            { "function": "configuration::get" },
            { "function": "harness::send" },
            { "function": "fp::batch" },
        ]}))
        .unwrap();
        let err = validate(&req).unwrap_err();
        assert!(err.starts_with("pipe refused, 3 problem(s)"), "{err}");
        for needle in [
            "step 1 (configuration::get)",
            "step 2 (harness::send)",
            "step 3 (fp::batch)",
        ] {
            assert!(err.contains(needle), "{err}");
        }
    }

    #[test]
    fn an_oversized_pipe_still_reports_every_other_problem() {
        let mut through = vec![json!({ "function": "configuration::get" })];
        through.extend((0..12).map(|_| json!({ "function": "browser::fetch" })));
        let req: PipeRequest = serde_json::from_value(json!({ "through": through })).unwrap();
        let err = validate(&req).unwrap_err();
        assert!(err.starts_with("pipe refused, 2 problem(s)"), "{err}");
        assert!(err.contains("a pipe takes 1..=12 steps"), "{err}");
        assert!(err.contains("step 1 (configuration::get)"), "{err}");
    }

    #[test]
    fn an_oversized_refusal_stays_bounded_and_counts_everything() {
        let through: Vec<Value> = (0..200)
            .map(|_| json!({ "function": "shell::exec" }))
            .collect();
        let req: PipeRequest = serde_json::from_value(json!({ "through": through })).unwrap();
        let err = validate(&req).unwrap_err();
        assert!(err.starts_with("pipe refused, 201 problem(s)"), "{err}");
        assert!(err.contains("and 169 more"), "{err}");
        assert!(err.len() < 12_000, "{}", err.len());
    }

    #[test]
    fn a_long_function_id_cannot_inflate_a_refusal() {
        let long_name = format!("configuration::{}", "x".repeat(100_000));
        let req: PipeRequest = serde_json::from_value(json!({
            "through": [{ "function": long_name }, { "function": "configuration::get" }]
        }))
        .unwrap();
        let err = validate(&req).unwrap_err();
        assert!(err.starts_with("pipe refused, 2 problem(s)"), "{err}");
        assert!(err.contains("step 1"), "{err}");
        assert!(err.len() < 12_000, "{}", err.len());
    }

    #[test]
    fn a_gate_adds_problems_that_the_refusal_must_name() {
        let req = parse(json!({ "through": [
            { "function": "configuration::get" },
            { "function": "database::execute", "payload": { "db": "primary", "sql": "DELETE FROM t" } },
        ]}))
        .unwrap();
        // Without the gate only the configuration step is refused...
        let without = validate_with_approval_gate(&req, false, "own").unwrap_err();
        assert!(
            without.starts_with("pipe refused, 1 problem(s)"),
            "{without}"
        );
        // ...with it the database write is refused too, so the run path must
        // consult the real gate state before reporting a complete list.
        let with = validate_with_approval_gate(&req, true, "own").unwrap_err();
        assert!(with.starts_with("pipe refused, 2 problem(s)"), "{with}");
        assert!(with.contains("database::execute"), "{with}");
    }

    #[tokio::test]
    async fn a_pipe_ending_in_a_stream_handle_withholds_its_key_in_the_preview() {
        // Transforms never touch the bus, so a disconnected client is fine.
        let iii = IIIClient::new("ws://127.0.0.1:1");
        // What `shell::fs::read` returns: a handle to the file, not its text.
        let read = json!({
            "content": { "channel_id": "c1", "access_key": "secret-key", "direction": "read" },
            "size": 16,
        });
        let req = parse(json!({ "through": [
            { "function": "fp::get", "payload": { "value": read, "path": "" } },
        ]}))
        .unwrap();
        let preview = run(&iii, req).await.unwrap().value_preview.unwrap();
        assert!(!preview.contains("secret-key"), "{preview}");
        assert!(preview.contains("coder::read-file"), "{preview}");
        assert!(preview.contains(r#""channel_id":"c1""#), "{preview}");
        assert!(preview.contains(r#""direction":"read""#), "{preview}");
    }

    #[tokio::test]
    async fn a_pipe_that_reshapes_a_stream_handle_still_withholds_its_key() {
        let iii = IIIClient::new("ws://127.0.0.1:1");
        let read = json!({
            "content": { "channel_id": "c1", "access_key": "secret-key", "direction": "read" },
            "size": 16,
        });
        let first = json!({ "function": "fp::get", "payload": { "value": read, "path": "" } });
        // Each tail leaves no handle shape behind: the bare key, a handle
        // without its channel_id, and the key behind a default.
        for tail in [
            json!([{ "function": "fp::get", "payload": { "path": "/content/access_key" } }]),
            json!([
                { "function": "fp::get", "payload": { "path": "/content" } },
                { "function": "fp::omit", "payload": { "paths": ["channel_id"] } },
            ]),
            json!([{ "function": "fp::getOr",
                     "payload": { "path": "/content/access_key", "default": "none" } }]),
        ] {
            let mut through = vec![first.clone()];
            through.extend(tail.as_array().unwrap().iter().cloned());
            let req = parse(json!({ "through": through })).unwrap();
            let preview = run(&iii, req).await.unwrap().value_preview.unwrap();
            assert!(!preview.contains("secret-key"), "{tail}: {preview}");
            assert!(preview.contains("coder::read-file"), "{tail}: {preview}");
        }
    }

    #[test]
    fn a_preview_withholds_nested_handle_keys_before_the_cut() {
        let handle = json!({ "channel_id": "c1", "access_key": "secret-key", "direction": "read" });
        let nested = json!([{ "reader": handle }, handle]);
        let mut keys = Vec::new();
        stream_access_keys(&nested, &mut keys);
        let whole = preview_of(&nested, 1_000, &keys);
        assert!(!whole.contains("secret-key"), "{whole}");
        assert_eq!(whole.matches("coder::read-file").count(), 2, "{whole}");
        // Keys sort, so `access_key` leads a handle: a cut at 30 chars lands in it.
        let cut = preview_of(&nested, 30, &keys);
        assert!(!cut.contains("secr"), "{cut}");
        // Without a string channel_id it is the caller's data, not a handle;
        // an empty key would otherwise match between every char.
        let mut none = Vec::new();
        stream_access_keys(&json!({ "access_key": "kept", "channel": "c1" }), &mut none);
        stream_access_keys(&json!({ "channel_id": "c1", "access_key": "" }), &mut none);
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn keys_are_withheld_longest_first_in_one_pass() {
        // The short key is listed first; a key-by-key pass would leave "def".
        let keys = ["abc".to_string(), "abcdef".to_string()];
        let out = withhold_access_keys("x abcdef y abc abcdef".into(), &keys);
        assert_eq!(out.matches(WITHHELD_ACCESS_KEY).count(), 3, "{out}");
        assert!(!out.contains("abc") && !out.contains("def"), "{out}");
        // A key that also occurs inside the note leaves the note whole.
        let out = withhold_access_keys("k=read".into(), &["read".to_string()]);
        assert_eq!(out, format!("k={WITHHELD_ACCESS_KEY}"));
    }

    /// fp follows the root iii-permissions.yaml by hand. A deny added there
    /// fails this test until `forbidden_unprefixed_step` refuses it or
    /// `ALLOWED_ON_PURPOSE` names a reason to accept it.
    #[test]
    fn every_call_the_root_policy_denies_is_refused_or_allowed_on_purpose() {
        use approval_gate::permissions::{
            Action, Decision, Permissions, compile_rule, parse_rules_from_config,
        };
        use approval_gate::types::PermissionMode;

        #[derive(Deserialize)]
        struct PolicyFile {
            rules: Vec<Value>,
        }
        let file: PolicyFile =
            serde_yaml_ng::from_str(include_str!("../../iii-permissions.yaml")).unwrap();
        let specs = parse_rules_from_config(&Value::Array(file.rules.clone()));
        // The parser drops an entry it cannot read, with no error.
        assert_eq!(specs.len(), file.rules.len(), "a rule did not parse");
        let policy = Permissions::compile(&specs).unwrap();
        let denied = |id: &str| {
            matches!(
                policy.check(id, &json!({}), PermissionMode::Manual),
                Decision::Deny { .. }
            )
        };

        // Each deny rule's literal id or glob samples, every exact id the file
        // names, and every id fp accepts on purpose; then all of them again
        // under an old mesh prefix. An exception inside a denied glob
        // (`zz::shell::pty::sessions`) is found only this way.
        let mut samples = Vec::new();
        for (index, spec) in specs.iter().enumerate() {
            let rule = compile_rule(spec, index).unwrap();
            let rule_samples = match rule.glob {
                None => vec![rule.function_id.clone()],
                Some(_) => vec![
                    rule.function_id.replace('*', "zz"),
                    rule.function_id.replace('*', "zz::yy"),
                    rule.function_id.replace('*', "zz:"),
                ],
            };
            if rule.action == Action::Deny {
                assert!(
                    rule_samples.iter().any(|s| denied(s)),
                    "{}: no sample is denied, so this test cannot see it",
                    rule.rule_id
                );
            } else if rule.glob.is_some() {
                continue;
            }
            samples.extend(rule_samples);
        }
        let on_purpose = SESSION_READS
            .iter()
            .chain(SECRETS_READS)
            .chain(ROUTER_READS)
            .chain(&["database::query"])
            .copied()
            .chain(ALLOWED_ON_PURPOSE.iter().map(|(id, _)| *id));
        samples.extend(on_purpose.map(str::to_string));
        let prefixed: Vec<String> = samples.iter().map(|s| format!("zz::{s}")).collect();
        samples.extend(prefixed);
        samples.sort();
        samples.dedup();

        let mut accepted = Vec::new();
        for sample in samples.iter().filter(|s| denied(s)) {
            if ALLOWED_ON_PURPOSE.iter().any(|(id, _)| id == sample) {
                continue;
            }
            // The old spelling pins fp's `::` suffix scan: the policy denies
            // few ids under a prefix.
            for id in [sample.clone(), format!("zz::{sample}")] {
                if forbidden_step(&id, false).is_none() {
                    accepted.push(id);
                }
            }
        }
        assert!(
            accepted.is_empty(),
            "the policy denies these, fp accepts them: {accepted:?}"
        );
        for (id, reason) in ALLOWED_ON_PURPOSE {
            assert!(
                denied(id),
                "{id} is no longer denied: drop it from the table"
            );
            assert!(!reason.trim().is_empty(), "{id} needs a reason");
        }
    }
}
