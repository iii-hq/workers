//! Claiming the durable dependencies.
//!
//! The worker registers its interface first and claims the store and the
//! queue after, in the background: the registry capture and the console must
//! see the function surface immediately, and CI boots this binary against an
//! isolated engine where neither `database` nor `queue` exists. Until both
//! answer, the worker is *up* but not *enabled* — ingest is refused, the
//! queue redelivers, and `sentinel::status` says so.
//!
//! Readiness is proven by **doing the work**, not by looking the dependency
//! up. A registry lookup answers a different question than a call does:
//! `engine::functions::info` resolves a bare id across namespaces, so it
//! happily reports `database::execute` as present when it is registered in
//! another namespace — while an actual call routes to this worker's own
//! namespace and fails. Migrating the schema and defining the queue are the
//! probe, and they are the same operations the worker needs anyway.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::iii_runtime::IngestQueue;
use crate::store::{Db, Store};
use crate::{Counters, SentinelError};

const RETRY: Duration = Duration::from_secs(1);

/// Defining the ingest queue, as a seam so boot can be tested without an
/// engine.
#[async_trait]
pub trait QueueSetup: Send + Sync {
    async fn ensure(&self) -> Result<(), SentinelError>;
}

#[async_trait]
impl QueueSetup for IngestQueue {
    async fn ensure(&self) -> Result<(), SentinelError> {
        IngestQueue::ensure(self).await
    }
}

/// Prepare the store and the queue, then open the gate. Retries forever: a
/// dependency that is not up yet is the normal case during a stack start, and
/// the alternative is a monitor that gave up quietly.
pub async fn claim<D: Db>(
    store: Arc<Store<D>>,
    queue: Arc<dyn QueueSetup>,
    counters: Arc<Counters>,
) {
    let mut announced = false;
    let version = loop {
        match store.migrate().await {
            Ok(version) => break version,
            Err(error) => {
                if !announced {
                    tracing::warn!(
                        %error,
                        "the sentinel store is not reachable yet; ingest stays closed"
                    );
                    announced = true;
                }
                tokio::time::sleep(RETRY).await;
            }
        }
    };

    let mut announced = false;
    loop {
        match queue.ensure().await {
            Ok(()) => break,
            Err(error) => {
                if !announced {
                    tracing::warn!(%error, "the ingest queue is not defined yet");
                    announced = true;
                }
                tokio::time::sleep(RETRY).await;
            }
        }
    }

    counters.mark_ready();
    tracing::info!(schema_version = version, "sentinel ingest is open");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FlakyQueue {
        failures_left: AtomicUsize,
    }

    #[async_trait]
    impl QueueSetup for FlakyQueue {
        async fn ensure(&self) -> Result<(), SentinelError> {
            if self.failures_left.load(Ordering::SeqCst) > 0 {
                self.failures_left.fetch_sub(1, Ordering::SeqCst);
                return Err(SentinelError::dependency("queue worker is not up"));
            }
            Ok(())
        }
    }

    /// A store whose first migration attempts fail, like a `database` worker
    /// that has not registered yet.
    struct FlakyDb {
        failures_left: AtomicUsize,
    }

    #[async_trait]
    impl Db for FlakyDb {
        async fn query(
            &self,
            _sql: &str,
            _params: Vec<serde_json::Value>,
        ) -> Result<Vec<crate::NamedRow>, SentinelError> {
            self.check()?;
            Ok(Vec::new())
        }

        async fn execute(
            &self,
            _sql: &str,
            _params: Vec<serde_json::Value>,
        ) -> Result<u64, SentinelError> {
            self.check()?;
            Ok(0)
        }

        async fn transaction(
            &self,
            _statements: &[crate::Statement],
        ) -> Result<Vec<crate::StepResult>, SentinelError> {
            self.check()?;
            Ok(Vec::new())
        }
    }

    impl FlakyDb {
        fn check(&self) -> Result<(), SentinelError> {
            if self.failures_left.load(Ordering::SeqCst) > 0 {
                self.failures_left.fetch_sub(1, Ordering::SeqCst);
                return Err(SentinelError::dependency(
                    "database::execute not found in this namespace",
                ));
            }
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_gate_opens_only_after_both_dependencies_answer() {
        let counters = Arc::new(Counters::default());
        let store = Arc::new(Store::new(FlakyDb {
            failures_left: AtomicUsize::new(2),
        }));
        let queue = Arc::new(FlakyQueue {
            failures_left: AtomicUsize::new(1),
        });

        assert!(!counters.is_ready());
        claim(store, queue, counters.clone()).await;
        assert!(
            counters.is_ready(),
            "readiness is proven by doing the work, not by looking it up"
        );
    }
}
