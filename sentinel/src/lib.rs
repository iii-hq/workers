//! sentinel — the error monitor for an iii engine.
//!
//! It subscribes to the engine's `trace` and `log` triggers, groups failures
//! by a deterministic fingerprint, freezes the evidence before the in-memory
//! span ring discards it, and opens assisted harness investigations beside
//! the console page. Resolving and ignoring are human decisions; regression
//! is detected automatically.
//!
//! Design of record:
//! <https://github.com/iii-hq/workers/blob/main/tech-specs/2026-09-sentinel/sentinel.md>.

/// The name this worker registers under. Its own traces are recognised by it,
/// so it is a constant rather than a string spelled out per call site.
pub const WORKER_NAME: &str = "sentinel";

pub mod adapters;
mod config;
pub mod configuration;
mod contract;
pub mod dependencies;
mod error;
pub mod evidence;
pub mod fingerprint;
pub mod functions;
pub mod ids;
pub mod ingest;
pub mod lifecycle;
pub mod manifest;
pub mod normalize;
pub mod redact;
pub mod registry;
mod status;
pub mod store;

pub use adapters::ErrorEvent;
pub use config::{
    ArchiveConfigV1, EvidenceConfigV1, FingerprintConfigV1, IngestConfigV1, InvestigationConfigV1,
    LogSourceConfigV1, RedactionConfigV1, RepositoryConfigV1, RetentionConfigV1, SourcesConfigV1,
    TraceSourceConfigV1, WorkerConfig,
};
pub use configuration::{ConfigCell, ConfigErrorCell};
pub use contract::{
    EngineStatusV1, ErrorSourceV1, GroupChangeReasonV1, GroupCountsV1, GroupStatusV1,
    IgnoreBaselineV1, IgnoreRuleV1, IngestStatusV1, InvestigationCountsV1, RepositoryStatusV1,
    SourcesStatusV1, StatusRequestV1, StatusResponseV1, TraceStoreStateV1,
};
pub use error::SentinelError;
pub use evidence::EvidenceBundleV1;
pub use ingest::{Ingest, IngestJob, IngestReport, Telemetry, TraceSummary};
pub use lifecycle::{GroupState, Transition};
pub use normalize::Normalizer;
pub use redact::Redactor;
pub use registry::{Owner, Registry};
pub use status::Counters;
pub use store::{
    Db, GroupRow, NamedRow, OccurrenceWrite, RecordOutcome, Statement, StepResult, Store,
};
