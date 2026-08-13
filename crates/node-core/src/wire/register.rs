use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct RegisterRequest {
    /// e.g. `my-app::greet`. The segment before the first `::` is the
    /// namespace; the first registration claims it and later ids must share
    /// it. code-runner keeps one node runtime per namespace as an
    /// implementation detail you never see or manage.
    pub function_id: String,
    /// JavaScript defining `handler(payload)`:
    /// `export function handler(p) { return p.n * 2 }`. The runtime loads
    /// the source, calls `handler`, and JSON-serialises the return value.
    pub source: String,
    /// What `engine::functions::info` shows a caller — write one.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional JSON Schema for the payload this function expects.
    /// `engine::functions::info` shows it to the next caller in place of
    /// "any". Must be a JSON object that actually constrains something —
    /// see [`validate_format`].
    #[serde(default)]
    pub request_format: Option<serde_json::Value>,
    /// Optional JSON Schema for the value the handler returns; same rules as
    /// `request_format`.
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
}

/// Serialized-size ceiling for one caller-supplied format. Schemas are
/// registry metadata every `engine::functions::info` caller downloads, not a
/// place to store data.
pub const MAX_FORMAT_BYTES: usize = 16 * 1024;

/// Top-level keywords at least one of which a supplied format must carry. A
/// JSON Schema without ANY of them (`{}`, `{"description": …}`) constrains
/// nothing — it is the "any" the caller is trying to replace — so refusing it
/// early beats publishing noise to the catalog.
const FORMAT_KEYWORDS: [&str; 10] = [
    "type",
    "properties",
    "$ref",
    "enum",
    "const",
    "anyOf",
    "oneOf",
    "allOf",
    "not",
    "items",
];

/// Validate one caller-supplied `request_format`/`response_format` value.
/// Shared by the worker wire (both languages) and `op_iii_register` (the
/// guest path), so the two trust boundaries cannot drift into different
/// rules.
pub fn validate_format(field: &str, format: &serde_json::Value) -> Result<(), String> {
    let Some(object) = format.as_object() else {
        return Err(format!(
            "{field} must be a JSON Schema OBJECT, e.g. {{\"type\": \"object\"}} — got {}",
            kind_of(format)
        ));
    };
    if !FORMAT_KEYWORDS.iter().any(|k| object.contains_key(*k)) {
        return Err(format!(
            "{field} constrains nothing — a schema needs at least one of {FORMAT_KEYWORDS:?} at \
             its top level (an empty object is the \"any\" you already get by omitting the field)"
        ));
    }
    let bytes = serde_json::to_string(format)
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if bytes > MAX_FORMAT_BYTES {
        return Err(format!(
            "{field} is {bytes} bytes serialized; the limit is {MAX_FORMAT_BYTES}"
        ));
    }
    Ok(())
}

fn kind_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

// `source` is tenant-authored code — a secret under the same rule the old
// `FunctionDef` hand-rolled Debug existed for. This is where it enters the
// crate, and nothing downstream retains a copy, so this is the only place it
// needs redacting.
impl std::fmt::Debug for RegisterRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterRequest")
            .field("function_id", &self.function_id)
            .field("source", &"<redacted>")
            .field("description", &self.description)
            .field("request_format", &self.request_format)
            .field("response_format", &self.response_format)
            .finish()
    }
}

// No secrets here — the id and namespace are public on the bus — so `Debug`
// derives.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisterResponse {
    /// The id now answering on the bus.
    pub function_id: String,
    /// The namespace it was registered under, normalised to end in `::`.
    pub namespace: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The finding this guards: a derived `Debug` on any type carrying
    /// tenant-authored source prints it the moment anything formats it with
    /// `{:?}`.
    #[test]
    fn debug_redacts_source_only() {
        let req = RegisterRequest {
            function_id: "app::greet".into(),
            source: "SECRET_TENANT_SOURCE_1234".into(),
            description: Some("greets".into()),
            request_format: None,
            response_format: None,
        };
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("SECRET_TENANT_SOURCE_1234"),
            "leaked tenant source: {rendered}"
        );
        assert!(
            rendered.contains("app::greet"),
            "non-secret fields should still show: {rendered}"
        );
        assert!(
            rendered.contains("greets"),
            "non-secret fields should still show: {rendered}"
        );
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_must_be_objects() {
        for bad in [json!("s"), json!(1), json!(true), json!(null), json!([1])] {
            let err = validate_format("request_format", &bad).unwrap_err();
            assert!(err.contains("OBJECT"), "{bad}: {err}");
        }
    }

    #[test]
    fn formats_must_constrain_something() {
        for hollow in [json!({}), json!({"description": "hi"})] {
            let err = validate_format("request_format", &hollow).unwrap_err();
            assert!(err.contains("constrains nothing"), "{hollow}: {err}");
        }
    }

    #[test]
    fn real_schema_shapes_pass() {
        for good in [
            json!({"type": "object", "properties": {"n": {"type": "number"}}}),
            json!({"$ref": "#/definitions/x"}),
            json!({"enum": [1, 2]}),
            json!({"anyOf": [{"type": "string"}, {"type": "number"}]}),
        ] {
            assert_eq!(validate_format("request_format", &good), Ok(()), "{good}");
        }
    }

    #[test]
    fn oversized_formats_are_refused() {
        let big = json!({"type": "object", "description": "x".repeat(MAX_FORMAT_BYTES)});
        let err = validate_format("response_format", &big).unwrap_err();
        assert!(err.contains("the limit is"), "{err}");
    }
}
