use std::sync::Arc;

use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::config::ShellConfig;
use crate::exec::host::parse_argv;
use crate::exec_dispatch::{err_to_string, pick_exec_backend};
use crate::functions::approval_bypass::{marker_wellformed, validate_approved_record_for_bypass};
use crate::functions::types::{ExecRequest, ExecResponse};

const FN_APPROVAL_LOOKUP_RECORD: &str = "approval::lookup_record";

async fn fetch_approval_record(iii: &iii_sdk::III, session_id: &str, call_id: &str) -> Result<Value, String> {
    let v = iii
        .trigger(TriggerRequest {
            function_id: FN_APPROVAL_LOOKUP_RECORD.into(),
            payload: json!({
                "session_id": session_id,
                "function_call_id": call_id,
            }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|e| e.to_string())?;
    if v.is_null() {
        Err("__from_approval marker without valid pending approval record".into())
    } else {
        Ok(v)
    }
}

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
    let handler_id = "shell::exec";
    let argv = if let Some(ref marker) = req.from_approval {
        marker_wellformed(marker)?;
        let rec = fetch_approval_record(&iii, &marker.session_id, &marker.call_id).await?;
        validate_approved_record_for_bypass(&rec, handler_id, &req.command, &req.args)?;
        let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;
        if let Some(reason) = cfg.denylist_hit_reason(&argv) {
            tracing::error!(
                reason = %reason,
                "post-approval defense-in-depth: denylisted argv on approval bypass path"
            );
            return Err(format!(
                "post-approval defense-in-depth: {}",
                reason
            ));
        }
        argv
    } else {
        let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;
        cfg.is_command_allowed(&argv)?;
        argv
    };

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
