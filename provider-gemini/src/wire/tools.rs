//! AgentFunction (iii function invocation schemas) → Interactions `tools`,
//! and `ResponseFormat` → the top-level `response_format`.
use crate::wire::names::encode_tool_name;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{Map, Value, json};

/// Flat function entries, no `function_declarations` wrapper. The harness's
/// default `agent_call` tool has a property-less `payload: {"type":"object"}`;
/// the model fills it with arbitrary nested arguments, so it is sent as is.
pub fn functions_to_wire(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            let mut entry = json!({
                "type": "function",
                "name": encode_tool_name(&t.name),
                "description": t.description,
            });
            // A missing `parameters` key and `{}` are both accepted upstream.
            if t.parameters.is_object() {
                entry["parameters"] = sanitize_schema(&t.parameters);
            }
            entry
        })
        .collect()
}

/// `{"type":"text","mime_type":"application/json","schema":…}`, schema
/// omitted when the caller gave none.
pub fn response_format_to_wire(rf: &ResponseFormat) -> Value {
    let mut out = json!({ "type": "text", "mime_type": "application/json" });
    if let Some(schema) = &rf.schema {
        out["schema"] = sanitize_schema(schema);
    }
    out
}

/// Keys whose value is a map of names to schemas: the names are not keywords.
const NAMED_SCHEMAS: [&str; 4] = ["properties", "definitions", "$defs", "patternProperties"];
/// Keys whose value is data, not a schema.
const LITERALS: [&str; 5] = ["enum", "const", "default", "examples", "required"];

/// The two rewrites the API needs; every other keyword is accepted and
/// honoured as sent. Upstream silently ignores `allOf` (the referenced enum
/// becomes invisible to the model) and `const` (the model writes any value).
/// schemars wraps every described `$ref` property in a one-member `allOf`.
pub fn sanitize_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(sanitize_object(map)),
        Value::Array(items) => Value::Array(items.iter().map(sanitize_schema).collect()),
        other => other.clone(),
    }
}

fn sanitize_object(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        let value = match key.as_str() {
            "allOf" => continue,
            k if NAMED_SCHEMAS.contains(&k) => match value {
                Value::Object(named) => Value::Object(
                    named
                        .iter()
                        .map(|(name, s)| (name.clone(), sanitize_schema(s)))
                        .collect(),
                ),
                other => other.clone(),
            },
            k if LITERALS.contains(&k) => value.clone(),
            _ => sanitize_schema(value),
        };
        out.insert(key.clone(), value);
    }
    if let Some(constant) = out.remove("const") {
        out.entry("enum").or_insert_with(|| json!([constant]));
    }
    // shortcut: one $ref survives a multi-member allOf; schemars only emits
    // one-member wrappers.
    for member in map
        .get("allOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Value::Object(member) = sanitize_schema(member) else {
            continue;
        };
        merge_into(&mut out, member);
    }
    out
}

