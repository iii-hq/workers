//! Registration: shared response types, the generic register helpers that
//! keep ~30 thin gh wrappers non-repetitive, and the wire-surface catalog
//! golden-tested in `tests/schemas.rs`.

pub mod actions;
pub mod issue;
pub mod passthrough;
pub mod pr;
pub mod release;
pub mod repo;
pub mod search;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::configuration::ConfigCell;
use crate::gh::{self, GhError, GhOutcome};

/// Exit codes that mean "the gh operation succeeded" for most commands.
const OK: &[i32] = &[0];
/// `gh pr checks` exit codes are data: 0 = all passed, 1 = some failed,
/// 8 = some pending. Only other codes are real failures.
const CHECKS_OK: &[i32] = &[0, 1, 8];

/// Parsed `--json` output of a gh command.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ValueResponse {
    /// The JSON gh printed for the requested `--json` fields.
    pub value: Value,
}

/// Trimmed stdout of a mutating gh command (gh prints the created/updated
/// URL or a confirmation line; some commands print nothing).
#[derive(Debug, Serialize, JsonSchema)]
pub struct TextResponse {
    /// gh's stdout, trimmed.
    pub output: String,
}

/// `github::pr::diff` response.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffResponse {
    /// The unified diff as printed by `gh pr diff`.
    pub diff: String,
    /// True when the diff exceeded the capture cap and was cut off.
    pub truncated: bool,
}

/// Lift a completed invocation into "the gh operation succeeded": a timeout
/// becomes `Error::Remote("timeout")`; an exit code outside `ok_codes` becomes
/// `Error::Handler` carrying gh's stderr (that's where gh explains auth
/// failures, 404s, and validation errors).
fn expect_ok(id: &str, out: GhOutcome, ok_codes: &[i32]) -> Result<GhOutcome, Error> {
    if out.timed_out {
        return Err(GhError::timeout(out.duration_ms).into());
    }
    match out.exit_code {
        Some(code) if ok_codes.contains(&code) => Ok(out),
        Some(code) => Err(Error::Handler(format!(
            "{id}: gh exited {code}: {}",
            out.stderr.trim()
        ))),
        None => Err(Error::Handler(format!(
            "{id}: gh was killed before exiting: {}",
            out.stderr.trim()
        ))),
    }
}

/// Parse gh's stdout as JSON. Empty stdout parses to `null` (some commands
/// print nothing on success); truncated stdout is an error — a cut-off JSON
/// document would parse to garbage or fail confusingly.
fn parse_stdout(id: &str, out: &GhOutcome) -> Result<Value, Error> {
    if out.stdout_truncated {
        return Err(Error::Handler(format!(
            "{id}: gh output exceeded max_output_bytes and was truncated; \
             lower --limit, raise the cap, or use github::exec"
        )));
    }
    let trimmed = out.stdout.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(trimmed).map_err(|e| {
        let head: String = trimmed.chars().take(200).collect();
        Error::Handler(format!("{id}: unexpected non-JSON gh output ({e}): {head}"))
    })
}

/// Register a curated wrapper whose stdout is `--json` output: typed request
/// → pure argv builder → bounded gh run → parsed `ValueResponse`.
fn register_json<Req>(
    iii: &IIIClient,
    cell: &ConfigCell,
    id: &'static str,
    desc: &'static str,
    ok_codes: &'static [i32],
    build: fn(&Req) -> Vec<String>,
) where
    Req: serde::de::DeserializeOwned + JsonSchema + Send + 'static,
{
    let cell = cell.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let cell = cell.clone();
            async move {
                let cfg = cell.read().await.clone();
                let out = gh::run(&cfg, &build(&req), None, None)
                    .await
                    .map_err(Error::from)?;
                let out = expect_ok(id, out, ok_codes)?;
                let value = parse_stdout(id, &out)?;
                Ok::<_, Error>(ValueResponse { value })
            }
        })
        .description(desc),
    );
}

/// Register a curated wrapper for a mutating command: typed request → pure
/// argv builder → bounded gh run → trimmed stdout as `TextResponse`.
fn register_text<Req>(
    iii: &IIIClient,
    cell: &ConfigCell,
    id: &'static str,
    desc: &'static str,
    build: fn(&Req) -> Vec<String>,
) where
    Req: serde::de::DeserializeOwned + JsonSchema + Send + 'static,
{
    let cell = cell.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let cell = cell.clone();
            async move {
                let cfg = cell.read().await.clone();
                let out = gh::run(&cfg, &build(&req), None, None)
                    .await
                    .map_err(Error::from)?;
                let out = expect_ok(id, out, OK)?;
                Ok::<_, Error>(TextResponse {
                    output: out.stdout.trim().to_string(),
                })
            }
        })
        .description(desc),
    );
}

