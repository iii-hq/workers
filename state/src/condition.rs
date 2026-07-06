//! Per-trigger-binding conditional gate: a `condition_function_id` that runs
//! before a state change-event is fanned out to its bound handler.
//!
//! The scope/key pre-filter on a trigger binding runs first; if it passes,
//! `check_condition` decides whether the event actually reaches the handler.
//! Only an explicit `false` return blocks delivery -- `null`/no result passes,
//! and an invocation error skips the binding (and is logged) rather than
//! blocking it. This worker uses `IIIClient::trigger`, which returns
//! `Result<Value, Error>`; `Value::Null` stands in for "condition function
//! returned no result".

use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use serde_json::Value;

/// Evaluates a condition function against the provided data.
///
/// Returns:
/// - `Ok(true)` -- proceed with the handler (condition passed, returned a
///   non-`false` value, or returned `null`/no value)
/// - `Ok(false)` -- skip the handler (condition explicitly returned `false`)
/// - `Err(Error)` -- condition function invocation failed
pub async fn check_condition(
    iii: &IIIClient,
    condition_function_id: &str,
    data: Value,
    timeout_ms: u64,
) -> Result<bool, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: condition_function_id.to_string(),
            payload: data,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await?;

    if result.is_null() {
        tracing::warn!(
            condition_function_id = %condition_function_id,
            "Condition function returned no result"
        );
        return Ok(true);
    }

    Ok(result.as_bool() != Some(false))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    /// Mirrors `check_condition`'s truthiness mapping without needing a live
    /// `IIIClient`: `null` (the "no result" case) and anything not exactly
    /// `false` pass; `false` blocks. The full call path (including the
    /// `IIIClient::trigger` round-trip and the timeout/error arms) is covered
    /// by `condition_false_blocks_null_passes` in `tests/e2e_state.rs`.
    fn maps(value: Value) -> bool {
        if value.is_null() {
            true
        } else {
            value.as_bool() != Some(false)
        }
    }

    #[test]
    fn true_passes() {
        assert!(maps(json!(true)));
    }

    #[test]
    fn false_blocks() {
        assert!(!maps(json!(false)));
    }

    #[test]
    fn null_passes() {
        assert!(maps(Value::Null));
    }

    #[test]
    fn non_bool_passes() {
        assert!(maps(json!("hello")));
        assert!(maps(json!(42)));
        assert!(maps(json!({})));
    }
}
