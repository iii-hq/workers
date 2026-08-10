//! Core outcome → wire response.
//!
//! The one semantic this file exists for: **a failing script is a RESPONSE,
//! not an error.** Both engines return a structured error when tenant code
//! throws; sandbox-code-runner's contract — which this worker reproduces —
//! reports that as a successful call with a non-zero `exit_code` and the
//! diagnostics on `stderr`. Errors stay reserved for infrastructure.
//!
//! Getting this backwards is how hostile input pages an operator: a tenant
//! that can reach an `internal`-shaped error at will has a way to make the
//! worker look broken.

use iii_node_core::error::NodeEngineError;
use iii_node_core::runtime::{EvalOutcome, LogLine};

use crate::error::CodeRunnerError;
use crate::functions::run::RunResponse;

/// Split level-tagged log lines into the two streams a process has.
///
/// `console.error` and `console.warn` are what Node itself routes to stderr,
/// so that is the faithful split. Rejected: everything to stdout (discards
/// the distinction the levels exist to carry), and a `[warn] ` text prefix
/// (nothing on the wire says these were ever level-tagged — that would be
/// invented data).
fn split_streams(logs: &[LogLine]) -> (String, String) {
    let mut out = String::new();
    let mut err = String::new();
    for line in logs {
        let sink = if line.level == "error" || line.level == "warn" {
            &mut err
        } else {
            &mut out
        };
        sink.push_str(&line.message);
        sink.push('\n');
    }
    (out, err)
}

/// A successful eval.
pub fn node_ok(outcome: EvalOutcome, runtime_id: Option<String>, duration_ms: u64) -> RunResponse {
    let (stdout, stderr) = split_streams(&outcome.logs);
    RunResponse {
        runtime_id,
        stdout,
        stderr,
        exit_code: 0,
        success: true,
        duration_ms,
        result: outcome.result,
    }
}

