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

mod config;
pub mod configuration;
mod contract;
pub mod dependencies;
mod error;
pub mod functions;
pub mod ids;
pub mod manifest;
mod status;

pub use config::{
    ArchiveConfigV1, EvidenceConfigV1, FingerprintConfigV1, IngestConfigV1, InvestigationConfigV1,
    LogSourceConfigV1, RedactionConfigV1, RepositoryConfigV1, RetentionConfigV1, SourcesConfigV1,
    TraceSourceConfigV1, WorkerConfig,
};
pub use configuration::{ConfigCell, ConfigErrorCell};
pub use contract::{
    EngineStatusV1, GroupCountsV1, IngestStatusV1, InvestigationCountsV1, RepositoryStatusV1,
    SourcesStatusV1, StatusRequestV1, StatusResponseV1, TraceStoreStateV1,
};
pub use error::SentinelError;
pub use status::Counters;
