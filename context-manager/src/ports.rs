//! Ports: the only seams between the pure context logic and the
//! outside world (llm-router, iii state, wall clock).
//!
//! The binary wires production adapters (`crate::adapters`); BDD wires
//! deterministic fakes. Handlers receive everything through [`Deps`],
//! so the full `context::*` surface is testable without an engine.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::types::Model;

#[derive(Debug, Clone, PartialEq)]
pub struct ModelBudget {
    pub model: Model,
    pub effective_max_output_tokens: u64,
}

/// Resolves a model id to its router catalog record.
#[async_trait]
pub trait ModelResolver: Send + Sync {
    /// `Ok(None)` when the router is installed but does not know the
    /// model; `Err` when the router is unreachable or absent. Both
    /// degrade to the conservative fallback (when allowed).
    async fn get_model_budget(
        &self,
        provider: Option<&str>,
        id: &str,
    ) -> Result<Option<ModelBudget>, String>;
}

/// One summariser call. The core renders the prompts (template,
/// previous-summary anchoring, media stripping) so this port only
/// transports them.
#[async_trait]
pub trait Summarizer: Send + Sync {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummarizeRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    /// Model id the summary runs on (same model as the request).
    pub model: String,
    pub provider: Option<String>,
}

/// Why a summariser call produced no summary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SummarizeError {
    /// `llm-router` is not installed / `router::chat` is not routable.
    /// Compaction is unavailable; `context::compact` maps this to
    /// `status: "overflow"` with a permanent error_kind in the log.
    #[error("summariser unavailable: {0}")]
    Unavailable(String),

    /// The summariser ran and failed (provider error, stream error,
    /// or the summariser itself overflowed).
    #[error("summariser failed: {0}")]
    Failed(String),

    /// The summariser returned an empty summary.
    #[error("summariser returned an empty summary")]
    Empty,
}

/// A `{nonce, ts}` lease claim (harness `runtime/lease.ts` semantics:
/// TTL is enforced by readers — a claim older than the TTL reads as
/// free, folding crash recovery into acquisition).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub nonce: String,
    /// Claim time, milliseconds since epoch.
    pub ts: i64,
}

/// Key-value storage backing compaction leases (scope `context_lease`).
#[async_trait]
pub trait LeaseStore: Send + Sync {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<LeaseRecord>, String>;

    /// `Some` writes the record; `None` clears the key.
    async fn set(&self, scope: &str, key: &str, value: Option<LeaseRecord>) -> Result<(), String>;

    /// Atomic read-modify-write: store `value` and return the previous
    /// record in one operation, so exactly one concurrent acquirer sees
    /// a free/expired prior and wins.
    async fn swap(
        &self,
        scope: &str,
        key: &str,
        value: LeaseRecord,
    ) -> Result<Option<LeaseRecord>, String>;
}

/// Hot-swappable lease store. A `lease_dir` config change rebuilds the
/// `FsLeaseStore` and swaps it here on the fly (see
/// [`crate::configuration`]), so the field hot-reloads — no restart.
pub type LeaseCell = Arc<RwLock<Arc<dyn LeaseStore>>>;

/// Wrap a lease store in a [`LeaseCell`]. Used at boot and on a
/// `lease_dir` change (the store is rebuilt and swapped). Callers pass an
/// `Arc::new(FsLeaseStore::new(..)?)` (or any `Arc<dyn LeaseStore>`).
pub fn lease_cell(store: Arc<dyn LeaseStore>) -> LeaseCell {
    Arc::new(RwLock::new(store))
}

/// Wall clock, faked in tests to drive lease TTL expiry.
pub trait Clock: Send + Sync {
    /// Milliseconds since epoch.
    fn now_ms(&self) -> i64;
}

/// Production clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Everything a `context::*` handler needs.
pub struct Deps {
    /// Hot-swappable config snapshot, sourced from the `configuration`
    /// worker and reloaded live (see [`crate::configuration`]).
    pub config: ConfigCell,
    pub resolver: Arc<dyn ModelResolver>,
    pub summarizer: Arc<dyn Summarizer>,
    /// Hot-swappable lease store: rebuilt + swapped on a `lease_dir` change.
    pub leases: LeaseCell,
    pub clock: Arc<dyn Clock>,
}

impl Deps {
    /// Take a cheap snapshot of the current config. Handlers call this
    /// once at the top of a request and read fields off the returned
    /// `Arc`; a concurrent hot-reload swaps the cell without disturbing
    /// snapshots already taken.
    pub async fn config(&self) -> Arc<WorkerConfig> {
        self.config.read().await.clone()
    }

    /// Take a cheap snapshot of the current lease store. A concurrent
    /// `lease_dir` reload swaps the cell without disturbing a snapshot
    /// already taken.
    pub async fn leases(&self) -> Arc<dyn LeaseStore> {
        self.leases.read().await.clone()
    }
}
