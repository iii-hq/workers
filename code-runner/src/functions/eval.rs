use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runner::Lang;

#[derive(Deserialize, JsonSchema)]
pub struct EvalRequest {
    /// Source run as a whole file by a fresh interpreter process. Variables
    /// do NOT survive between evals; whether files and installed packages
    /// do depends on the path below. A global `iii` is in scope — the real
    /// iii-sdk client, lazily connected to the engine on first use:
    /// `await iii.trigger({ function_id, payload })` in Node,
    /// `iii.trigger({'function_id': ..., 'payload': ...})` in Python.
    /// Functions registered with `iii.registerFunction` live only until
    /// this process exits — use code-runner::register_function (callable
    /// through `iii.trigger`) for one that persists.
    pub code: String,
    /// Evaluate in a SPECIFIC runtime, sharing its filesystem: the write and
    /// the run land in that VM, and it is NOT stopped afterwards — you own
    /// it. Omit this to run one-shot (see `keep`).
    #[serde(default)]
    pub runtime_id: Option<String>,
    /// Required when `runtime_id` is omitted — picks the sandbox image
    /// ("node" or "python"). On an existing runtime: omit it, or pass the
    /// runtime's own language; languages cannot be mixed in one runtime.
    #[serde(default)]
    pub lang: Option<Lang>,
    /// Only meaningful when `runtime_id` is omitted. `false` (the default):
    /// one-shot — boot a VM, run `code`, return the result, destroy the VM.
    /// Nothing persists: no files, no installed packages. `true`: boot a VM
    /// and leave it running; the response's `runtime_id` addresses it for
    /// later evals (pass it back to keep working in the same filesystem) and
    /// is the capability `code-runner::teardown` needs to stop it. Every
    /// runtime boots with outbound network (the `iii` global's engine link
    /// needs it), so npm/pip installs work on any path.
    #[serde(default)]
    pub keep: bool,
    /// Wall-clock budget in milliseconds, clamped to the configured maximum.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

// `code` is tenant-authored source; `runtime_id` — when present — is a
// capability. Hand-rolled `Debug` keeps both out of `{:?}`.
impl std::fmt::Debug for EvalRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalRequest")
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
pub struct EvalResponse {
    /// Present when this eval addresses a runtime that outlives the call:
    /// the `runtime_id` you passed in, or — when you passed `keep: true`
    /// with no `runtime_id` — the one just minted for the VM this call left
    /// running. `None` on the default one-shot path: the VM is already gone
    /// by the time this response is sent, so there is nothing to address.
    /// Treat a present value as a secret: it is the capability to eval into
    /// or tear down that runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub success: bool,
    pub duration_ms: u64,
}

impl std::fmt::Debug for EvalResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalResponse")
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
        let req = EvalRequest {
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
        let res = EvalResponse {
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
        let res = EvalResponse {
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