/// Parent keys win; `properties` and `required` are unioned.
fn merge_into(parent: &mut Map<String, Value>, member: Map<String, Value>) {
    for (key, value) in member {
        match (key.as_str(), parent.get_mut(&key), value) {
            ("properties", Some(Value::Object(own)), Value::Object(theirs)) => {
                for (name, s) in theirs {
                    own.entry(name).or_insert(s);
                }
            }
            ("required", Some(Value::Array(own)), Value::Array(theirs)) => {
                for name in theirs {
                    if !own.contains(&name) {
                        own.push(name);
                    }
                }
            }
            (_, Some(_), _) => {}
            (_, None, value) => {
                parent.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(parameters: Value) -> AgentFunction {
        AgentFunction {
            name: "agent::call".into(),
            description: "Invoke an iii function".into(),
            parameters,
            label: None,
            execution_mode: None,
        }
    }

    #[test]
    fn flat_tool_shape() {
        let wire = functions_to_wire(&[tool(json!({ "type": "object" }))]);
        assert_eq!(
            wire,
            vec![json!({
                "type": "function",
                "name": "agent__call",
                "description": "Invoke an iii function",
                "parameters": { "type": "object" },
            })]
        );
    }

    #[test]
    fn non_object_parameters_omit_the_key() {
        let wire = functions_to_wire(&[tool(Value::Null)]);
        assert!(wire[0].get("parameters").is_none());
    }

    #[test]
    fn one_member_all_of_is_merged_into_the_parent() {
        let s = json!({
            "type": "object",
            "properties": {
                "priority": { "description": "How urgent.", "allOf": [{ "$ref": "#/definitions/P" }] }
            },
            "definitions": { "P": { "type": "string", "enum": ["zq_alpha", "zq_omega"] } }
        });
        let out = sanitize_schema(&s);
        assert_eq!(
            out["properties"]["priority"],
            json!({ "description": "How urgent.", "$ref": "#/definitions/P" })
        );
        assert_eq!(out["definitions"], s["definitions"]);
    }

    #[test]
    fn multi_member_all_of_unions_properties_and_required_parent_wins() {
        let s = json!({
            "description": "parent",
            "properties": { "a": { "type": "string" } },
            "required": ["a"],
            "allOf": [
                { "$ref": "#/$defs/X", "description": "member", "properties": { "a": { "type": "integer" }, "b": { "type": "integer" } }, "required": ["b"] },
                { "$ref": "#/$defs/Y", "required": ["a", "c"] }
            ]
        });
        assert_eq!(
            sanitize_schema(&s),
            json!({
                "description": "parent",
                "$ref": "#/$defs/X",
                "properties": { "a": { "type": "string" }, "b": { "type": "integer" } },
                "required": ["a", "b", "c"],
            })
        );
    }

    #[test]
    fn const_becomes_a_one_value_enum() {
        assert_eq!(
            sanitize_schema(&json!({ "type": "string", "const": "zq_kind_7" })),
            json!({ "type": "string", "enum": ["zq_kind_7"] })
        );
    }

    #[test]
    fn properties_named_like_keywords_survive() {
        let s = json!({
            "type": "object",
            "properties": {
                "const": { "type": "string" },
                "allOf": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["const", "allOf"]
        });
        assert_eq!(sanitize_schema(&s), s);
    }

    #[test]
    fn everything_else_passes_through() {
        let s = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "T",
            "type": "object",
            "properties": {
                "kind": { "oneOf": [{ "type": "string" }, { "type": "integer" }] },
                "maybe": { "anyOf": [{ "$ref": "#/definitions/D" }, { "type": "null" }] },
                "n": { "type": ["integer", "null"], "format": "uint64", "minimum": 0, "default": 3 },
                "any": true,
                "url": { "type": "string", "format": "uri", "pattern": "^https://" },
                "rows": { "type": "array", "maxItems": 100, "items": { "type": "array", "items": { "type": "string" } } },
                "lit": { "default": { "const": 1, "allOf": [] }, "examples": [{ "const": 2 }] }
            },
            "definitions": { "D": { "type": "string" } },
            "additionalProperties": false
        });
        assert_eq!(sanitize_schema(&s), s);
    }

    #[test]
    fn harness_send_schema_has_no_all_of_or_const_left() {
        let golden: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../harness/tests/golden/schemas/harness.send.json"
        )))
        .unwrap();
        let request = &golden["request_schema"];
        assert!(
            request.to_string().contains("allOf"),
            "fixture lost its allOf"
        );
        let out = sanitize_schema(request);

        fn keyword_free(v: &Value) -> bool {
            match v {
                Value::Object(m) => m.iter().all(|(k, v)| {
                    if NAMED_SCHEMAS.contains(&k.as_str()) {
                        v.as_object()
                            .is_none_or(|named| named.values().all(keyword_free))
                    } else if LITERALS.contains(&k.as_str()) {
                        k != "const"
                    } else {
                        k != "allOf" && keyword_free(v)
                    }
                }),
                Value::Array(a) => a.iter().all(keyword_free),
                _ => true,
            }
        }
        assert!(keyword_free(&out), "{out}");
        assert_eq!(
            out.pointer("/definitions/FunctionPolicy/properties/expose/$ref"),
            Some(&json!("#/definitions/ExposeMode"))
        );
    }

    #[test]
    fn response_format_with_and_without_schema() {
        let rf = ResponseFormat {
            r#type: "json".into(),
            schema: Some(json!({ "type": "object", "properties": { "k": { "const": "v" } } })),
        };
        assert_eq!(
            response_format_to_wire(&rf),
            json!({
                "type": "text",
                "mime_type": "application/json",
                "schema": { "type": "object", "properties": { "k": { "enum": ["v"] } } },
            })
        );
        let rf = ResponseFormat {
            r#type: "json".into(),
            schema: None,
        };
        assert_eq!(
            response_format_to_wire(&rf),
            json!({ "type": "text", "mime_type": "application/json" })
        );
    }
}
