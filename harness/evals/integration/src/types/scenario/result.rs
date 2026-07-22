use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::script::SchemaVersion1;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Pass,
    SetupError,
    ContractFailure,
    Timeout,
    ProcessCrash,
    RunnerError,
}

impl Classification {
    pub fn precedence(self) -> u8 {
        match self {
            Classification::Pass => 0,
            Classification::ContractFailure => 1,
            Classification::Timeout => 2,
            Classification::SetupError => 3,
            Classification::ProcessCrash => 4,
            Classification::RunnerError => 5,
        }
    }

    pub fn combine(self, other: Classification) -> Classification {
        if other.precedence() > self.precedence() {
            other
        } else {
            self
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Classification::Pass => 0,
            Classification::ContractFailure | Classification::Timeout => 2,
            Classification::SetupError
            | Classification::ProcessCrash
            | Classification::RunnerError => 3,
        }
    }
}

/// Stable, byte-comparable scenario result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationResultV1 {
    pub schema_version: SchemaVersion1,
    pub scenario_id: String,
    pub classification: Classification,
    /// First floor or verify failure, with run/session/turn ids scrubbed to
    /// `{{run_id}}`/`{{session_id}}`/`{{turn_id}}` placeholders so the
    /// persisted result stays comparable across runs.
    pub failure: Option<String>,
    pub artifacts: Vec<String>,
}

/// Volatile execution metadata kept separate from the stable result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReportV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub scenario_id: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub result_path: String,
    pub result_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_orders_and_combines() {
        use Classification::*;
        assert_eq!(ContractFailure.combine(RunnerError), RunnerError);
        assert_eq!(RunnerError.combine(ContractFailure), RunnerError);
        assert_eq!(ProcessCrash.combine(SetupError), ProcessCrash);
        assert_eq!(SetupError.combine(Timeout), SetupError);
        assert_eq!(Timeout.combine(ContractFailure), Timeout);
        assert_eq!(Pass.combine(ContractFailure), ContractFailure);
    }

    #[test]
    fn exit_codes_match_the_documented_contract() {
        use Classification::*;
        assert_eq!(Pass.exit_code(), 0);
        assert_eq!(ContractFailure.exit_code(), 2);
        assert_eq!(Timeout.exit_code(), 2);
        assert_eq!(RunnerError.exit_code(), 3);
        assert_eq!(SetupError.exit_code(), 3);
        assert_eq!(ProcessCrash.exit_code(), 3);
    }
}