/// `github::pr::diff` is hand-registered: its response is the raw diff, with
/// the truncation flag surfaced instead of erroring (a cut-off diff is still
/// useful, unlike cut-off JSON).
fn register_diff(iii: &IIIClient, cell: &ConfigCell) {
    let cell = cell.clone();
    iii.register_function(
        pr::DIFF_ID,
        RegisterFunction::new_async(move |req: pr::DiffRequest| {
            let cell = cell.clone();
            async move {
                let cfg = cell.read().await.clone();
                let out = gh::run(&cfg, &pr::diff_args(&req), None, None)
                    .await
                    .map_err(Error::from)?;
                let out = expect_ok(pr::DIFF_ID, out, OK)?;
                Ok::<_, Error>(DiffResponse {
                    truncated: out.stdout_truncated,
                    diff: out.stdout,
                })
            }
        })
        .description(pr::DIFF_DESC),
    );
}

/// Register every `github::*` function. Keep in lockstep with [`catalog`].
pub fn register_all(iii: &IIIClient, cell: &ConfigCell) {
    register_json(iii, cell, pr::LIST_ID, pr::LIST_DESC, OK, pr::list_args);
    register_json(iii, cell, pr::VIEW_ID, pr::VIEW_DESC, OK, pr::view_args);
    register_text(iii, cell, pr::CREATE_ID, pr::CREATE_DESC, pr::create_args);
    register_text(iii, cell, pr::EDIT_ID, pr::EDIT_DESC, pr::edit_args);
    register_text(iii, cell, pr::MERGE_ID, pr::MERGE_DESC, pr::merge_args);
    register_text(
        iii,
        cell,
        pr::COMMENT_ID,
        pr::COMMENT_DESC,
        pr::comment_args,
    );
    register_text(iii, cell, pr::REVIEW_ID, pr::REVIEW_DESC, pr::review_args);
    register_diff(iii, cell);
    register_json(
        iii,
        cell,
        pr::CHECKS_ID,
        pr::CHECKS_DESC,
        CHECKS_OK,
        pr::checks_args,
    );

    register_json(
        iii,
        cell,
        issue::LIST_ID,
        issue::LIST_DESC,
        OK,
        issue::list_args,
    );
    register_json(
        iii,
        cell,
        issue::VIEW_ID,
        issue::VIEW_DESC,
        OK,
        issue::view_args,
    );
    register_text(
        iii,
        cell,
        issue::CREATE_ID,
        issue::CREATE_DESC,
        issue::create_args,
    );
    register_text(
        iii,
        cell,
        issue::EDIT_ID,
        issue::EDIT_DESC,
        issue::edit_args,
    );
    register_text(
        iii,
        cell,
        issue::COMMENT_ID,
        issue::COMMENT_DESC,
        issue::comment_args,
    );
    register_text(
        iii,
        cell,
        issue::CLOSE_ID,
        issue::CLOSE_DESC,
        issue::close_args,
    );

    register_json(
        iii,
        cell,
        repo::VIEW_ID,
        repo::VIEW_DESC,
        OK,
        repo::view_args,
    );
    register_json(
        iii,
        cell,
        repo::LIST_ID,
        repo::LIST_DESC,
        OK,
        repo::list_args,
    );

    register_json(
        iii,
        cell,
        actions::RUN_LIST_ID,
        actions::RUN_LIST_DESC,
        OK,
        actions::run_list_args,
    );
    register_json(
        iii,
        cell,
        actions::RUN_VIEW_ID,
        actions::RUN_VIEW_DESC,
        OK,
        actions::run_view_args,
    );
    register_text(
        iii,
        cell,
        actions::RUN_RERUN_ID,
        actions::RUN_RERUN_DESC,
        actions::run_rerun_args,
    );
    register_text(
        iii,
        cell,
        actions::RUN_CANCEL_ID,
        actions::RUN_CANCEL_DESC,
        actions::run_cancel_args,
    );
    register_json(
        iii,
        cell,
        actions::WORKFLOW_LIST_ID,
        actions::WORKFLOW_LIST_DESC,
        OK,
        actions::workflow_list_args,
    );
    register_text(
        iii,
        cell,
        actions::WORKFLOW_RUN_ID,
        actions::WORKFLOW_RUN_DESC,
        actions::workflow_run_args,
    );

    register_json(
        iii,
        cell,
        release::LIST_ID,
        release::LIST_DESC,
        OK,
        release::list_args,
    );
    register_json(
        iii,
        cell,
        release::VIEW_ID,
        release::VIEW_DESC,
        OK,
        release::view_args,
    );
    register_text(
        iii,
        cell,
        release::CREATE_ID,
        release::CREATE_DESC,
        release::create_args,
    );

    register_json(
        iii,
        cell,
        search::REPOS_ID,
        search::REPOS_DESC,
        OK,
        search::repos_args,
    );
    register_json(
        iii,
        cell,
        search::ISSUES_ID,
        search::ISSUES_DESC,
        OK,
        search::issues_args,
    );
    register_json(
        iii,
        cell,
        search::PRS_ID,
        search::PRS_DESC,
        OK,
        search::prs_args,
    );
    register_json(
        iii,
        cell,
        search::CODE_ID,
        search::CODE_DESC,
        OK,
        search::code_args,
    );

    passthrough::register(iii, cell);
}

// ---- argv-building helpers (pure; shared by the group modules) ----

