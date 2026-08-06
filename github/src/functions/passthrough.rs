//! The escape hatches: `github::exec` (any gh command, outcome as data) and
//! `github::api` (any GitHub REST endpoint via `gh api`).

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{argv, expect_ok, push_opt, ValueResponse, OK};
use crate::configuration::ConfigCell;
use crate::events::{self, CalledEmitter};
use crate::gh;

pub const EXEC_ID: &str = "github::exec";
pub const EXEC_DESC: &str = "Run any gh command: { args: [\"pr\", \"list\", \"-R\", \"owner/name\"], stdin?, timeout_ms? } -> { stdout, stderr, exit_code, duration_ms, timed_out, stdout_truncated, stderr_truncated }. A non-zero exit or a timeout is DATA here, not an error. The escape hatch for everything without a typed github::* function.";

pub const API_ID: &str = "github::api";
pub const API_DESC: &str = "Call any GitHub REST endpoint via gh api: { path: \"repos/OWNER/REPO/issues\", method?, fields?, body?, jq?, paginate?, timeout_ms? } -> { value: <parsed JSON> }. fields become -f key=value form fields (default method flips to POST); body is sent verbatim as the JSON request body. Non-2xx responses are errors carrying gh's stderr.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecRequest {
    /// gh argv, without the leading `gh` (e.g. ["pr", "status", "-R", "o/r"]).
    pub args: Vec<String>,
    /// Bytes piped to gh's stdin (for flags like `--body-file -`).
    pub stdin: Option<String>,
    /// Per-call timeout in ms, clamped to the configured max_timeout_ms.
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApiRequest {
    /// Endpoint path, with concrete owner/repo values (e.g. "rate_limit" or
    /// "repos/cli/cli/issues") — placeholder expansion needs a checkout the
    /// worker does not have.
    pub path: String,
    /// HTTP method. gh defaults to GET, or POST when fields/body are present.
    pub method: Option<String>,
    /// Form fields, each passed as `-f key=value` (string-typed on the wire).
    pub fields: Option<std::collections::BTreeMap<String, String>>,
    /// Raw JSON request body, piped to gh via `--input -`. Mutually exclusive
    /// with fields (gh rejects the combination).
    pub body: Option<Value>,
    /// jq expression gh applies to the response (`--jq`); keeps large
    /// responses small.
    pub jq: Option<String>,
    /// Follow pagination to the end (`--paginate`); combine with jq.
    pub paginate: Option<bool>,
    /// Per-call timeout in ms, clamped to the configured max_timeout_ms.
    pub timeout_ms: Option<u64>,
}

pub fn api_args(r: &ApiRequest) -> Vec<String> {
    let mut a = argv(["api", r.path.as_str()]);
    push_opt(&mut a, "-X", r.method.as_deref());
    if let Some(fields) = &r.fields {
        for (k, v) in fields {
            a.push("-f".to_string());
            a.push(format!("{k}={v}"));
        }
    }
    if r.body.is_some() {
        a.push("--input".to_string());
        a.push("-".to_string());
    }
    push_opt(&mut a, "--jq", r.jq.as_deref());
    if r.paginate == Some(true) {
        a.push("--paginate".to_string());
    }
    a
}

pub(crate) fn register(iii: &IIIClient, cell: &ConfigCell, emitter: &CalledEmitter) {
    let c = cell.clone();
    let em = emitter.clone();
    iii.register_function(
        EXEC_ID,
        RegisterFunction::new_async(move |req: ExecRequest| {
            let cell = c.clone();
            let emitter = em.clone();
            async move {
                let args_summary = events::summarize_args(&req.args);
                let repo = events::repo_from_args(&req.args);
                events::run_and_emit(&emitter, EXEC_ID, args_summary, repo, async move {
                    let cfg = cell.read().await.clone();
                    gh::run(&cfg, &req.args, req.stdin, req.timeout_ms)
                        .await
                        .map_err(Error::from)
                })
                .await
            }
        })
        .description(EXEC_DESC),
    );

    let c = cell.clone();
    let em = emitter.clone();
    iii.register_function(
        API_ID,
        RegisterFunction::new_async(move |req: ApiRequest| {
            let cell = c.clone();
            let emitter = em.clone();
            async move {
                let args = api_args(&req);
                let args_summary = events::summarize_args(&args);
                let repo = events::repo_from_args(&args);
                events::run_and_emit(&emitter, API_ID, args_summary, repo, async move {
                    let cfg = cell.read().await.clone();
                    let stdin = match &req.body {
                        Some(v) => Some(serde_json::to_string(v).map_err(|e| {
                            Error::Handler(format!("github::api: body does not serialize: {e}"))
                        })?),
                        None => None,
                    };
                    let out = gh::run(&cfg, &args, stdin, req.timeout_ms)
                        .await
                        .map_err(Error::from)?;
                    let out = expect_ok(API_ID, out, OK)?;
                    if out.stdout_truncated {
                        return Err(Error::Handler(
                            "github::api: response exceeded max_output_bytes and was truncated; \
                             narrow it with jq, paginate less, or raise the cap"
                                .to_string(),
                        ));
                    }
                    Ok::<_, Error>(ValueResponse {
                        value: parse_api_stdout(&out.stdout),
                    })
                })
                .await
            }
        })
        .description(API_DESC),
    );
}

/// gh api output: empty (204s) → null; JSON → parsed; anything else (a --jq
/// raw-string result) → the raw text as a JSON string.
fn parse_api_stdout(stdout: &str) -> Value {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_maps_method_fields_body_jq_paginate() {
        let r: ApiRequest = serde_json::from_value(json!({
            "path": "repos/o/r/issues",
            "method": "POST",
            "fields": { "title": "t", "body": "b" },
            "jq": ".url",
            "paginate": true
        }))
        .unwrap();
        assert_eq!(
            api_args(&r),
            vec![
                "api",
                "repos/o/r/issues",
                "-X",
                "POST",
                "-f",
                "body=b",
                "-f",
                "title=t",
                "--jq",
                ".url",
                "--paginate",
            ]
        );
    }

    #[test]
    fn api_body_pipes_via_input_dash() {
        let r: ApiRequest = serde_json::from_value(json!({
            "path": "repos/o/r/labels",
            "body": { "name": "bug" }
        }))
        .unwrap();
        assert_eq!(
            api_args(&r),
            vec!["api", "repos/o/r/labels", "--input", "-"]
        );
    }

    #[test]
    fn api_stdout_parsing_covers_empty_json_and_raw() {
        assert_eq!(parse_api_stdout(""), Value::Null);
        assert_eq!(parse_api_stdout("{\"a\":1}"), json!({"a": 1}));
        assert_eq!(
            parse_api_stdout("plain jq line\n"),
            Value::String("plain jq line".to_string())
        );
    }
}
