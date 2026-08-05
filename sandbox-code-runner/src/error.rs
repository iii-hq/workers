//! The worker's error taxonomy, plus the classifier that turns iii-sandbox
//! wire errors (S-codes embedded as JSON in the message) into code-runner
//! terms. Every variant maps to a stable `code-runner::<snake_case>` wire
//! code; handlers convert into the SDK error so the engine surfaces
//! `code: message` to callers.

use iii_sdk::errors::Error;

/// Deliberate exception to the redaction convention (same as
/// `NodeEngineError`): this type `Display`s runtime ids to the holder — the
/// caller who already supplied them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeRunnerError {
    InvalidRequest(String),
    RuntimeNotFound(String),
    /// `code-runner::teardown namespace=…` naming a namespace with no
    /// runtime backing it. Same wire code as `RuntimeNotFound` (both mean
    /// "nothing addressable by this"), a distinct variant only because the
    /// message wording differs — `RuntimeNotFound`'s hardcodes "runtime_id".
    NamespaceNotFound(String),
    /// The backing VM was reaped or stopped. By the time the caller sees
    /// this, the runtime's bus functions are unregistered and the record is
    /// gone — re-create the runtime.
    Expired(String),
    /// The daemon refused a new sandbox (its `max_concurrent_sandboxes` or a
    /// per-image cap — code-runner keeps no cap of its own).
    Capacity(String),
    /// The exec blew its in-daemon deadline.
    Timeout,
    /// The handler threw, or returned something JSON cannot represent.
    HandlerError(String),
    /// Anything else from the bus or the daemon, passed through.
    Engine(String),
}

impl CodeRunnerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "code-runner::invalid_request",
            Self::RuntimeNotFound(_) | Self::NamespaceNotFound(_) => {
                "code-runner::runtime_not_found"
            }
            Self::Expired(_) => "code-runner::expired",
            Self::Capacity(_) => "code-runner::capacity",
            Self::Timeout => "code-runner::timeout",
            Self::HandlerError(_) => "code-runner::handler_error",
            Self::Engine(_) => "code-runner::engine",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::InvalidRequest(m) => m.clone(),
            Self::RuntimeNotFound(id) => format!("unknown runtime_id {id}"),
            Self::NamespaceNotFound(ns) => format!(
                "no runtime is registered for namespace {ns:?}; register a function in it \
                 first, or pass a runtime_id instead"
            ),
            Self::Expired(id) => format!(
                "runtime {id} expired: its idle VM was reaped and its functions \
                 unregistered — call eval again without this runtime_id to boot a fresh one \
                 (its filesystem starts empty)"
            ),
            Self::Capacity(m) => m.clone(),
            Self::Timeout => "execution exceeded its deadline".into(),
            Self::HandlerError(m) => m.clone(),
            Self::Engine(m) => m.clone(),
        }
    }
}

impl std::fmt::Display for CodeRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for CodeRunnerError {}

impl From<CodeRunnerError> for Error {
    fn from(e: CodeRunnerError) -> Self {
        Error::Handler(e.to_string())
    }
}

/// How a `sandbox::*` call failed, in code-runner terms. `Gone` (not
/// `Expired`) because only the manager knows which runtime_id to name and
/// which record to clean up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxFailure {
    /// S002/S004 — the sandbox no longer exists.
    Gone,
    /// S200 — the exec blew its in-daemon deadline.
    Timeout,
    /// S400 — a daemon capacity bound; the message says which.
    Capacity(String),
    /// Everything else, diagnostic preserved.
    Other(String),
}

/// The daemon returns errors as JSON embedded in `error.message`
/// (`{type, code, message, docs_url, fix, retryable}` — see the iii-sandbox
/// README's "Error responses" section), and the bus wraps that in its own
/// framing. Scan to the first `{` and stream-parse ONE JSON value, tolerating
/// trailing text; anything that does not yield an object with a string
/// `"code"` classifies as `Other(raw)`, untouched.
pub fn classify_sandbox_error(raw: &str) -> SandboxFailure {
    let detail = raw.find('{').and_then(|start| {
        serde_json::Deserializer::from_str(&raw[start..])
            .into_iter::<serde_json::Value>()
            .next()?
            .ok()
    });
    let Some(v) = detail else {
        return SandboxFailure::Other(raw.to_string());
    };
    let Some(code) = v.get("code").and_then(|c| c.as_str()) else {
        return SandboxFailure::Other(raw.to_string());
    };
    let message = v
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    match code {
        "S002" | "S004" => SandboxFailure::Gone,
        "S200" => SandboxFailure::Timeout,
        "S400" => SandboxFailure::Capacity(message),
        // S003 (ConcurrentExec) is the daemon's own guard against two execs
        // racing one sandbox — reachable despite our own `exec_lock` because
        // the bus trigger deadline (timeout_ms + margin) outlives the
        // daemon's in-daemon exec deadline (timeout_ms): a daemon slower
        // than that margin can leave `exec_in_progress` true after
        // code-runner already released its lock. The daemon's own message
        // embeds `sandbox_id` ("concurrent exec on sandbox {id}: …"), and
        // `sandbox_id` must never reach a caller (see the module doc and
        // `RuntimeRecord`'s no-`Debug` note) — a caller holding it could
        // drive `sandbox::*` directly, bypassing this worker's mutex and
        // teardown accounting entirely. Fixed, id-free, actionable text
        // instead of passing the daemon's message through.
        "S003" => SandboxFailure::Other(
            "S003: an exec is already in flight on this runtime; retry".to_string(),
        ),
        _ => SandboxFailure::Other(format!("{code}: {message}")),
    }
}

