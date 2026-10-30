//! JSON as Python's `json.dumps` writes it with its default separators: a
//! space after every `:` and `,`. Published chat templates (DeepSeek's
//! encoder, GLM's, Qwen's) serialize tool schemas and call arguments this
//! way, so a count that renders their prompt has to as well; serde_json's
//! compact form is shorter by a token or so per field.

use serde_json::Value;

/// `json.dumps(value, ensure_ascii=False)` for a value serde_json decoded:
/// keys in the map's order, non-ASCII kept as is.
pub fn python_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let fields: Vec<String> = map
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", Value::from(key.as_str()), python_json(value))
                })
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(python_json).collect();
            format!("[{}]", items.join(", "))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn separators_carry_a_space_and_nesting_follows() {
        assert_eq!(
            python_json(&json!({"a": [1, "x", null], "b": {"c": true}, "é": "ü"})),
            r#"{"a": [1, "x", null], "b": {"c": true}, "é": "ü"}"#
        );
        assert_eq!(python_json(&json!([])), "[]");
        assert_eq!(python_json(&json!({})), "{}");
    }
}
