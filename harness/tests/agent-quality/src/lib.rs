pub mod comparison;
pub mod completion;
pub mod context;
pub mod error;
pub mod limits;
pub mod report;
pub mod scenarios;
pub mod subject;
pub mod suite;

pub use comparison::{
    compare_runs, comparison_dimension, ComparisonDimension, PromptComparisonReportV1,
};
pub use context::ScenarioContext;
pub use error::{EvalError, FailureClass, FailureRecord, Phase};
pub use limits::{AgentQualityLimitsV1, EvaluationLimitsV1, ExecutionLimitsV1, LimitsArgs};
pub use report::{AgentQualityRunReportV1, AgentQualityScenarioReportV1, ScenarioObservationV1};
pub use subject::{ResolvedAgentQualitySubjectV1, SubjectArtifactV1};
pub use suite::{run_suite, SuiteRunConfig, SuiteRunOutcome};
