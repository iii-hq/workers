//! Guest requests use this worker's bus authority. Keep operator credentials
//! and private state out of that delegation, including fire-and-forget calls
//! and observers. This is a host-bridge restriction, not an engine-wide
//! sandbox or ACL.

fn validate_function(function_id: &str) -> Result<(), String> {
    // Match the harness agent-call boundary. Scope guards on private state do
    // not authenticate the caller, and `internal` metadata is not authorization.
    //
    // `engine::observers::dispose` is here because every guest shares this
    // worker's one connection, and the engine lets the connection that
    // created an observer dispose it: through a raw call, one tenant could
    // remove another's observers, or this worker's own. A guest disposes its
    // own observers through the handle `iii.observe` returned.
    if function_id == "state::claim-namespace"
        || function_id == "engine::observers::dispose"
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

/// Vet a guest observer. Observing an operator-only function would hand its
/// inputs and outputs to a target the guest chose; targeting one would run
/// it with every call the guest's source makes.
pub(crate) fn validate_observe(source: &str, target: &str) -> Result<(), String> {
    validate_function(source)?;
    validate_function(target)
}

pub(crate) fn validate_call(function_id: &str) -> Result<(), String> {
    validate_function(function_id)
}
