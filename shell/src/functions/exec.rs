use std::sync::Arc;

use crate::config::ShellConfig;
use crate::exec::host::parse_argv;
use crate::exec_dispatch::{err_to_string, pick_exec_backend};
use crate::functions::types::{ExecRequest, ExecResponse};

pub async fn handle(
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::III,
    req: ExecRequest,
) -> Result<ExecResponse, String> {
    // Field-level type errors (wrong-type `command`, non-string `args[i]`,
    // bad `target.kind`) come from the per-field deserializers in
    // `functions::types`; they surface here as the trigger `Err` carrying
    // the actionable text the LLM needs to self-correct.
    // `args.as_ref()` preserves the legacy two-mode contract on `parse_argv`:
    //   None → tokenize `command` via shell-words (single-string path)
    //   Some(_) → use args verbatim, even if empty
    // The typed-schema migration must NOT collapse "absent args" into
    // "args: []" or callers lose the shell-words path.
    let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    let timeout = cfg.resolve_timeout(req.timeout_ms);

    let backend = pick_exec_backend(req.target, cfg, iii);

    let out = backend.run(&argv, timeout).await.map_err(err_to_string)?;

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
