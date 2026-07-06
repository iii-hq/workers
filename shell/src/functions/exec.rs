use std::sync::Arc;

use crate::config::ShellConfig;
use crate::exec::host::parse_argv;
use crate::exec::policy::{build_overrides, scope_root_for_target};
use crate::exec_dispatch::pick_exec_backend;
use crate::functions::types::{ExecRequest, ExecResponse};

pub async fn handle(
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::IIIClient,
    req: ExecRequest,
) -> Result<ExecResponse, iii_sdk::errors::Error> {
    // Field-level type errors (wrong-type `command`, non-string `args[i]`,
    // bad `target.kind`) come from the per-field deserializers in
    // `functions::types`; they surface here as the trigger `Err` carrying
    // the actionable text the LLM needs to self-correct.
    // `args.as_ref()` preserves the legacy two-mode contract on `parse_argv`:
    //   None → tokenize `command` via shell-words (single-string path)
    //   Some(_) → use args verbatim, even if empty
    // The typed-schema path must NOT collapse "absent args" into
    // "args: []" or callers lose the shell-words path.
    // argv-parse and denylist rejections are plain Strings with no
    // S-code; via `From<String> for Error` they become the engine's
    // `invocation_failed` envelope, message naming the violation. Only the
    // backend `ExecError` below carries an S-code, surfaced as the wire `code`
    // through `From<ExecError> for Error` (Remote) so an agent can branch
    // on `error.code`.
    let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    // Gate the per-call cwd/env BEFORE picking a backend. A jail-escaping cwd
    // (S215) or an env key outside env.allow / in DANGEROUS_ENV_KEYS (S210)
    // rejects here, carrying the S-code to the wire via From<ExecError>. The
    // sandbox backend additionally rejects any populated override (host-only).
    // `scope_root` only scopes the host working directory: drop it for a sandbox
    // target so the harness-injected session dir does not surface as a host cwd
    // override and get rejected (see `scope_root_for_target`).
    let scope_root =
        scope_root_for_target(&req.target, crate::fs::scope_root(req.fs_scope.as_ref()));
    let mut overrides = build_overrides(
        req.cwd.as_deref(),
        req.env.as_ref(),
        scope_root,
        crate::fs::scope_grants(req.fs_scope.as_ref()),
        &cfg,
    )
    .map_err(iii_sdk::errors::Error::from)?;
    // stdin needs no gating (opaque input bytes); it is host-only, enforced by
    // the sandbox backend's is_empty() rejection of any populated override.
    overrides.stdin = req.stdin;

    let timeout = cfg.resolve_timeout(req.timeout_ms);

    let backend = pick_exec_backend(req.target, cfg, iii);

    let out = backend
        .run(&argv, timeout, &overrides)
        .await
        .map_err(iii_sdk::errors::Error::from)?;

    Ok(ExecResponse::from(out))
}

#[cfg(test)]
mod tests {
    use crate::target::Target;
    use serde_json::{json, Value};

    #[test]
    fn target_defaults_to_host_when_absent() {
        let payload = json!({ "command": "echo" });
        let target: Target = match payload.get("target") {
            None | Some(Value::Null) => Target::default(),
            Some(v) => serde_json::from_value(v.clone()).unwrap(),
        };
        assert_eq!(target, Target::Host);
    }

    #[test]
    fn target_field_parses_sandbox_kind() {
        let id = uuid::Uuid::new_v4();
        let payload = json!({
            "command": "ls",
            "target": { "kind": "sandbox", "sandbox_id": id.to_string() },
        });
        let target: Target = serde_json::from_value(payload["target"].clone()).unwrap();
        assert_eq!(target, Target::Sandbox { sandbox_id: id });
    }

    #[test]
    fn malformed_target_returns_error() {
        let payload = json!({
            "command": "ls",
            "target": { "kind": "moon" },
        });
        let result: Result<Target, _> = serde_json::from_value(payload["target"].clone());
        assert!(result.is_err());
    }
}