/// Start an argv from static parts.
pub(crate) fn argv<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Push `flag value` when the value is present.
pub(crate) fn push_opt(a: &mut Vec<String>, flag: &str, v: Option<impl std::fmt::Display>) {
    if let Some(v) = v {
        a.push(flag.to_string());
        a.push(v.to_string());
    }
}

/// Push a bare `flag` when the option is `Some(true)`.
pub(crate) fn push_bool(a: &mut Vec<String>, flag: &str, v: Option<bool>) {
    if v == Some(true) {
        a.push(flag.to_string());
    }
}

/// Push `flag value` once per element.
pub(crate) fn push_each(a: &mut Vec<String>, flag: &str, vs: &Option<Vec<String>>) {
    if let Some(vs) = vs {
        for v in vs {
            a.push(flag.to_string());
            a.push(v.clone());
        }
    }
}

// ---- wire-surface catalog ----

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()` on the handler's request/response types):
/// `RegisterFunction::new_async` auto-extracts schemas from the SAME structs
/// referenced here, with the same schemars 0.8 generator settings, so a
/// catalog snapshot pins exactly what registration emits.
fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with [`register_all`].
/// `github::on-config-change` is internal and deliberately not listed.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<pr::ListRequest, ValueResponse>(pr::LIST_ID, pr::LIST_DESC),
        spec::<pr::ViewRequest, ValueResponse>(pr::VIEW_ID, pr::VIEW_DESC),
        spec::<pr::CreateRequest, TextResponse>(pr::CREATE_ID, pr::CREATE_DESC),
        spec::<pr::EditRequest, TextResponse>(pr::EDIT_ID, pr::EDIT_DESC),
        spec::<pr::MergeRequest, TextResponse>(pr::MERGE_ID, pr::MERGE_DESC),
        spec::<pr::CommentRequest, TextResponse>(pr::COMMENT_ID, pr::COMMENT_DESC),
        spec::<pr::ReviewRequest, TextResponse>(pr::REVIEW_ID, pr::REVIEW_DESC),
        spec::<pr::DiffRequest, DiffResponse>(pr::DIFF_ID, pr::DIFF_DESC),
        spec::<pr::ChecksRequest, ValueResponse>(pr::CHECKS_ID, pr::CHECKS_DESC),
        spec::<issue::ListRequest, ValueResponse>(issue::LIST_ID, issue::LIST_DESC),
        spec::<issue::ViewRequest, ValueResponse>(issue::VIEW_ID, issue::VIEW_DESC),
        spec::<issue::CreateRequest, TextResponse>(issue::CREATE_ID, issue::CREATE_DESC),
        spec::<issue::EditRequest, TextResponse>(issue::EDIT_ID, issue::EDIT_DESC),
        spec::<issue::CommentRequest, TextResponse>(issue::COMMENT_ID, issue::COMMENT_DESC),
        spec::<issue::CloseRequest, TextResponse>(issue::CLOSE_ID, issue::CLOSE_DESC),
        spec::<repo::ViewRequest, ValueResponse>(repo::VIEW_ID, repo::VIEW_DESC),
        spec::<repo::ListRequest, ValueResponse>(repo::LIST_ID, repo::LIST_DESC),
        spec::<actions::RunListRequest, ValueResponse>(
            actions::RUN_LIST_ID,
            actions::RUN_LIST_DESC,
        ),
        spec::<actions::RunViewRequest, ValueResponse>(
            actions::RUN_VIEW_ID,
            actions::RUN_VIEW_DESC,
        ),
        spec::<actions::RunRerunRequest, TextResponse>(
            actions::RUN_RERUN_ID,
            actions::RUN_RERUN_DESC,
        ),
        spec::<actions::RunCancelRequest, TextResponse>(
            actions::RUN_CANCEL_ID,
            actions::RUN_CANCEL_DESC,
        ),
        spec::<actions::WorkflowListRequest, ValueResponse>(
            actions::WORKFLOW_LIST_ID,
            actions::WORKFLOW_LIST_DESC,
        ),
        spec::<actions::WorkflowRunRequest, TextResponse>(
            actions::WORKFLOW_RUN_ID,
            actions::WORKFLOW_RUN_DESC,
        ),
        spec::<release::ListRequest, ValueResponse>(release::LIST_ID, release::LIST_DESC),
        spec::<release::ViewRequest, ValueResponse>(release::VIEW_ID, release::VIEW_DESC),
        spec::<release::CreateRequest, TextResponse>(release::CREATE_ID, release::CREATE_DESC),
        spec::<search::ReposRequest, ValueResponse>(search::REPOS_ID, search::REPOS_DESC),
        spec::<search::IssuesRequest, ValueResponse>(search::ISSUES_ID, search::ISSUES_DESC),
        spec::<search::PrsRequest, ValueResponse>(search::PRS_ID, search::PRS_DESC),
        spec::<search::CodeRequest, ValueResponse>(search::CODE_ID, search::CODE_DESC),
        spec::<passthrough::ExecRequest, GhOutcome>(passthrough::EXEC_ID, passthrough::EXEC_DESC),
        spec::<passthrough::ApiRequest, ValueResponse>(passthrough::API_ID, passthrough::API_DESC),
    ]
}
