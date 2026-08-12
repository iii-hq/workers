use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lang::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct RunRequest {
    /// Node: the body of an async function — `return x` yields the result,
    /// top-level `await` works. Python: a module — assign `result = x`. Both
    /// get a global `iii` that can trigger any bus function; node's can also
    /// register functions for the life of the runtime and reach its private
    /// scratch directory via `iii.files`, python's is synchronous and gets
    /// `/work` instead.
    pub code: String,
    /// Run in a SPECIFIC runtime, sharing its scratch directory and its
    /// globals: the run lands in that runtime and it is NOT destroyed
    /// afterwards — you own it. Omit this to run one-shot (see `keep`).
    #[serde(default)]
    pub runtime_id: Option<String>,
    /// Required when `runtime_id` is omitted — picks the engine ("node" or
    /// "python"). On an existing runtime: omit it, or pass the runtime's own
    /// language; languages cannot be mixed in one runtime.
    #[serde(default)]
    pub lang: Option<Lang>,
    /// Only meaningful when `runtime_id` is omitted. `false` (the default):
    /// one-shot — create a runtime, run `code`, return the result, destroy
    /// it. Nothing persists. `true`: leave it running; the response's
    /// `runtime_id` addresses it for later runs and is the capability
    /// `code-runner::teardown` needs to stop it.
    #[serde(default)]
    pub keep: bool,
    /// Wall-clock budget in milliseconds, clamped to the configured maximum.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

// `code` is the tenant's own program and `runtime_id` is a capability;
// neither may reach a log line through a stray `{:?}`. Hand-rolled rather
// than derived for exactly that reason — the sibling engines learned this the
// hard way, four separate times in one branch.
impl std::fmt::Debug for RunRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRequest")
            .field("code", &"<redacted>")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("lang", &self.lang)
            .field("keep", &self.keep)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Serialize, JsonSchema)]
pub struct RunResponse {
    /// Present when this run addresses a runtime that outlives the call — a
    /// `keep: true` run that SUCCEEDED, or a `runtime_id` echoed back.
    /// Absent, not null, otherwise: there is nothing to address. Treat a
    /// present value as a secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub success: bool,
    pub duration_ms: u64,
    /// The completion value: what the code returned (node) or assigned to
    /// `result` (python). Null when it returned nothing, when it returned
    /// something JSON cannot represent, or when the run failed.
    ///
    /// Always present, never skipped — unlike `runtime_id`, a null result is
    /// information ("the code returned nothing") rather than an absence.
    pub result: serde_json::Value,
}

impl std::fmt::Debug for RunResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunResponse")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("stdout", &"<redacted>")
            .field("stderr", &"<redacted>")
            .field("exit_code", &self.exit_code)
            .field("success", &self.success)
            .field("duration_ms", &self.duration_ms)
            .field("result", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `runtime_id` is a capability, so its absence must be an ABSENT key,
    /// not `null` — a caller checking `'runtime_id' in response` has to get
    /// the right answer. Mutation: drop `skip_serializing_if`.
    #[test]
    fn the_response_omits_runtime_id_entirely_when_absent() {
        let r = RunResponse {
            runtime_id: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            success: true,
            duration_ms: 1,
            result: serde_json::Value::Null,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("runtime_id").is_none(), "key must be absent: {v}");
        // `result` is the opposite case and must survive as an explicit null.
        assert_eq!(v.get("result"), Some(&serde_json::Value::Null));
    }

    /// Mutation: derive `Debug` on either struct. Both would then print the
    /// tenant's program and the capability.
    #[test]
    fn debug_redacts_the_code_and_the_capability() {
        let req = RunRequest {
            code: "SECRET_SOURCE".into(),
            runtime_id: Some("rt-secret".into()),
            lang: Some(Lang::Node),
            keep: true,
            timeout_ms: Some(5),
        };
        let s = format!("{req:?}");
        assert!(!s.contains("SECRET_SOURCE"), "{s}");
        assert!(!s.contains("rt-secret"), "{s}");
        assert!(
            s.contains("Node") && s.contains("keep"),
            "diagnostics survive: {s}"
        );
    }
}
