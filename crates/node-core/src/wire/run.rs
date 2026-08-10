use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::runtime::LogLine;

#[derive(Deserialize, JsonSchema)]
pub struct RunRequest {
    /// JavaScript source. Wrapped in an async IIFE, so `return` sets the
    /// result and top-level `await` works.
    pub code: String,
    /// Run in a SPECIFIC runtime, sharing its globals and registrations —
    /// that runtime is NEVER disposed by this call, you own it. Omit this to
    /// run one-shot (see `keep`).
    #[serde(default)]
    pub runtime_id: Option<String>,
    /// Required prefix for ids this runtime may register, normalized to end
    /// in `::` — `app`, `app:`, and `app::` all mean `app::`. Must be one
    /// worker name: lowercase letters, digits, `.`, `_` or `-`, at most 64
    /// bytes, and no inner `::`; ids may still nest below it, so
    /// `myapp::v2::save` is fine under `myapp::`. Accepted only when creating
    /// a runtime; defaults to `node-engine::<runtime_id>::`, where
    /// `runtime_id` is itself `rt-<uuid>`.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Only meaningful when `runtime_id` is omitted. `false` (the default):
    /// one-shot — create a runtime, evaluate `code`, and dispose the
    /// runtime; nothing persists, and the response carries no `runtime_id`.
    /// `true`: create a runtime and leave it running; the response's
    /// `runtime_id` addresses it for later calls and is the capability
    /// `node-engine::teardown` needs to stop it.
    #[serde(default)]
    pub keep: bool,
    /// Wall-clock budget in milliseconds, clamped to the configured maximum.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

// `code` is tenant-authored JavaScript — the same secret `FunctionDef::handler`
// hand-rolls `Debug` to keep out of `{:?}`. `runtime_id` — when present — is a
// capability, same rule as `RegisterRequest`'s identical optional field.
impl std::fmt::Debug for RunRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRequest")
            .field("code", &"<redacted>")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("namespace", &self.namespace)
            .field("keep", &self.keep)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Serialize, JsonSchema)]
pub struct RunResponse {
    /// Present only when the runtime outlives this call (`keep: true`, or a
    /// `runtime_id` you passed in). Treat as a secret: it is the capability
    /// to run into or tear down that runtime. Absent — not null — on the
    /// default one-shot path: the runtime is already disposed by the time
    /// this response is sent, so there is nothing left to address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// The completion value, JSON-encoded. Null when the code returned
    /// nothing or something JSON cannot represent.
    pub result: serde_json::Value,
    pub logs: Vec<LogLine>,
    /// Engine function ids registered during this eval.
    pub registered: Vec<String>,
}

// `runtime_id` is a capability — its own doc comment says so, and it is a
// direct sibling of `RegisterResponse`, which redacts the identical field.
// `.as_ref().map(...)` rather than a bare `"<redacted>"`: `None` must still
// print as `None`, or a one-shot response's debug output would lie about
// there being a secret to redact.
impl std::fmt::Debug for RunResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunResponse")
            .field(
                "runtime_id",
                &self.runtime_id.as_ref().map(|_| "<redacted>"),
            )
            .field("result", &self.result)
            .field("logs", &self.logs)
            .field("registered", &self.registered)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The finding this guards: a derived `Debug` on `RunRequest` would print
    /// both tenant-authored `code` and, when present, the `runtime_id`
    /// capability the moment anything formats it with `{:?}`.
    #[test]
    fn debug_does_not_leak_code_or_the_runtime_id() {
        let req = RunRequest {
            code: "SECRET_TENANT_SOURCE_1234".into(),
            runtime_id: Some("rt-secret-capability".into()),
            namespace: Some("app::".into()),
            keep: false,
            timeout_ms: None,
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("SECRET_TENANT_SOURCE_1234"),
            "leaked tenant code: {rendered}"
        );
        assert!(
            !rendered.contains("rt-secret-capability"),
            "leaked the runtime_id: {rendered}"
        );
        assert!(
            rendered.contains("app::"),
            "non-secret fields should still show: {rendered}"
        );
    }

    /// `RunResponse`'s own doc comment says to treat `runtime_id` as a
    /// secret; a derived `Debug` would print it verbatim.
    #[test]
    fn debug_does_not_leak_the_runtime_id() {
        let res = RunResponse {
            runtime_id: Some("rt-secret-capability".into()),
            result: serde_json::json!({ "a": 1 }),
            logs: vec![],
            registered: vec!["app::a".into()],
        };
        let rendered = format!("{res:?}");
        assert!(
            !rendered.contains("rt-secret-capability"),
            "leaked the runtime_id: {rendered}"
        );
        assert!(
            rendered.contains("app::a"),
            "non-secret fields should still show: {rendered}"
        );
    }

    /// A one-shot response's `runtime_id` must be ABSENT on the wire, not
    /// `null`: nothing survives the call to address, so the key itself must
    /// not be there to imply otherwise (`skip_serializing_if`).
    #[test]
    fn response_omits_runtime_id_on_the_wire_when_absent() {
        let res = RunResponse {
            runtime_id: None,
            result: serde_json::json!(1),
            logs: vec![],
            registered: vec![],
        };
        let value = serde_json::to_value(&res).unwrap();
        assert!(
            !value.as_object().unwrap().contains_key("runtime_id"),
            "a one-shot response must not carry a null runtime_id key: {value}"
        );
    }
}
