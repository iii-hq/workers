//! Shared runtime failure vocabulary. Every phase reports a typed error and
//! classification is derived in one place, so early returns cannot silently
//! invent a different precedence rule.

use crate::types::scenario::Classification;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Allocate,
    Boot,
    Probe,
    Arm,
    Send,
    Fault,
    Release,
    Await,
    Collect,
    Grade,
    Report,
    Teardown,
}

impl std::fmt::Display for RunPhase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            RunPhase::Allocate => "allocate",
            RunPhase::Boot => "boot",
            RunPhase::Probe => "probe",
            RunPhase::Arm => "arm",
            RunPhase::Send => "send",
            RunPhase::Fault => "fault",
            RunPhase::Release => "release",
            RunPhase::Await => "await",
            RunPhase::Collect => "collect",
            RunPhase::Grade => "grade",
            RunPhase::Report => "report",
            RunPhase::Teardown => "teardown",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunErrorKind {
    Setup,
    Contract,
    Timeout,
    ProcessCrash,
    Runner,
}

impl RunErrorKind {
    pub fn classification(self) -> Classification {
        match self {
            RunErrorKind::Setup => Classification::SetupError,
            RunErrorKind::Contract => Classification::ContractFailure,
            RunErrorKind::Timeout => Classification::Timeout,
            RunErrorKind::ProcessCrash => Classification::ProcessCrash,
            RunErrorKind::Runner => Classification::RunnerError,
        }
    }
}

impl std::fmt::Display for RunErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            RunErrorKind::Setup => "setup error",
            RunErrorKind::Contract => "contract failure",
            RunErrorKind::Timeout => "timeout",
            RunErrorKind::ProcessCrash => "process crash",
            RunErrorKind::Runner => "runner error",
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{phase}: {kind}: {message}")]
pub struct RunError {
    pub phase: RunPhase,
    pub kind: RunErrorKind,
    pub message: String,
    #[source]
    pub source: Option<anyhow::Error>,
}

impl RunError {
    pub fn new(phase: RunPhase, kind: RunErrorKind, message: impl Into<String>) -> Self {
        Self {
            phase,
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        phase: RunPhase,
        kind: RunErrorKind,
        message: impl Into<String>,
        source: impl Into<anyhow::Error>,
    ) -> Self {
        Self {
            phase,
            kind,
            message: message.into(),
            source: Some(source.into()),
        }
    }

    pub fn classification(&self) -> Classification {
        self.kind.classification()
    }

    /// Render this typed failure plus every nested source. `anyhow`'s
    /// alternate formatter only walks chains when formatting an
    /// `anyhow::Error` directly, so callers must not rely on `{self:#}`.
    pub fn chain_string(&self) -> String {
        let mut rendered = self.to_string();
        let mut source = self
            .source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn std::error::Error + 'static));
        while let Some(error) = source {
            rendered.push_str(": ");
            rendered.push_str(&error.to_string());
            source = error.source();
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_kind_has_one_classification() {
        assert_eq!(
            RunErrorKind::Setup.classification(),
            Classification::SetupError
        );
        assert_eq!(
            RunErrorKind::Contract.classification(),
            Classification::ContractFailure
        );
        assert_eq!(
            RunErrorKind::Timeout.classification(),
            Classification::Timeout
        );
        assert_eq!(
            RunErrorKind::ProcessCrash.classification(),
            Classification::ProcessCrash
        );
        assert_eq!(
            RunErrorKind::Runner.classification(),
            Classification::RunnerError
        );
    }

    #[test]
    fn chain_string_keeps_nested_causes() {
        let source = anyhow::anyhow!("permission denied").context("opening recorder log");
        let error = RunError::with_source(
            RunPhase::Boot,
            RunErrorKind::Setup,
            "start recorder",
            source,
        );
        assert_eq!(
            error.chain_string(),
            "boot: setup error: start recorder: opening recorder log: permission denied"
        );
    }
}
