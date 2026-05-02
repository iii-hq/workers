use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod handler;

pub const SET_ID: &str = "flag::set";
pub const CLEAR_ID: &str = "flag::clear";
pub const IS_SET_ID: &str = "flag::is_set";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagRequest {
    pub name: String,
    pub session_id: String,
}

pub fn flag_key(name: &str, session_id: &str) -> String {
    match name {
        "abort" => format!("session/{session_id}/abort_signal"),
        other => format!("session/{session_id}/flags/{other}"),
    }
}

pub fn build_set_op() -> Value {
    serde_json::json!({ "type": "set", "path": "", "value": true })
}

pub fn build_clear_op() -> Value {
    serde_json::json!({ "type": "set", "path": "", "value": false })
}

pub fn is_set(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

pub fn register_with_iii(iii: &iii_sdk::III) {
    handler::register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn abort_flag_key_matches_existing_runtime() {
        assert_eq!(flag_key("abort", "abc"), "session/abc/abort_signal");
    }

    #[test]
    fn non_abort_flag_key_uses_flags_namespace() {
        assert_eq!(flag_key("paused", "abc"), "session/abc/flags/paused");
    }

    #[test]
    fn set_builds_true_state_value() {
        let op = build_set_op();

        assert_eq!(op["type"], "set");
        assert_eq!(op["path"], "");
        assert_eq!(op["value"], true);
    }

    #[test]
    fn clear_builds_false_state_value() {
        let op = build_clear_op();

        assert_eq!(op["type"], "set");
        assert_eq!(op["path"], "");
        assert_eq!(op["value"], false);
    }

    #[test]
    fn is_set_treats_missing_value_as_false() {
        assert!(!is_set(None));
    }

    #[test]
    fn is_set_reads_boolean_values() {
        assert!(is_set(Some(&json!(true))));
        assert!(!is_set(Some(&json!(false))));
        assert!(!is_set(Some(&json!("true"))));
    }
}
