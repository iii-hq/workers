//! sentinel — the error monitor for an iii engine.
//!
//! It subscribes to the engine's `trace` and `log` triggers, groups failures
//! by a deterministic fingerprint, freezes the evidence before the in-memory
//! span ring discards it, and opens assisted harness investigations beside
//! the console page. Resolving and ignoring are human decisions; regression
//! is detected automatically.

/// The name this worker registers under. Its own traces are recognised by it,
/// so it is a constant rather than a string spelled out per call site.
pub const WORKER_NAME: &str = "sentinel";

pub mod adapters;
mod config;
pub mod configuration;
mod contract;
pub mod dependencies;
mod error;
pub mod events;
pub mod evidence;
pub mod fingerprint;
pub mod functions;
pub mod ids;
pub mod iii_runtime;
pub mod ingest;
pub mod investigation;
pub mod lifecycle;
pub mod manifest;
pub mod normalize;
pub mod offload;
pub mod redact;
pub mod registry;
pub mod retention;
pub mod service;
mod status;
pub mod store;
pub mod triggers;
pub mod ui;

pub use adapters::ErrorEvent;
pub use config::{
    ArchiveConfigV1, EvidenceConfigV1, FingerprintConfigV1, IngestConfigV1, InvestigationConfigV1,
    LogSourceConfigV1, RedactionConfigV1, RepositoryConfigV1, RetentionConfigV1, SourcesConfigV1,
    TraceSourceConfigV1, WorkerConfig,
};
pub use configuration::{ConfigCell, ConfigErrorCell};
pub use contract::{
    ConfidenceV1, DiagnosesListRequestV1, DiagnosesListResponseV1, DiagnosisCategoryV1,
    DiagnosisEvidenceV1, DiagnosisRecordRequestV1, DiagnosisRecordResponseV1, DiagnosisRecordV1,
    DiagnosisSourceV1, DiagnosisV1, DoorbellResponseV1, EngineStatusV1, ErrorSourceV1,
    EvidenceGetRequestV1, EvidenceGetResponseV1, EvidenceKindV1, GroupActionRequestV1,
    GroupChangeReasonV1, GroupChangedConfigV1, GroupChangedEventV1, GroupChangedOpV1,
    GroupCountsV1, GroupGetRequestV1, GroupGetResponseV1, GroupHistoryRequestV1,
    GroupHistoryResponseV1, GroupStateResponseV1, GroupStatusV1, GroupSummaryV1, GroupTransitionV1,
    GroupsListRequestV1, GroupsListResponseV1, IgnoreBaselineV1, IgnoreRequestV1, IgnoreRuleV1,
    IngestStatusV1, InvestigateRequestV1, InvestigateResponseV1, InvestigationCancelRequestV1,
    InvestigationChangedEventV1, InvestigationChangedOpV1, InvestigationCountsV1,
    InvestigationGetRequestV1, InvestigationGetResponseV1, InvestigationModeV1,
    InvestigationStatusV1, InvestigationSummaryV1, InvestigationsListRequestV1,
    InvestigationsListResponseV1, LogsListRequestV1, LogsListResponseV1, OccurrenceSummaryV1,
    OccurrencesListRequestV1, OccurrencesListResponseV1, ProposedFixV1, RepositoryStatusV1,
    ResolveRequestV1, RiskV1, RootCauseV1, SourcesStatusV1, StatusRequestV1, StatusResponseV1,
    TraceGetRequestV1, TraceGetResponseV1, TraceStoreStateV1, TurnCompletedEventV1,
};
pub use error::SentinelError;
pub use events::Emitter;
pub use evidence::EvidenceBundleV1;
pub use ingest::{Ingest, IngestJob, IngestReport, Telemetry, TraceSummary};
pub use investigation::Investigations;
pub use lifecycle::{GroupState, Transition};
pub use normalize::Normalizer;
pub use redact::Redactor;
pub use registry::{Owner, Registry};
pub use service::Service;
pub use status::Counters;
pub use store::{
    Actor, Db, GroupRow, NamedRow, OccurrenceWrite, RecordOutcome, Statement, StepResult, Store,
};
