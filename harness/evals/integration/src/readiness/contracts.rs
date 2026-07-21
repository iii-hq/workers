use serde_json::Value;

use crate::types::recorder::{
    LifecycleAcceptedV1, LifecycleEventV1, RecorderConfigV1, RecorderTargetV1,
};

/// Validation-relevant function contract expected from the live engine
/// registry. A missing optional field means that field is intentionally not
/// part of this readiness assertion.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedFunctionContract {
    pub function_id: String,
    pub description: Option<String>,
    pub request_schema: Option<Value>,
    pub response_schema: Option<Value>,
    pub schema_comparison: SchemaComparison,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedTriggerBinding {
    pub trigger_type: String,
    pub function_id: String,
    pub config: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaComparison {
    /// Producer-owned mirrors may differ only in JSON Schema annotations
    /// that do not affect validation.
    AnnotationInsensitive,
    /// Authored target schemas are model-visible and must survive registry
    /// registration byte-for-byte after canonical object-key ordering.
    Exact,
}

impl ExpectedFunctionContract {
    pub fn id_only(function_id: impl Into<String>) -> Self {
        Self {
            function_id: function_id.into(),
            description: None,
            request_schema: None,
            response_schema: None,
            schema_comparison: SchemaComparison::AnnotationInsensitive,
        }
    }

    pub fn from_golden(raw: &str) -> Self {
        let value: Value = serde_json::from_str(raw).expect("checked-in function golden is JSON");
        Self {
            function_id: required_string(&value, "function_id"),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .map(String::from),
            request_schema: value.get("request_schema").cloned(),
            response_schema: value.get("response_schema").cloned(),
            schema_comparison: SchemaComparison::AnnotationInsensitive,
        }
    }
}

pub(crate) fn router_contract(function_id: &str) -> ExpectedFunctionContract {
    router_contracts()
        .into_iter()
        .find(|contract| contract.function_id == function_id)
        .unwrap_or_else(|| panic!("no scripted-router contract for {function_id}"))
}

pub(crate) fn router_contracts() -> Vec<ExpectedFunctionContract> {
    [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.chat.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.abort.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.models.list.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.models.get.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.models.supports.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.system_prompt.get.json"
        )),
    ]
    .into_iter()
    .map(ExpectedFunctionContract::from_golden)
    .collect()
}

pub(crate) fn pre_harness_contracts() -> Vec<ExpectedFunctionContract> {
    let mut contracts = [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../session-manager/tests/golden/schemas/session.messages.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../context-manager/tests/golden/schemas/context.assemble.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../context-manager/tests/golden/schemas/context.count-tokens.json"
        )),
    ]
    .into_iter()
    .map(ExpectedFunctionContract::from_golden)
    .collect::<Vec<_>>();
    contracts.extend(router_contracts());
    contracts.push(lifecycle_contract());
    // Queue readiness is asserted semantically by calling this function and
    // validating its topic response. There is no worker-owned golden for its
    // engine builtin descriptor.
    contracts.push(ExpectedFunctionContract::id_only(
        "engine::queue::list_topics",
    ));
    contracts
}

pub(crate) fn harness_contracts() -> Vec<ExpectedFunctionContract> {
    [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../harness/tests/golden/schemas/harness.send.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../harness/tests/golden/schemas/harness.status.json"
        )),
    ]
    .into_iter()
    .map(ExpectedFunctionContract::from_golden)
    .collect()
}

pub(crate) fn controlled_contracts(config: &RecorderConfigV1) -> Vec<ExpectedFunctionContract> {
    std::iter::once(&config.target)
        .chain(config.extra_functions.iter())
        .map(controlled_contract)
        .collect()
}

fn controlled_contract(target: &RecorderTargetV1) -> ExpectedFunctionContract {
    ExpectedFunctionContract {
        function_id: target.function_id.clone(),
        description: Some(target.description.clone()),
        request_schema: Some(Value::Object(target.request_schema.clone())),
        response_schema: Some(crate::recorder::const_response_schema(&target.response)),
        schema_comparison: SchemaComparison::Exact,
    }
}

fn lifecycle_contract() -> ExpectedFunctionContract {
    ExpectedFunctionContract {
        function_id: "integration-recorder::lifecycle".to_string(),
        description: Some("Durable sink for harness lifecycle trigger deliveries.".to_string()),
        request_schema: Some(schema_for::<LifecycleEventV1>()),
        response_schema: Some(schema_for::<LifecycleAcceptedV1>()),
        schema_comparison: SchemaComparison::Exact,
    }
}