/// How a raw `engine::functions::info` probe error should be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The target id genuinely does not exist — free to claim.
    Free,
    /// The error does not tell us whether `target` is free. Either it is not
    /// a "not found"-shaped error at all (e.g. FORBIDDEN), or — the case
    /// this exists to catch — it IS "not found"-shaped but names the PROBE
    /// function itself (`engine::functions::info`) rather than `target`,
    /// which means this engine cannot dispatch the probe at all (an older
    /// engine, or one missing the builtin). That says nothing about whether
    /// `target` is free, so it must never be read as "free".
    Inconclusive,
}

/// Classify a raw `engine::functions::info` error against the `target`
/// function id that was probed. Mirrors the disambiguation in
/// `iii/engine/src/cli_trigger/help.rs`'s `fetch_function_info`: a
/// "not found" naming the probe itself means the DISPATCHER couldn't find
/// `engine::functions::info`, not that `target` is absent.
pub fn classify_probe_error(raw: &str, target: &str) -> ProbeOutcome {
    let lower = raw.to_lowercase();
    let looks_not_found = lower.contains("not_found") || lower.contains("not found");
    if !looks_not_found {
        return ProbeOutcome::Inconclusive;
    }
    let names_target = lower.contains(&target.to_lowercase());
    let names_probe_itself = lower.contains("engine::functions::info");
    if names_probe_itself && !names_target {
        return ProbeOutcome::Inconclusive;
    }
    ProbeOutcome::Free
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_wire_strings() {
        assert_eq!(
            CodeRunnerError::InvalidRequest("x".into()).code(),
            "code-runner::invalid_request"
        );
        assert_eq!(
            CodeRunnerError::RuntimeNotFound("r".into()).code(),
            "code-runner::runtime_not_found"
        );
        assert_eq!(
            CodeRunnerError::Expired("r".into()).code(),
            "code-runner::expired"
        );
        assert_eq!(
            CodeRunnerError::Capacity("full".into()).code(),
            "code-runner::capacity"
        );
        assert_eq!(CodeRunnerError::Timeout.code(), "code-runner::timeout");
        assert_eq!(
            CodeRunnerError::HandlerError("x".into()).code(),
            "code-runner::handler_error"
        );
        assert_eq!(
            CodeRunnerError::Engine("x".into()).code(),
            "code-runner::engine"
        );
    }

    #[test]
    fn display_is_code_colon_message() {
        let e = CodeRunnerError::RuntimeNotFound("rt-7".into());
        assert_eq!(
            e.to_string(),
            "code-runner::runtime_not_found: unknown runtime_id rt-7"
        );
    }

    #[test]
    fn converts_into_sdk_handler_error_preserving_code() {
        let sdk: iii_sdk::errors::Error = CodeRunnerError::Timeout.into();
        assert!(sdk.to_string().contains("code-runner::timeout"));
    }

    /// The daemon embeds `{type, code, message, …}` JSON inside the error
    /// string, and the bus wraps it in its own framing. The classifier must
    /// find and parse it through that wrapping.
    #[test]
    fn classifies_wrapped_sandbox_errors_by_s_code() {
        let wrap = |json: &str| format!("remote error (invocation_failed): handler error: {json}");
        let gone = wrap(
            r#"{"type":"SandboxNotFound","code":"S002","message":"no sandbox with that id","docs_url":"https://x/#S002","fix":null,"retryable":false}"#,
        );
        assert_eq!(classify_sandbox_error(&gone), SandboxFailure::Gone);

        let stopped =
            wrap(r#"{"type":"SandboxStopped","code":"S004","message":"reaped","retryable":false}"#);
        assert_eq!(classify_sandbox_error(&stopped), SandboxFailure::Gone);

        let timeout =
            wrap(r#"{"type":"ExecTimeout","code":"S200","message":"deadline","retryable":false}"#);
        assert_eq!(classify_sandbox_error(&timeout), SandboxFailure::Timeout);

        let full = wrap(
            r#"{"type":"ResourceLimit","code":"S400","message":"max_concurrent_sandboxes reached","retryable":true}"#,
        );
        assert_eq!(
            classify_sandbox_error(&full),
            SandboxFailure::Capacity("max_concurrent_sandboxes reached".into())
        );
    }

    /// MUST FIX 3 (final review): S003's raw daemon message embeds
    /// `sandbox_id` ("concurrent exec on sandbox {id}: …" — see
    /// `iii-worker/src/sandbox_daemon/errors.rs`'s `ConcurrentExec` Display).
    /// `sandbox_id` must never reach a caller (see the module doc's design
    /// invariant), so the classifier must swap in fixed, id-free text rather
    /// than passing the daemon's own message through — the redaction on the
    /// ERROR path, not just `Debug`.
    #[test]
    fn s003_classifies_with_fixed_id_free_text() {
        let sandbox_id = "11111111-2222-3333-4444-555555555555";
        let wrap = |json: &str| format!("remote error (invocation_failed): handler error: {json}");
        let raw = wrap(&format!(
            r#"{{"type":"validation","code":"S003","message":"concurrent exec on sandbox {sandbox_id}: an exec is already in flight. Exec is serialized one-at-a-time per sandbox","docs_url":"https://x/#S003","fix":null,"retryable":false}}"#
        ));
        match classify_sandbox_error(&raw) {
            SandboxFailure::Other(msg) => {
                assert!(
                    !msg.contains(sandbox_id),
                    "the sandbox_id leaked into the classified message: {msg}"
                );
                assert!(msg.contains("S003"), "{msg}");
                assert!(msg.contains("retry"), "message should be actionable: {msg}");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    /// Unknown S-codes and boot failures pass through with code + message —
    /// the daemon's diagnostic (e.g. the S300 stderr tail) must reach the
    /// caller, not be swallowed.
    #[test]
    fn unknown_codes_pass_through_with_their_diagnostic() {
        let raw = r#"handler error: {"type":"VmBootFailed","code":"S300","message":"no /dev/kvm: stderr tail here","retryable":false}"#;
        match classify_sandbox_error(raw) {
            SandboxFailure::Other(msg) => {
                assert!(msg.contains("S300"), "{msg}");
                assert!(msg.contains("stderr tail here"), "{msg}");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    /// A non-sandbox error (no embedded JSON, or JSON without a code) is not
    /// mangled — the raw string comes back verbatim.
    #[test]
    fn non_sandbox_errors_pass_through_verbatim() {
        for raw in [
            "connection refused",
            "no such function: sandbox::exec",
            r#"weird {"not":"a sandbox error"} text"#,
        ] {
            assert_eq!(
                classify_sandbox_error(raw),
                SandboxFailure::Other(raw.to_string()),
                "{raw}"
            );
        }
    }

    /// The ordinary case: the engine dispatched the probe fine and the
    /// TARGET id genuinely isn't registered.
    #[test]
    fn probe_not_found_naming_the_target_is_free() {
        let raw = "remote error (NOT_FOUND): Function 'app::greet' is not registered.";
        assert_eq!(classify_probe_error(raw, "app::greet"), ProbeOutcome::Free);
    }

    /// MUST FIX 2 (final review): a "not found" naming the PROBE function
    /// itself — `engine::functions::info` — means this engine cannot
    /// dispatch the probe at all (an older engine, or one missing the
    /// builtin). That says nothing about whether the target id is free, so
    /// this must NOT classify as `Free`. Before this fix, the old
    /// lowercase-substring matcher could not tell this apart from the
    /// "target genuinely absent" case above — both contain "not found" — so
    /// it treated an unprobeable engine as "id is free" and let
    /// `RuntimeManager::publish` register over a live production function on
    /// the bus with no error to either worker. This is the failing-open
    /// direction this test exists to catch.
    #[test]
    fn probe_not_found_naming_the_probe_itself_is_inconclusive() {
        let raw = "remote error (function_not_found): Function engine::functions::info not found";
        assert_eq!(
            classify_probe_error(raw, "app::greet"),
            ProbeOutcome::Inconclusive,
            "an engine that cannot dispatch the probe itself must never be read as \
             'the target id is free'"
        );
    }

    /// A non-"not found" error (RBAC denial, transport failure, …) is
    /// unverifiable and must fail closed, same as before this fix.
    #[test]
    fn probe_non_not_found_error_is_inconclusive() {
        let raw = "remote error: FORBIDDEN: rbac denies functions.info";
        assert_eq!(
            classify_probe_error(raw, "app::greet"),
            ProbeOutcome::Inconclusive
        );
    }
}
