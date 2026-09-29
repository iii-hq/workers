//! Guest requests use this worker's bus authority. Keep operator credentials
//! and private state out of that delegation, including deferred trigger calls.
//! This is a host-bridge restriction, not an engine-wide sandbox or ACL.
use serde_json::Value;

fn validate_function(function_id: &str) -> Result<(), String> {
    // Match the harness agent-call boundary. Scope guards on private state do
    // not authenticate the caller, and `internal` metadata is not authorization.
    if function_id == "state::claim-namespace"
        || [
            "harness::state::",
            "provider::openai-codex::login::",
            "provider::openai-codex::auth::",
            "provider-openai-codex::state::",
        ]
        .iter()
        .any(|prefix| function_id.starts_with(prefix))
    {
        return Err(format!(
            "operator-only function `{function_id}` is not available through code-runner"
        ));
    }
    Ok(())
}

pub(crate) fn validate_trigger_target(function_id: &str) -> Result<(), String> {
    validate_function(function_id)?;
    // A fired registration bypasses this host bridge and could install a
    // second binding whose target was never checked here.
    if matches!(
        function_id,
        "engine::register_trigger" | "engine::unregister_trigger"
    ) {
        return Err(format!(
            "operator-only trigger target `{function_id}` cannot be bound through code-runner; \
             invoke trigger controls directly so their targets can be checked"
        ));
    }
    Ok(())
}

pub(crate) fn validate_call(function_id: &str, payload: &Value) -> Result<(), String> {
    validate_function(function_id)?;
    if function_id == "engine::register_trigger" {
        let target = payload
            .get("function_id")
            .and_then(Value::as_str)
            .ok_or("invalid trigger registration: `function_id` must name a target")?;
        validate_trigger_target(target)?;
    }
    Ok(())
}
