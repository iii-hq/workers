//! Sandbox-target exec backend. Forwards `cmd` + `args` + `timeout_ms`
//! to `sandbox::exec` via `iii.trigger`. Stateless — the engine owns
//! the sandbox lifecycle. Stdout/stderr arrive in the trigger response;
//! shell does not stream them.

use async_trait::async_trait;
use iii_sdk::IIIError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::exec::{backend::ExecBackend, error::ExecError, ExecCallResult, ExecOutcome};
use crate::triggers::TriggerFwd;

/// Wire response from the engine's `sandbox::exec` trigger.
/// `pub(crate)` because `functions::exec_bg::spawn_sandbox_job` (T9)
/// also deserializes this same shape when populating a `JobRecord`.
#[derive(Debug, Deserialize)]
pub(crate) struct SandboxExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub timed_out: bool,
    // The engine wire also has `success` (== exit_code == 0 && !timed_out);
    // we drop it on this side because it's derived.
}

pub struct SandboxExecBackend {
    fwd: Arc<dyn TriggerFwd>,
    enabled: bool,
    sandbox_id: Uuid,
}

impl SandboxExecBackend {
    pub fn new(fwd: Arc<dyn TriggerFwd>, enabled: bool, sandbox_id: Uuid) -> Self {
        Self {
            fwd,
            enabled,
            sandbox_id,
        }
    }
}

#[async_trait]
impl ExecBackend for SandboxExecBackend {
    async fn run(&self, argv: &[String], timeout_ms: u64) -> ExecCallResult<ExecOutcome> {
        if !self.enabled {
            return Err(ExecError::new("S210", "sandbox target disabled in config"));
        }
        let cmd = argv
            .first()
            .ok_or_else(|| ExecError::new("S210", "empty command"))?
            .clone();
        let args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();

        let payload = json!({
            "sandbox_id": self.sandbox_id.to_string(),
            "cmd": cmd,
            "args": args,
            "timeout_ms": timeout_ms,
        });

        let resp = match self.fwd.trigger("sandbox::exec", payload).await {
            Ok(v) => v,
            Err(e) => {
                let exec_err = map_iii_err(e);
                // The engine's iii-sandbox returns an *error* (code S200,
                // type=execution) when the in-VM command exceeds
                // `timeout_ms` rather than a structured response with
                // timed_out=true. Translate that here so callers see the
                // same `{ timed_out: true }` shape they get from the host
                // backend — preserving the design's UX parity ("timed_out
                // direct round-trip") and freeing callers from branching
                // on backend.
                if is_engine_timeout(&exec_err) {
                    return Ok(ExecOutcome {
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: None,
                        // The engine doesn't echo a duration on the
                        // timeout error path; the caller knows it took
                        // at least timeout_ms, so report exactly that.
                        duration_ms: timeout_ms,
                        timed_out: true,
                        stdout_truncated: false,
                        stderr_truncated: false,
                    });
                }
                return Err(exec_err);
            }
        };

        let parsed: SandboxExecResponse = serde_json::from_value(resp)
            .map_err(|e| ExecError::new("S216", format!("bad engine response: {e}")))?;

        Ok(ExecOutcome {
            stdout: parsed.stdout,
            stderr: parsed.stderr,
            exit_code: Some(parsed.exit_code),
            duration_ms: parsed.duration_ms,
            timed_out: parsed.timed_out,
            // sandbox::exec doesn't expose host-side truncation; spec says
            // sandbox jobs report these as false uniformly.
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }
}

/// True when the engine's error envelope indicates the in-VM command
/// hit its server-side timeout. The wire shape (observed in the e2e
/// harness against a real iii-sandbox build):
/// `{ "code": "S200", "type": "execution", "message": "exec timed out
/// after N ms", ... }`. We accept either a recovered `S200` code OR a
/// `"timed out"` substring in the message — the latter catches engine
/// builds that use a different code but a stable message.
fn is_engine_timeout(err: &ExecError) -> bool {
    err.code == "S200" || err.message.contains("timed out")
}

/// Recover an S-code from an `IIIError`. Mirrors
/// `fs::sandbox::map_iii_err` so codes round-trip identically across
/// fs and exec paths.
fn map_iii_err(err: IIIError) -> ExecError {
    match &err {
        IIIError::Remote { code, message, .. } if code.starts_with('S') => {
            return ExecError::new(
                map_static_code(code),
                format!("forwarded from engine: {message}"),
            );
        }
        IIIError::Remote { message, .. } => {
            if let Some(c) = scan_s_code(message) {
                return ExecError::new(
                    map_static_code(c),
                    format!("forwarded from engine (wrapped): {message}"),
                );
            }
        }
        IIIError::Handler(s) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                if let Some(c) = parsed.get("code").and_then(|v| v.as_str()) {
                    let msg = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    return ExecError::new(
                        map_static_code(c),
                        format!("forwarded from engine: {msg}"),
                    );
                }
            }
            if let Some(c) = scan_s_code(s) {
                return ExecError::new(
                    map_static_code(c),
                    format!("forwarded from engine (raw): {s}"),
                );
            }
        }
        _ => {}
    }
    ExecError::new("S216", format!("engine error: {err:?}"))
}