pub fn contract_failures(expected: &ExpectedFunctionContract, actual: &Value) -> Vec<String> {
    let descriptor = actual.get("function").unwrap_or(actual);
    let mut failures = Vec::new();

    let actual_id = descriptor
        .get("function_id")
        .or_else(|| descriptor.get("id"))
        .and_then(Value::as_str);
    if actual_id != Some(expected.function_id.as_str()) {
        failures.push(format!(
            "function {} id: expected {:?}, got {:?}",
            expected.function_id, expected.function_id, actual_id
        ));
    }

    if let Some(description) = &expected.description {
        let actual_description = descriptor.get("description").and_then(Value::as_str);
        if actual_description != Some(description.as_str()) {
            failures.push(format!(
                "function {} description mismatch: expected {:?}, got {:?}",
                expected.function_id, description, actual_description
            ));
        }
    }

    compare_schema(
        &mut failures,
        expected,
        descriptor,
        "request",
        "request_schema",
        "request_format",
        expected.request_schema.as_ref(),
    );
    compare_schema(
        &mut failures,
        expected,
        descriptor,
        "response",
        "response_schema",
        "response_format",
        expected.response_schema.as_ref(),
    );
    failures
}

fn compare_schema(
    failures: &mut Vec<String>,
    expected_contract: &ExpectedFunctionContract,
    descriptor: &Value,
    side: &str,
    primary_key: &str,
    compatibility_key: &str,
    expected: Option<&Value>,
) {
    let Some(expected) = expected else {
        return;
    };
    let actual = descriptor
        .get(primary_key)
        .or_else(|| descriptor.get(compatibility_key))
        .filter(|schema| !schema.is_null());
    let Some(actual) = actual else {
        failures.push(format!(
            "function {} {side} schema missing",
            expected_contract.function_id
        ));
        return;
    };
    let (expected, actual) = match expected_contract.schema_comparison {
        SchemaComparison::AnnotationInsensitive => {
            (validation_schema(expected), validation_schema(actual))
        }
        SchemaComparison::Exact => (expected.clone(), actual.clone()),
    };
    if actual != expected {
        failures.push(format!(
            "function {} {side} schema mismatch: expected_sha256={}, actual_sha256={}",
            expected_contract.function_id,
            crate::canonical::sha256_of_canonical(&expected),
            crate::canonical::sha256_of_canonical(&actual)
        ));
    }
}

/// Strip only JSON Schema annotation keywords. Validation-affecting
/// keywords, including `default` (part of the callable contract in this
/// repository), remain in the comparison.
pub fn validation_schema(schema: &Value) -> Value {
    const ANNOTATIONS: &[&str] = &[
        "$comment",
        "$schema",
        "deprecated",
        "description",
        "examples",
        "readOnly",
        "title",
        "writeOnly",
    ];
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !ANNOTATIONS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), validation_schema(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(validation_schema).collect()),
        value => value.clone(),
    }
}

fn schema_for<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("JSON schema serializes")
}

fn required_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("checked-in function golden is missing {key}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn annotation_only_schema_changes_are_ignored() {
        let left = json!({
            "$schema": "draft",
            "title": "Left",
            "type": "object",
            "properties": { "value": { "type": "string", "description": "left" } }
        });
        let right = json!({
            "title": "Right",
            "type": "object",
            "properties": { "value": { "type": "string", "description": "right" } }
        });
        assert_eq!(validation_schema(&left), validation_schema(&right));
    }

    #[test]
    fn validation_keywords_still_mismatch() {
        let expected = ExpectedFunctionContract {
            function_id: "test::function".into(),
            description: None,
            request_schema: Some(json!({ "type": "string" })),
            response_schema: None,
            schema_comparison: SchemaComparison::AnnotationInsensitive,
        };
        let failures = contract_failures(
            &expected,
            &json!({
                "function_id": "test::function",
                "request_schema": { "type": "integer" }
            }),
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("request schema mismatch"));
    }

    #[test]
    fn exact_target_contract_keeps_property_descriptions() {
        let expected = ExpectedFunctionContract {
            function_id: "run::target".into(),
            description: Some("target".into()),
            request_schema: Some(json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string", "description": "authoritative" }
                }
            })),
            response_schema: Some(json!({ "const": { "ok": true } })),
            schema_comparison: SchemaComparison::Exact,
        };
        let failures = contract_failures(
            &expected,
            &json!({
                "function_id": "run::target",
                "description": "target",
                "request_schema": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "string", "description": "changed" }
                    }
                },
                "response_schema": { "const": { "ok": true } }
            }),
        );
        assert!(failures
            .iter()
            .any(|failure| failure.contains("request schema mismatch")));
    }

    #[test]
    fn exact_target_contract_checks_description_and_response_schema() {
        let expected = ExpectedFunctionContract {
            function_id: "run::target".into(),
            description: Some("authoritative target".into()),
            request_schema: Some(json!({ "type": "object" })),
            response_schema: Some(json!({ "const": { "ok": true } })),
            schema_comparison: SchemaComparison::Exact,
        };
        let failures = contract_failures(
            &expected,
            &json!({
                "function_id": "run::target",
                "description": "changed target",
                "request_schema": { "type": "object" },
                "response_schema": { "const": { "ok": false } }
            }),
        );
        assert_eq!(failures.len(), 2);
        assert!(failures
            .iter()
            .any(|failure| failure.contains("description mismatch")));
        assert!(failures
            .iter()
            .any(|failure| failure.contains("response schema mismatch")));
    }
}
