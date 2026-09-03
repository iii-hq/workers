use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct RunRequest {
    /// Source run as a file by a fresh interpreter process; variables do not survive
    /// between runs. A global `iii` (iii-sdk client) is in scope.
    // Functions registered through `iii.registerFunction` die with the process;
    // persistent ones go through sandbox-code-runner::register_function.
    pub code: String,
    /// Run in this existing runtime (from a keep: true run), sharing its filesystem; it is
    /// not stopped afterwards. Omit to run one-shot.
    #[serde(default)]
    pub runtime_id: Option<String>,
    /// Required when `runtime_id` is omitted; on an existing runtime omit it or pass the
    /// runtime's own language.
    #[serde(default)]
    pub lang: Option<Lang>,
    /// When `runtime_id` is omitted: `true` leaves the VM running and returns its
    /// `runtime_id`; `false` destroys it after the run.
    #[serde(default)]
    pub keep: bool,
    /// Wall-clock budget in milliseconds, clamped to the configured maximum.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

// `code` is tenant-authored source; `runtime_id` — when present — is a
// capability. Hand-rolled `Debug` keeps both out of `{:?}`.
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
    /// Present when this run addresses a runtime that outlives the call:
    /// the `runtime_id` you passed in, or — when you passed `keep: true`
    /// with no `runtime_id` — the one just minted for the VM this call left
    /// running. `None` on the default one-shot path: the VM is already gone
    /// by the time this response is sent, so there is nothing to address.
    /// Treat a present value as a secret: it is the capability to run into
    /// or tear down that runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub success: bool,
    pub duration_ms: u64,
}

impl std::fmt::Debug for RunResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunResponse")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("exit_code", &self.exit_code)
            .field("success", &self.success)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_code_or_the_runtime_id() {
        let req = RunRequest {
            code: "SECRET_TENANT_SOURCE_1234".into(),
            runtime_id: Some("rt-secret-capability".into()),
            lang: Some(Lang::Node),
            keep: false,
            timeout_ms: None,
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("SECRET_TENANT_SOURCE_1234"),
            "{rendered}"
        );
        assert!(!rendered.contains("rt-secret-capability"), "{rendered}");
        assert!(
            rendered.contains("Node"),
            "non-secrets still show: {rendered}"
        );
    }

    #[test]
    fn response_debug_does_not_leak_the_runtime_id() {
        let res = RunResponse {
            runtime_id: Some("rt-secret-capability".into()),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            success: true,
            duration_ms: 1,
        };
        let rendered = format!("{res:?}");
        assert!(!rendered.contains("rt-secret-capability"), "{rendered}");
    }

    #[test]
    fn response_omits_runtime_id_on_the_wire_when_absent() {
        let res = RunResponse {
            runtime_id: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            success: true,
            duration_ms: 1,
        };
        let value = serde_json::to_value(&res).unwrap();
        assert!(
            !value.as_object().unwrap().contains_key("runtime_id"),
            "a one-shot response must not carry a null runtime_id key: {value}"
        );
    }
}
