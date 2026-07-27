use iii_sdk::errors::Error as SdkError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    SetupError,
    SubjectError,
    AssertionFailure,
    EvidenceError,
    Timeout,
    CleanupError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Setup,
    Send,
    Await,
    Collect,
    Assert,
    Cleanup,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureRecord {
    pub class: FailureClass,
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct EvalError {
    pub record: FailureRecord,
    message: String,
}

impl EvalError {
    pub fn new(
        class: FailureClass,
        phase: Phase,
        code: Option<String>,
        function_id: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        Self {
            record: FailureRecord {
                class,
                phase,
                code,
                function_id,
                message: message.clone(),
            },
            message,
        }
    }

    pub fn setup(message: impl Into<String>) -> Self {
        Self::new(FailureClass::SetupError, Phase::Setup, None, None, message)
    }

    pub fn timeout(phase: Phase, message: impl Into<String>) -> Self {
        Self::new(FailureClass::Timeout, phase, None, None, message)
    }

    pub fn assertion(message: impl Into<String>) -> Self {
        Self::new(
            FailureClass::AssertionFailure,
            Phase::Assert,
            None,
            None,
            message,
        )
    }

    pub fn limit(message: impl Into<String>) -> Self {
        Self::new(
            FailureClass::AssertionFailure,
            Phase::Assert,
            Some("agent_quality/limit_exceeded".to_string()),
            Some("harness::metrics".to_string()),
            message,
        )
    }

    pub fn evidence(function_id: &str, message: impl Into<String>) -> Self {
        Self::new(
            FailureClass::EvidenceError,
            Phase::Collect,
            None,
            Some(function_id.to_string()),
            message,
        )
    }

    pub fn from_sdk(phase: Phase, function_id: &str, error: SdkError) -> Self {
        let code = error.invocation_error().map(|value| value.code);
        let class = if matches!(error, SdkError::Timeout) {
            FailureClass::Timeout
        } else {
            class_for_phase(phase)
        };
        Self::new(
            class,
            phase,
            code,
            Some(function_id.to_string()),
            error.to_string(),
        )
    }

    pub fn serialization(phase: Phase, function_id: &str, message: impl Into<String>) -> Self {
        Self::new(
            class_for_phase(phase),
            phase,
            Some("SERDE".to_string()),
            Some(function_id.to_string()),
            message,
        )
    }
}

pub fn class_for_phase(phase: Phase) -> FailureClass {
    match phase {
        Phase::Setup => FailureClass::SetupError,
        Phase::Send => FailureClass::SubjectError,
        Phase::Await | Phase::Collect => FailureClass::EvidenceError,
        Phase::Assert => FailureClass::AssertionFailure,
        Phase::Cleanup => FailureClass::CleanupError,
    }
}
