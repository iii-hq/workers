//! Dependency-free, lenient JSON Schema subset used to validate live-binding
//! configuration and query payloads against the schemas a provider registered
//! with the engine (`trigger_request_format`, function `request_schema`).
//!
//! Supported keywords: boolean schemas, `type`, `enum`, `const`, `required`,
//! `properties`, `additionalProperties`, `items`, `minLength`, `maxLength`,
//! `minimum`, `maximum`, `anyOf`, `oneOf` (treated as `anyOf`), `allOf`, and
//! local `$ref` into `#/definitions/*` or `#/$defs/*`. Every other keyword is
//! ignored, so this check is never stricter than the schema itself; the
//! provider's own registration-time validation stays authoritative.

use serde_json::Value;

const MAX_DEPTH: usize = 32;

/// Validate `value` against `schema`. `label` prefixes error messages.
pub fn validate(label: &str, schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, schema, value, "", 0).map_err(|error| {
        if error.starts_with(':') {
            format!("{label}{error}")
        } else {
            format!("{label} {error}")
        }
    })
}

fn check(
    root: &Value,
    schema: &Value,
    value: &Value,
    at: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let schema = match schema {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return Err(format!("{}: no value is allowed here", here(at))),
        Value::Object(object) => object,
        _ => return Ok(()),
    };
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if let Some(target) = resolve_ref(root, reference) {
            check(root, target, value, at, depth + 1)?;
        }
    }
    if let Some(expected) = schema.get("type") {
        let allowed: Vec<&str> = match expected {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.iter().any(|kind| matches_type(kind, value)) {
            return Err(format!(
                "{}: expected {}, got {}",
                here(at),
                allowed.join(" or "),
                type_name(value)
            ));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            return Err(format!(
                "{}: must be one of {}",
                here(at),
                Value::Array(options.clone())
            ));
        }
    }
    if let Some(expected) = schema.get("const") {
        if expected != value {
            return Err(format!("{}: must equal {expected}", here(at)));
        }
    }
    match value {
        Value::Object(object) => {
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for key in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(key) {
                        return Err(format!("{}: missing required field `{key}`", here(at)));
                    }
                }
            }
            let properties = schema.get("properties").and_then(Value::as_object);
            for (key, item) in object {
                let child = format!("{at}/{key}");
                if let Some(property) = properties.and_then(|props| props.get(key)) {
                    check(root, property, item, &child, depth + 1)?;
                    continue;
                }
                match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        return Err(format!("{}: unsupported field `{key}`", here(at)));
                    }
                    Some(extra @ Value::Object(_)) => {
                        check(root, extra, item, &child, depth + 1)?;
                    }
                    _ => {}
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items").filter(|item| item.is_object()) {
                for (index, item) in items.iter().enumerate() {
                    check(root, item_schema, item, &format!("{at}/{index}"), depth + 1)?;
                }
            }
        }
        Value::String(text) => {
            let length = text.chars().count() as u64;
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
                if length < min {
                    return Err(format!("{}: shorter than {min} characters", here(at)));
                }
            }
            if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
                if length > max {
                    return Err(format!("{}: longer than {max} characters", here(at)));
                }
            }
        }
        Value::Number(number) => {
            if let Some(number) = number.as_f64() {
                if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
                    if number < min {
                        return Err(format!("{}: below minimum {min}", here(at)));
                    }
                }
                if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
                    if number > max {
                        return Err(format!("{}: above maximum {max}", here(at)));
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
        for item in all {
            check(root, item, value, at, depth + 1)?;
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        if let Some(options) = schema.get(keyword).and_then(Value::as_array) {
            if options.is_empty() {
                continue;
            }
            let mut first_error = None;
            let matched =
                options
                    .iter()
                    .any(|option| match check(root, option, value, at, depth + 1) {
                        Ok(()) => true,
                        Err(error) => {
                            first_error.get_or_insert(error);
                            false
                        }
                    });
            if !matched {
                return Err(first_error
                    .unwrap_or_else(|| format!("{}: matches no allowed shape", here(at))));
            }
        }
    }
    Ok(())
}

fn resolve_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    if !(pointer.starts_with("/definitions/") || pointer.starts_with("/$defs/")) {
        return None;
    }
    root.pointer(pointer)
}

fn matches_type(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|n| n.fract() == 0.0)
        }
        _ => true,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Location prefix for errors; empty at the root ("binding config: ...").
fn here(at: &str) -> String {
    at.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_schemars_style_config_schemas() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "CounterConfig",
            "type": "object",
            "required": ["counter"],
            "properties": {
                "counter": {"type": "string", "minLength": 1, "maxLength": 64},
                "min_value": {"type": ["integer", "null"], "minimum": 0},
                "mode": {"$ref": "#/definitions/Mode"}
            },
            "additionalProperties": false,
            "definitions": {"Mode": {"type": "string", "enum": ["all", "changes"]}}
        });
        assert!(validate("config", &schema, &json!({"counter": "clicks"})).is_ok());
        assert!(validate(
            "config",
            &schema,
            &json!({"counter": "clicks", "min_value": null, "mode": "all"})
        )
        .is_ok());

        let missing = validate("config", &schema, &json!({})).unwrap_err();
        assert_eq!(missing, "config: missing required field `counter`");
        let extra = validate("config", &schema, &json!({"counter": "c", "x": 1})).unwrap_err();
        assert!(extra.contains("unsupported field `x`"), "{extra}");
        let wrong = validate("config", &schema, &json!({"counter": 7})).unwrap_err();
        assert!(
            wrong.contains("/counter: expected string, got number"),
            "{wrong}"
        );
        let bad_ref = validate("config", &schema, &json!({"counter": "c", "mode": "x"}));
        assert!(bad_ref.unwrap_err().contains("must be one of"));
        let below = validate("config", &schema, &json!({"counter": "c", "min_value": -1}));
        assert!(below.unwrap_err().contains("below minimum"));
        let empty = validate("config", &schema, &json!({"counter": ""}));
        assert!(empty.unwrap_err().contains("shorter than"));
    }

    #[test]
    fn unions_booleans_and_unknown_keywords() {
        let union =
            json!({"anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "integer"}}]});
        assert!(validate("x", &union, &json!("a")).is_ok());
        assert!(validate("x", &union, &json!([1, 2])).is_ok());
        assert!(validate("x", &union, &json!([1, "b"])).is_err());
        assert!(validate("x", &json!(true), &json!({"any": 1})).is_ok());
        assert!(validate("x", &json!(false), &json!(1)).is_err());
        assert!(validate(
            "x",
            &json!({"format": "uuid", "pattern": "^z"}),
            &json!("a")
        )
        .is_ok());
        assert!(validate(
            "x",
            &json!({"$ref": "https://example.com/remote"}),
            &json!(1)
        )
        .is_ok());
    }

    #[test]
    fn recursion_is_bounded() {
        let schema = json!({"$ref": "#/definitions/Loop", "definitions": {"Loop": {"$ref": "#/definitions/Loop"}}});
        assert!(validate("x", &schema, &json!(1)).is_ok());
    }
}
