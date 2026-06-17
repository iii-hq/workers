//! Output-contract strategy (harness.md § Output contract). A turn declares
//! what it must produce — free text or JSON (optionally schema-validated). The
//! harness picks a strategy per turn: provider-native structured output when
//! the model supports it, else the synthetic `submit_result` fallback.

use jsonschema::JSONSchema;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::policy;
use crate::types::model::AgentFunction;
use crate::types::output::OutputContract;
use crate::types::turn::TurnRecord;

/// The resolved delivery strategy for a turn's output contract.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputStrategy {
    /// Free text — the result is the final assistant message's text.
    Text,
    /// Provider-native structured output (`response_format`).
    ProviderNativeJson { schema: Option<Value> },
    /// The `submit_result` fallback (a synthetic invocation schema).
    SubmitResultJson { schema: Option<Value> },
}

impl OutputStrategy {
    /// Pick the strategy: provider-native when `router::models::supports(model,
    /// "structured_output")`, else the `submit_result` fallback.
    pub async fn resolve(deps: &Deps, record: &TurnRecord) -> OutputStrategy {
        match &record.options.output {
            OutputContract::Text => OutputStrategy::Text,
            OutputContract::Json { schema } => {
                let provider = record.options.provider.clone().unwrap_or_default();
                let supports = if provider.is_empty() {
                    false
                } else {
                    deps.router()
                        .await
                        .models_supports(&provider, &record.options.model, "structured_output")
                        .await
                };
                if supports {
                    OutputStrategy::ProviderNativeJson {
                        schema: schema.clone(),
                    }
                } else {
                    OutputStrategy::SubmitResultJson {
                        schema: schema.clone(),
                    }
                }
            }
        }
    }

    /// The `response_format` to send on `router::chat` (provider-native only).
    pub fn response_format(&self) -> Option<Value> {
        match self {
            OutputStrategy::ProviderNativeJson { schema } => {
                let mut v = json!({ "type": "json" });
                if let Some(s) = schema {
                    v["schema"] = s.clone();
                }
                Some(v)
            }
            _ => None,
        }
    }

    /// The synthetic `submit_result` tool to attach (fallback only).
    pub fn submit_result_tool(&self) -> Option<AgentFunction> {
        match self {
            OutputStrategy::SubmitResultJson { schema } => {
                Some(policy::submit_result_schema(schema.as_ref()))
            }
            _ => None,
        }
    }

    pub fn schema(&self) -> Option<&Value> {
        match self {
            OutputStrategy::ProviderNativeJson { schema }
            | OutputStrategy::SubmitResultJson { schema } => schema.as_ref(),
            OutputStrategy::Text => None,
        }
    }

    pub fn is_json(&self) -> bool {
        !matches!(self, OutputStrategy::Text)
    }
}

/// Parse final assistant text as a JSON value (provider-native result path).
pub fn parse_json_text(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    serde_json::from_str::<Value>(trimmed).map_err(|e| format!("result is not valid JSON: {e}"))
}

/// Validate a JSON value against an optional schema. `Ok(())` when no schema
/// is supplied. Errors join the first few schema violations for a nudge.
pub fn validate_json(value: &Value, schema: Option<&Value>) -> Result<(), String> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let compiled =
        JSONSchema::compile(schema).map_err(|e| format!("invalid output schema: {e}"))?;
    if let Err(errors) = compiled.validate(value) {
        let msgs: Vec<String> = errors.take(10).map(|e| e.to_string()).collect();
        return Err(format!(
            "result failed schema validation: {}",
            msgs.join("; ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_text_trims_and_parses() {
        assert_eq!(
            parse_json_text("  { \"a\": 1 } ").unwrap(),
            json!({ "a": 1 })
        );
        assert!(parse_json_text("not json").is_err());
    }

    #[test]
    fn validate_passes_without_schema() {
        assert!(validate_json(&json!({ "x": 1 }), None).is_ok());
    }

    #[test]
    fn validate_enforces_schema() {
        let schema = json!({
            "type": "object",
            "properties": { "category": { "type": "string" } },
            "required": ["category"]
        });
        assert!(validate_json(&json!({ "category": "billing" }), Some(&schema)).is_ok());
        let err = validate_json(&json!({ "category": 42 }), Some(&schema)).unwrap_err();
        assert!(err.contains("schema validation"), "got: {err}");
        assert!(validate_json(&json!({}), Some(&schema)).is_err());
    }

    #[test]
    fn response_format_only_for_provider_native() {
        let native = OutputStrategy::ProviderNativeJson {
            schema: Some(json!({ "type": "object" })),
        };
        assert_eq!(
            native.response_format(),
            Some(json!({ "type": "json", "schema": { "type": "object" } }))
        );
        assert!(OutputStrategy::Text.response_format().is_none());
        assert!(OutputStrategy::SubmitResultJson { schema: None }
            .response_format()
            .is_none());
    }

    #[test]
    fn submit_result_tool_only_for_fallback() {
        let fallback = OutputStrategy::SubmitResultJson { schema: None };
        assert!(fallback.submit_result_tool().is_some());
        assert!(OutputStrategy::Text.submit_result_tool().is_none());
        assert!(OutputStrategy::ProviderNativeJson { schema: None }
            .submit_result_tool()
            .is_none());
    }
}