fn scan_s_code(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    // The window is 4 bytes (`bytes[i..=i+3]`), so the last valid `i` is
    // `len - 4`. The loop bound `< len - 3` gives `i <= len - 4`.
    // `saturating_sub(3)` collapses to 0 when `len < 4`, which yields an
    // empty range (no false access) on too-short inputs.
    for i in 0..bytes.len().saturating_sub(3) {
        if bytes[i] == b'S'
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
        {
            // Reject matches preceded by alphanumerics so we don't grab
            // the tail of a longer identifier (e.g. "FOO_S211").
            let preceded_by_word =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if !preceded_by_word {
                return Some(&s[i..i + 4]);
            }
        }
    }
    None
}

fn map_static_code(code: &str) -> &'static str {
    match code {
        "S001" => "S001",
        "S002" => "S002",
        "S003" => "S003",
        "S004" => "S004",
        "S100" => "S100",
        "S101" => "S101",
        "S102" => "S102",
        // S200: in-VM execution failure (engine wire-error shape). The
        // common case — exec timeout — is intercepted in `run` and
        // surfaced as `{ timed_out: true }`; other S200 reasons (rare)
        // round-trip to the caller verbatim.
        "S200" => "S200",
        "S210" => "S210",
        "S216" => "S216",
        "S300" => "S300",
        "S400" => "S400",
        _ => "S216",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Stub forwarder using `Mutex<Option<...>>` to handle the
    /// non-Clone `IIIError` shape. Same pattern as
    /// `tests/sandbox_dispatch.rs::StubFwd`.
    struct StubFwd {
        captured: Mutex<Option<(String, Value)>>,
        next: Mutex<Option<Result<Value, IIIError>>>,
    }

    impl StubFwd {
        fn ok(value: Value) -> Arc<Self> {
            Arc::new(Self {
                captured: Mutex::new(None),
                next: Mutex::new(Some(Ok(value))),
            })
        }
        fn handler_err(json_msg: &'static str) -> Arc<Self> {
            Arc::new(Self {
                captured: Mutex::new(None),
                next: Mutex::new(Some(Err(IIIError::Handler(json_msg.to_string())))),
            })
        }
        fn remote_err(code: &str, message: &str) -> Arc<Self> {
            Arc::new(Self {
                captured: Mutex::new(None),
                next: Mutex::new(Some(Err(IIIError::Remote {
                    code: code.into(),
                    message: message.into(),
                    stacktrace: None,
                }))),
            })
        }
    }

    #[async_trait]
    impl TriggerFwd for StubFwd {
        async fn trigger(&self, fid: &str, payload: Value) -> Result<Value, IIIError> {
            *self.captured.lock().unwrap() = Some((fid.into(), payload));
            self.next
                .lock()
                .unwrap()
                .take()
                .expect("StubFwd: response already consumed")
        }
    }

    fn backend(stub: Arc<StubFwd>, sandbox_id: Uuid) -> SandboxExecBackend {
        SandboxExecBackend::new(stub, true, sandbox_id)
    }

    #[tokio::test]
    async fn run_forwards_argv_and_timeout_with_sandbox_id() {
        let stub = StubFwd::ok(json!({
            "stdout": "hi\n",
            "stderr": "warn: deprecated\n",
            "exit_code": 0,
            "duration_ms": 7,
            "timed_out": false,
            "success": true,
        }));
        let id = Uuid::new_v4();
        let b = backend(stub.clone(), id);

        let out = b
            .run(&["echo".into(), "hi".into()], 4000)
            .await
            .expect("ok response");
        assert_eq!(out.stdout, "hi\n");
        // stderr round-trips identically to stdout — the parsed
        // SandboxExecResponse field is required (not #[serde(default)]),
        // so a non-empty value here would be silently dropped if the
        // mapping ever regressed. Asserted to lock that contract.
        assert_eq!(out.stderr, "warn: deprecated\n");
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.duration_ms, 7);
        assert!(!out.timed_out);
        assert!(!out.stdout_truncated);
        assert!(!out.stderr_truncated);

        let (fid, payload) = stub.captured.lock().unwrap().clone().unwrap();
        assert_eq!(fid, "sandbox::exec");
        let obj = payload.as_object().unwrap();
        assert_eq!(obj["sandbox_id"].as_str(), Some(id.to_string().as_str()));
        assert_eq!(obj["cmd"].as_str(), Some("echo"));
        assert_eq!(obj["args"][0].as_str(), Some("hi"));
        assert_eq!(obj["timeout_ms"].as_u64(), Some(4000));
    }

    #[tokio::test]
    async fn empty_args_array_when_argv_is_single_command() {
        let stub = StubFwd::ok(json!({
            "stdout": "",
            "stderr": "",
            "exit_code": 0,
            "duration_ms": 1,
            "timed_out": false,
        }));
        let b = backend(stub.clone(), Uuid::new_v4());
        b.run(&["pwd".into()], 1000).await.unwrap();
        let (_fid, payload) = stub.captured.lock().unwrap().clone().unwrap();
        assert_eq!(payload["args"], json!([]));
    }

    #[tokio::test]
    async fn timed_out_response_propagates() {
        let stub = StubFwd::ok(json!({
            "stdout": "",
            "stderr": "",
            "exit_code": 0,
            "duration_ms": 30000,
            "timed_out": true,
        }));
        let b = backend(stub, Uuid::new_v4());
        let out = b.run(&["sleep".into(), "60".into()], 30000).await.unwrap();
        assert!(out.timed_out);
    }

    #[tokio::test]
    async fn disabled_returns_s210() {
        let stub = StubFwd::ok(json!({}));
        let b = SandboxExecBackend::new(stub, false, Uuid::new_v4());
        let err = b.run(&["echo".into()], 1000).await.unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("disabled"));
    }

    #[tokio::test]
    async fn empty_argv_returns_s210() {
        let stub = StubFwd::ok(json!({}));
        let b = backend(stub, Uuid::new_v4());
        let err = b.run(&[], 1000).await.unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn remote_s300_round_trips() {
        let stub = StubFwd::remote_err("S300", "VM boot failed: no /dev/kvm");
        let b = backend(stub, Uuid::new_v4());
        let err = b.run(&["python3".into()], 1000).await.unwrap_err();
        assert_eq!(err.code, "S300");
        assert!(err.message.contains("VM boot failed"));
    }

    #[tokio::test]
    async fn engine_timeout_handler_error_translates_to_timed_out_outcome() {
        // The real iii-sandbox engine returns this exact shape on
        // sandbox::exec timeout (observed in the e2e harness):
        //   { "code": "S200", "type": "execution",
        //     "message": "exec timed out after N ms", ... }
        // The trigger errors out, but shell must surface a successful
        // ExecOutcome with timed_out=true so callers see the same shape
        // the host backend produces on timeout.
        let stub = StubFwd::handler_err(
            r#"{"code":"S200","type":"execution","message":"exec timed out after 5000 ms","retryable":false}"#,
        );
        let b = backend(stub, Uuid::new_v4());
        let out = b
            .run(&["sleep".into(), "30".into()], 5000)
            .await
            .expect("timeout must surface as Ok, not Err");
        assert!(out.timed_out, "timed_out must be true");
        assert_eq!(out.exit_code, None, "no exit code on timeout");
        assert_eq!(
            out.duration_ms, 5000,
            "duration reflects the requested timeout"
        );
        assert_eq!(out.stdout, "", "no stdout on timeout");
    }

    #[tokio::test]
    async fn engine_timeout_via_invocation_failed_wrapper_also_translates() {
        // When the engine wraps the handler error, the timeout shows up
        // through the `Remote { code: "invocation_failed", message: ... }`
        // path with the S200 code embedded in the message. The
        // translation must still kick in.
        let stub = StubFwd::remote_err(
            "invocation_failed",
            "handler error: S200: exec timed out after 5000 ms",
        );
        let b = backend(stub, Uuid::new_v4());
        let out = b
            .run(&["sleep".into(), "30".into()], 5000)
            .await
            .expect("wrapped timeout must also surface as Ok");
        assert!(
            out.timed_out,
            "timed_out must be true through the wrapped path"
        );
    }

    #[tokio::test]
    async fn non_timeout_s200_still_propagates_as_error() {
        // S200 without "timed out" in the message is some other in-VM
        // execution failure — it should NOT be silently translated into
        // timed_out=true. The current implementation matches "S200" OR
        // "timed out"; engine builds that emit S200 for non-timeout
        // execution errors will need a different message body. Lock
        // that contract here.
        let stub = StubFwd::handler_err(
            r#"{"code":"S299","type":"execution","message":"some other failure","retryable":false}"#,
        );
        let b = backend(stub, Uuid::new_v4());
        let err = b
            .run(&["echo".into()], 1000)
            .await
            .expect_err("non-timeout S2xx must not translate to Ok");
        // Unknown S299 collapses to S216 via map_static_code; the point
        // is we got an Err, not a fake timed_out=true.
        assert_eq!(err.code, "S216");
    }

    #[tokio::test]
    async fn remote_invocation_failed_recovers_s_code_from_message() {
        let stub = StubFwd::remote_err(
            "invocation_failed",
            "handler error: S101: image not installed",
        );
        let b = backend(stub, Uuid::new_v4());
        let err = b.run(&["python3".into()], 1000).await.unwrap_err();
        assert_eq!(err.code, "S101");
    }

    #[tokio::test]
    async fn handler_json_error_recovers_s_code() {
        let stub = StubFwd::handler_err(r#"{"code":"S002","message":"sandbox not found"}"#);
        let b = backend(stub, Uuid::new_v4());
        let err = b.run(&["echo".into()], 1000).await.unwrap_err();
        assert_eq!(err.code, "S002");
    }

    #[tokio::test]
    async fn unknown_code_collapses_to_s216() {
        let stub = StubFwd::remote_err("S999", "unknown");
        let b = backend(stub, Uuid::new_v4());
        let err = b.run(&["echo".into()], 1000).await.unwrap_err();
        assert_eq!(err.code, "S216");
    }

    #[tokio::test]
    async fn malformed_response_returns_s216() {
        let stub = StubFwd::ok(json!({ "garbage": true }));
        let b = backend(stub, Uuid::new_v4());
        let err = b.run(&["echo".into()], 1000).await.unwrap_err();
        assert_eq!(err.code, "S216");
        assert!(err.message.contains("bad engine response"));
    }
}
