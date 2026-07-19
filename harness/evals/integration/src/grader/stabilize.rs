use serde_json::Value;

use super::Evidence;

/// Keep result.json stable while detailed evidence retains raw run-scoped
/// values. Diagnostic artifacts still contain every original id and
/// timestamp.
pub(super) fn stabilize_value(value: &mut Value, evidence: &Evidence) {
    match value {
        Value::String(text) => {
            replace_identity(text, &evidence.run_id, "{{run_id}}");
            replace_identity(text, &evidence.session_id, "{{session_id}}");
            if let Some(turn_id) = &evidence.turn_id {
                replace_identity(text, turn_id, "{{turn_id}}");
            }
        }
        Value::Array(items) => {
            for item in items {
                stabilize_value(item, evidence);
            }
        }
        Value::Object(map) => {
            map.remove("timestamp");
            map.remove("received_at");
            for item in map.values_mut() {
                stabilize_value(item, evidence);
            }
        }
        _ => {}
    }
}

fn replace_identity(text: &mut String, identity: &str, placeholder: &str) {
    if !identity.is_empty() && text.contains(identity) {
        *text = text.replace(identity, placeholder);
    }
}
