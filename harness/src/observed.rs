//! What an observer target of this worker is called with, and the event it
//! acts on.

/// The payload an observer target receives: the `Observed` envelope its
/// observer's `capture: [output]` filled.
pub type Delivered<T> = iii_sdk::Observed<serde_json::Value, T>;

/// The observed call's event, the part the target acts on: its output.
/// `None` when the activation carried no output.
pub fn delivered_event<T>(delivered: Delivered<T>) -> Option<T> {
    delivered.output
}

/// An `Observed` envelope of a successful call of `function_id` that
/// returned `output`, as the engine sends it for `capture: [output]`.
#[cfg(test)]
pub fn envelope(function_id: &str, output: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "observer_id": "obs-1",
        "activation_id": "act-1",
        "function_id": function_id,
        "invocation_id": "inv-1",
        "status": "success",
        "started_at_ms": 1,
        "duration_ms": 0.5,
        "output": output,
    })
}
