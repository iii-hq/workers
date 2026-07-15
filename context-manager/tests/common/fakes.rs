//! Deterministic fakes behind the ports for @pure scenarios.
//!
//! Each fake records what the production code asked of it (summariser
//! prompts, resolver lookups) so scenarios can assert not just the
//! response but the *interaction* — e.g. that a second compaction was
//! anchored on the previous summary instead of starting over.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use context_manager::ports::{
    Clock, LeaseRecord, LeaseStore, ModelBudget, ModelResolver, SummarizeError, SummarizeRequest,
    Summarizer,
};
use context_manager::types::Model;

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Starts at a fixed epoch so lease timestamps are reproducible;
/// scenarios advance it explicitly to drive TTL expiry.
pub struct FakeClock {
    now_ms: AtomicI64,
}

pub const CLOCK_START_MS: i64 = 1_000_000;

impl FakeClock {
    pub fn new() -> Self {
        Self {
            now_ms: AtomicI64::new(CLOCK_START_MS),
        }
    }

    pub fn advance(&self, ms: i64) {
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Model resolver
// ---------------------------------------------------------------------------

/// Scripted router catalog. `unavailable` simulates `llm-router` not
/// being installed (every lookup errors); an id missing from `models`
/// simulates a router that does not know the model.
pub struct FakeModelResolver {
    pub models: Mutex<HashMap<String, Model>>,
    pub unavailable: AtomicBool,
    /// Recorded `(provider, id)` lookups.
    pub calls: Mutex<Vec<(Option<String>, String)>>,
}

impl FakeModelResolver {
    pub fn new() -> Self {
        Self {
            models: Mutex::new(HashMap::new()),
            unavailable: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn insert(&self, model: Model) {
        self.models
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(model.id.clone(), model);
    }
}

#[async_trait]
impl ModelResolver for FakeModelResolver {
    async fn get_model_budget(
        &self,
        provider: Option<&str>,
        id: &str,
    ) -> Result<Option<ModelBudget>, String> {
        self.calls
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push((provider.map(str::to_string), id.to_string()));
        if self.unavailable.load(Ordering::SeqCst) {
            return Err("router::models::get is not routable".to_string());
        }
        Ok(self
            .models
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(id)
            .cloned()
            .map(|model| ModelBudget {
                effective_max_output_tokens: model.max_output_tokens,
                model,
            }))
    }
}

// ---------------------------------------------------------------------------
// Summariser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SummarizerScript {
    Return(String),
    Unavailable,
    Fail(String),
    Empty,
}

pub struct FakeSummarizer {
    pub script: Mutex<SummarizerScript>,
    /// Every request the production pipeline sent, in order.
    pub calls: Mutex<Vec<SummarizeRequest>>,
}

impl FakeSummarizer {
    pub fn new() -> Self {
        Self {
            // A recognisable default so scenarios that don't care about
            // the text still see a plausible summary flow through.
            script: Mutex::new(SummarizerScript::Return(
                "## Goal\n- (scripted default summary)".to_string(),
            )),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn set_script(&self, script: SummarizerScript) {
        *self
            .script
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = script;
    }

    pub fn calls(&self) -> Vec<SummarizeRequest> {
        self.calls
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

#[async_trait]
impl Summarizer for FakeSummarizer {
    async fn summarize(&self, req: SummarizeRequest) -> Result<String, SummarizeError> {
        self.calls
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(req);
        let script = self
            .script
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        match script {
            SummarizerScript::Return(summary) => Ok(summary),
            SummarizerScript::Unavailable => Err(SummarizeError::Unavailable(
                "router::chat is not routable".to_string(),
            )),
            SummarizerScript::Fail(reason) => Err(SummarizeError::Failed(reason)),
            SummarizerScript::Empty => Err(SummarizeError::Empty),
        }
    }
}

// ---------------------------------------------------------------------------
// Lease store
// ---------------------------------------------------------------------------

/// In-memory `(scope, key) -> LeaseRecord` map. `swap` is genuinely
/// atomic under the mutex, mirroring `FsLeaseStore`'s under-lock swap.
/// `fail_all` simulates a lease-store outage (every call errors).
pub struct InMemoryLeaseStore {
    records: Mutex<HashMap<(String, String), LeaseRecord>>,
    pub fail_all: AtomicBool,
}

impl InMemoryLeaseStore {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            fail_all: AtomicBool::new(false),
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.fail_all.load(Ordering::SeqCst) {
            return Err("state is unavailable".to_string());
        }
        Ok(())
    }

    /// Direct write for Given steps (a foreign holder's claim).
    pub fn put(&self, scope: &str, key: &str, record: LeaseRecord) {
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert((scope.to_string(), key.to_string()), record);
    }

    /// Direct read for Then steps.
    pub fn peek(&self, scope: &str, key: &str) -> Option<LeaseRecord> {
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&(scope.to_string(), key.to_string()))
            .cloned()
    }

    /// All keys currently holding a claim (any scope).
    pub fn keys(&self) -> Vec<String> {
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .keys()
            .map(|(_, key)| key.clone())
            .collect()
    }
}

#[async_trait]
impl LeaseStore for InMemoryLeaseStore {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<LeaseRecord>, String> {
        self.check()?;
        Ok(self.peek(scope, key))
    }

    async fn set(&self, scope: &str, key: &str, value: Option<LeaseRecord>) -> Result<(), String> {
        self.check()?;
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match value {
            Some(record) => {
                records.insert((scope.to_string(), key.to_string()), record);
            }
            None => {
                records.remove(&(scope.to_string(), key.to_string()));
            }
        }
        Ok(())
    }

    async fn swap(
        &self,
        scope: &str,
        key: &str,
        value: LeaseRecord,
    ) -> Result<Option<LeaseRecord>, String> {
        self.check()?;
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Ok(records.insert((scope.to_string(), key.to_string()), value))
    }
}
