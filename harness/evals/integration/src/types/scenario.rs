//! `IntegrationScenarioV1` / `IntegrationResultV1` (spec § Proposed scenario
//! and result schemas) plus the failure-classification lattice.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::recorder::RecorderConfigV1;
use super::script::SchemaVersion1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationScenarioV1 {
    pub schema_version: SchemaVersion1,
    pub id: String,
    pub description: String,
    pub stack_profile: String,
    /// The exact `harness::send` request (after placeholder expansion). Kept
    /// as raw JSON: the harness owns this schema and the send response is
    /// graded against the request as-sent.
    pub send: serde_json::Value,
    /// Path to the router script, relative to the scenario directory.
    pub router_script: String,
    pub recorder: RecorderConfigV1,
    pub deadlines: DeadlinesV1,
    pub invariants: Vec<InvariantSpecV1>,
    /// Explicit fault seed (spec § Goals: "reproduce process crashes and
    /// restart boundaries with explicit fault seeds"). Absent = no fault.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<FaultV1>,
    /// Trigger bindings the runner creates after the harness boots (the
    /// harness owns these trigger types), e.g. `harness::hook::pre-trigger`
    /// hook-chain bindings. The bound function ids are recorder-controlled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<TriggerBindingV1>,
    /// Release step for hook-held calls: once `function_call_id` appears in
    /// `harness::status` pending calls, call `harness::function::resolve`
    /// with this action (issue-506 family scenarios).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseV1>,
    /// Quarantined scenarios reproduce a known-open defect: they assert the
    /// EXPECTED behavior, fail until the defect is fixed, and are excluded
    /// from `--scenario all` (runnable only by explicit id).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quarantine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultV1 {
    pub kind: FaultKind,
    /// Inject once the recorder has durably observed this many target calls
    /// (the dispatched call is executing inside its response delay).
    pub after_target_calls: u64,
    /// Downtime before the engine is respawned.
    pub restart_delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// SIGKILL the engine process, then respawn it from the same config and
    /// data directories after `restart_delay_ms`.
    EngineSigkill,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriggerBindingV1 {
    /// e.g. `harness::hook::pre-trigger`.
    pub trigger_type: String,
    /// Recorder-controlled function id (run-scoped).
    pub function_id: String,
    /// Binding config passed verbatim (e.g. `{functions, priority}`).
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseV1 {
    /// The scripted call id to wait for and release (e.g. `call-1`).
    pub function_call_id: String,
    /// `execute` or `deliver` (passed verbatim to `function::resolve`).
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeadlinesV1 {
    pub readiness_ms: u64,
    pub scenario_ms: u64,
    pub teardown_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvariantSpecV1 {
    pub id: String,
    #[serde(default)]
    pub parameters: serde_json::Map<String, serde_json::Value>,
}

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
    /// Failure precedence (spec § Proposed scenario and result schemas):
    /// runner_error > process_crash > setup_error > timeout > contract_failure.
    /// Higher wins when combining classifications for one scenario.
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

    /// Process exit code contribution: 0 pass, 2 contract_failure/timeout,
    /// 3 runner_error/setup_error/process_crash. The run's exit code is the
    /// max over selected scenarios.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvariantResultV1 {
    pub id: String,
    pub passed: bool,
    pub expected: serde_json::Value,
    pub actual: serde_json::Value,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationResultV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub scenario_id: String,
    pub classification: Classification,
    pub invariants: Vec<InvariantResultV1>,
    pub artifacts: Vec<String>,
    pub started_at: String,
    pub duration_ms: u64,
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
    fn exit_codes_match_spec_table() {
        use Classification::*;
        assert_eq!(Pass.exit_code(), 0);
        assert_eq!(ContractFailure.exit_code(), 2);
        assert_eq!(Timeout.exit_code(), 2);
        assert_eq!(RunnerError.exit_code(), 3);
        assert_eq!(SetupError.exit_code(), 3);
        assert_eq!(ProcessCrash.exit_code(), 3);
    }
}