/// A failed eval: either a response (the tenant's own fault) or an error
/// (ours).
///
/// `runtime_id` is threaded in rather than read off the error, because the
/// caller who ran one-shot never held one and must not learn it.
pub fn node_err(
    err: NodeEngineError,
    runtime_id: Option<String>,
    duration_ms: u64,
) -> Result<RunResponse, CodeRunnerError> {
    match err {
        // The tenant's script threw. Its stdout still stands, and its message
        // goes where a process would put it.
        NodeEngineError::EvalFailed { message, logs } => {
            let (stdout, mut stderr) = split_streams(&logs);
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&message);
            stderr.push('\n');
            Ok(RunResponse {
                runtime_id,
                stdout,
                stderr,
                exit_code: 1,
                success: false,
                duration_ms,
                result: serde_json::Value::Null,
            })
        }
        // Everything below is infrastructure, and stays an error.
        NodeEngineError::Timeout => Err(CodeRunnerError::Timeout),
        NodeEngineError::Oom => Err(CodeRunnerError::ResourceExhausted(
            "the runtime exceeded its memory cap and was killed".into(),
        )),
        NodeEngineError::RuntimeNotFound(id) => Err(CodeRunnerError::RuntimeNotFound(id)),
        NodeEngineError::RuntimeGone(id) => Err(CodeRunnerError::Expired(id)),
        NodeEngineError::Capacity(max) => Err(CodeRunnerError::Capacity(format!(
            "all {max} runtime slots are in use"
        ))),
        NodeEngineError::NamespaceDenied { .. }
        | NodeEngineError::IdTaken(_)
        | NodeEngineError::InvalidRequest(_) => {
            Err(CodeRunnerError::InvalidRequest(err.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(level: &str, message: &str) -> LogLine {
        LogLine {
            level: level.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn console_error_and_warn_go_to_stderr_the_rest_to_stdout() {
        let (out, err) = split_streams(&[
            log("log", "one"),
            log("error", "bad"),
            log("info", "two"),
            log("warn", "careful"),
        ]);
        assert_eq!(out, "one\ntwo\n");
        assert_eq!(err, "bad\ncareful\n");
    }

    /// The whole point of this module. Mutation: return
    /// `Err(CodeRunnerError::HandlerError(message))` instead — the call then
    /// fails where sandbox-code-runner's contract says it succeeds, and the
    /// tenant's stdout is lost.
    #[test]
    fn a_thrown_script_is_a_response_with_its_stdout_intact() {
        let resp = node_err(
            NodeEngineError::EvalFailed {
                message: "Error: boom".into(),
                logs: vec![log("log", "printed first")],
            },
            None,
            7,
        )
        .expect("a tenant throw must be a response, not an error");
        assert_eq!(resp.stdout, "printed first\n");
        assert!(resp.stderr.contains("Error: boom"));
        assert_eq!(resp.exit_code, 1);
        assert!(!resp.success);
        assert_eq!(resp.result, serde_json::Value::Null);
        assert_eq!(resp.duration_ms, 7);
    }

    /// The other half of the same rule: infrastructure stays an error, so a
    /// timeout is never reported as a successful run. Mutation: map `Timeout`
    /// into the response arm with `exit_code: 124`.
    #[test]
    fn infrastructure_failures_stay_errors() {
        for (err, code) in [
            (NodeEngineError::Timeout, "code-runner::timeout"),
            (NodeEngineError::Oom, "code-runner::resource_exhausted"),
            (NodeEngineError::Capacity(4), "code-runner::capacity"),
            (
                NodeEngineError::RuntimeGone("rt-1".into()),
                "code-runner::expired",
            ),
        ] {
            let got = node_err(err, None, 0).expect_err("must stay an error");
            assert_eq!(got.code(), code);
        }
    }

    #[test]
    fn a_successful_eval_reports_zero_and_carries_its_value() {
        let resp = node_ok(
            EvalOutcome {
                result: serde_json::json!({ "n": 2 }),
                logs: vec![log("log", "hi")],
                registered: vec![],
            },
            Some("rt-1".into()),
            3,
        );
        assert_eq!(resp.exit_code, 0);
        assert!(resp.success);
        assert_eq!(resp.stdout, "hi\n");
        assert_eq!(resp.result, serde_json::json!({ "n": 2 }));
        assert_eq!(resp.runtime_id.as_deref(), Some("rt-1"));
    }
}

// --- python ---

use iii_python_core::error::{ErrorKind, PythonEngineError};
use iii_python_core::manager::RunOutput;

/// A successful python run.
///
/// The streams arrive already separated — the sandbox captured two pipes —
/// so unlike the node path there is nothing to split. `truncated` becomes a
/// final stderr line rather than a wire field, because sandbox-code-runner's
/// response has nowhere else to say it and silence would be worse.
pub fn python_ok(out: RunOutput, duration_ms: u64) -> RunResponse {
    let mut stderr = out.stderr;
    if out.truncated {
        stderr.push_str("[code-runner] output was truncated; further output is dropped\n");
    }
    RunResponse {
        runtime_id: None,
        stdout: out.stdout,
        stderr,
        exit_code: 0,
        success: true,
        duration_ms,
        result: out.result,
    }
}

/// A failed python run: a response when the tenant's own code failed, an
/// error when the sandbox did.
///
/// The split mirrors the node path exactly. `Clean(n)` is the truest process
/// shape there is — a tenant `sys.exit(3)` — and python-core reports it as a
/// `PythonException` naming the status, which is why the message is carried
/// through to stderr rather than reshaped.
pub fn python_err(e: PythonEngineError, duration_ms: u64) -> Result<RunResponse, CodeRunnerError> {
    let tenant = |e: PythonEngineError| {
        let mut stderr = e.message.clone();
        stderr.push('\n');
        if let Some(tb) = &e.traceback {
            stderr.push_str(tb);
            if !tb.ends_with('\n') {
                stderr.push('\n');
            }
        }
        RunResponse {
            runtime_id: None,
            stdout: String::new(),
            stderr,
            exit_code: 1,
            success: false,
            duration_ms,
            result: serde_json::Value::Null,
        }
    };
    match e.kind {
        ErrorKind::SyntaxError | ErrorKind::PythonException | ErrorKind::ResultTooLarge => {
            Ok(tenant(e))
        }
        ErrorKind::InvalidInput => Err(CodeRunnerError::InvalidRequest(e.message)),
        ErrorKind::RuntimeNotFound => Err(CodeRunnerError::RuntimeNotFound(e.message)),
        // Admission, not a mid-run kill: retrying later can succeed once a
        // slot frees, which is exactly the distinction `resource_exhausted`
        // exists to keep separate.
        ErrorKind::Capacity => Err(CodeRunnerError::Capacity(e.message)),
        ErrorKind::Timeout => Err(CodeRunnerError::Timeout),
        // Both are mid-run caps: retrying identically will fail identically,
        // which is exactly what `capacity` must NOT be used to say.
        ErrorKind::OutOfMemory | ErrorKind::DiskQuotaExceeded => {
            Err(CodeRunnerError::ResourceExhausted(e.message))
        }
        ErrorKind::Internal => Err(CodeRunnerError::Engine(e.message)),
    }
}

#[cfg(test)]
mod python_tests {
    use super::*;

    /// Same rule as the node side, and it must hold identically or the two
    /// languages have different contracts behind one API. Mutation: move
    /// `PythonException` into the error arm.
    #[test]
    fn a_python_exception_is_a_response_with_its_traceback_on_stderr() {
        let mut e = PythonEngineError::new(ErrorKind::PythonException, "ValueError: boom");
        e.traceback = Some("Traceback (most recent call last):\n  File \"<code>\"".into());
        let r = python_err(e, 5).expect("a tenant exception must be a response");
        assert_eq!(r.exit_code, 1);
        assert!(!r.success);
        assert!(r.stderr.contains("ValueError: boom"));
        assert!(r.stderr.contains("<code>"), "the traceback must survive");
        assert_eq!(r.result, serde_json::Value::Null);
    }

    #[test]
    fn python_infrastructure_failures_stay_errors() {
        for (kind, code) in [
            (ErrorKind::Timeout, "code-runner::timeout"),
            (ErrorKind::OutOfMemory, "code-runner::resource_exhausted"),
            (
                ErrorKind::DiskQuotaExceeded,
                "code-runner::resource_exhausted",
            ),
            (ErrorKind::Internal, "code-runner::engine"),
            (ErrorKind::InvalidInput, "code-runner::invalid_request"),
            (ErrorKind::RuntimeNotFound, "code-runner::runtime_not_found"),
            // Admission vs mid-run: `capacity` may succeed on retry,
            // `resource_exhausted` will not. Folding them together would tell
            // a caller the opposite of the truth.
            (ErrorKind::Capacity, "code-runner::capacity"),
        ] {
            let got =
                python_err(PythonEngineError::new(kind, "x"), 0).expect_err("must stay an error");
            assert_eq!(got.code(), code, "wrong mapping for {kind:?}");
        }
    }

    /// A one-shot python run has nothing to address, and truncation must be
    /// said out loud since the response has no field for it.
    #[test]
    fn a_successful_python_run_carries_its_streams_and_says_when_truncated() {
        let r = python_ok(
            RunOutput {
                result: serde_json::json!(6),
                stdout: "working\n".into(),
                stderr: String::new(),
                truncated: true,
            },
            9,
        );
        assert_eq!(r.stdout, "working\n");
        assert_eq!(r.result, serde_json::json!(6));
        assert!(r.success);
        assert!(r.runtime_id.is_none());
        assert!(r.stderr.contains("truncated"), "stderr was {:?}", r.stderr);
    }
}
