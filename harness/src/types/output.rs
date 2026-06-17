//! Output contract — what a turn must produce (README § Output contract).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Free text by default; `json` constrains the final answer to a JSON value,
/// validated against `schema` when supplied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OutputContract {
    #[default]
    Text,
    Json {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
    },
}

impl OutputContract {
    pub fn is_json(&self) -> bool {
        matches!(self, OutputContract::Json { .. })
    }

    pub fn schema(&self) -> Option<&Value> {
        match self {
            OutputContract::Json { schema } => schema.as_ref(),
            OutputContract::Text => None,
        }
    }
}
