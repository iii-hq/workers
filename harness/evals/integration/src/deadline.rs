//! One monotonic deadline shared by all phases of a scenario run.
//!
//! Keeping the deadline here avoids each caller creating its own timeout
//! window. In particular, an RPC can be capped by both its normal maximum
//! duration and the time that remains in the scenario.

use std::future::Future;
use std::time::Duration;

use thiserror::Error;
use tokio::time::Instant;

/// A monotonic, cloneable deadline.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    expires_at: Instant,
}

impl Deadline {
    /// Start a deadline that expires after `duration`.
    pub fn after(duration: Duration) -> Self {
        Self::at(Instant::now() + duration)
    }

    /// Use an existing Tokio instant as the deadline.
    pub const fn at(expires_at: Instant) -> Self {
        Self { expires_at }
    }

    /// The instant at which this deadline expires.
    pub const fn expires_at(self) -> Instant {
        self.expires_at
    }

    /// Remaining time, saturated at zero.
    pub fn remaining(self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }

    pub fn is_expired(self) -> bool {
        self.remaining().is_zero()
    }

    /// Cap an operation-specific timeout by the global deadline.
    pub fn cap(
        self,
        maximum: Duration,
        operation: impl Into<String>,
    ) -> Result<Duration, DeadlineExceeded> {
        let remaining = self.remaining();
        if remaining.is_zero() {
            return Err(DeadlineExceeded::new(operation));
        }
        Ok(remaining.min(maximum))
    }

    /// Millisecond-oriented form of [`Deadline::cap`] for client APIs whose
    /// timeout option is expressed as an integer.
    pub fn timeout_ms(
        self,
        maximum_ms: u64,
        operation: impl Into<String>,
    ) -> Result<u64, DeadlineExceeded> {
        let capped = self.cap(Duration::from_millis(maximum_ms), operation)?;
        // Do not turn a positive sub-millisecond remainder into an
        // accidental "no timeout" value in clients where zero is special.
        Ok(u64::try_from(capped.as_millis()).unwrap_or(u64::MAX).max(1))
    }

    /// Run a future until the global deadline.
    pub async fn timeout<F>(
        self,
        operation: impl Into<String>,
        future: F,
    ) -> Result<F::Output, DeadlineExceeded>
    where
        F: Future,
    {
        self.timeout_within(operation, self.remaining(), future)
            .await
    }

    /// Run a future until the earlier of an operation-specific maximum and
    /// the global deadline.
    pub async fn timeout_within<F>(
        self,
        operation: impl Into<String>,
        maximum: Duration,
        future: F,
    ) -> Result<F::Output, DeadlineExceeded>
    where
        F: Future,
    {
        let operation = operation.into();
        let duration = self.cap(maximum, operation.clone())?;
        tokio::time::timeout(duration, future)
            .await
            .map_err(|_| DeadlineExceeded::new(operation))
    }

    /// Run an anyhow-returning future and flatten timeout and operation
    /// failures into the same result type.
    pub async fn run<T, F>(self, operation: impl Into<String>, future: F) -> anyhow::Result<T>
    where
        F: Future<Output = anyhow::Result<T>>,
    {
        self.timeout(operation, future).await?
    }

    /// Poll until the probe returns a value or the deadline expires.
    ///
    /// A probe error is returned immediately. Both each probe invocation and
    /// the sleeps between probes are bounded by this deadline.
    pub async fn poll_until<T, F, Fut>(
        self,
        operation: impl Into<String>,
        interval: Duration,
        mut probe: F,
    ) -> anyhow::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = anyhow::Result<Option<T>>>,
    {
        let operation = operation.into();
        loop {
            if let Some(value) = self.timeout(operation.clone(), probe()).await?? {
                return Ok(value);
            }

            let wake_at = (Instant::now() + interval).min(self.expires_at);
            tokio::time::sleep_until(wake_at).await;
            if self.is_expired() {
                return Err(DeadlineExceeded::new(operation).into());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{operation} exceeded its deadline")]
pub struct DeadlineExceeded {
    operation: String,
}

impl DeadlineExceeded {
    fn new(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
        }
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_caps_a_hung_operation() {
        let deadline = Deadline::after(Duration::from_millis(30));
        let error = deadline
            .timeout("hung rpc", std::future::pending::<()>())
            .await
            .expect_err("deadline should fire");
        assert_eq!(error.operation(), "hung rpc");
        assert!(deadline.is_expired());
    }

    #[tokio::test]
    async fn operation_timeout_is_capped_by_global_remaining_time() {
        let deadline = Deadline::after(Duration::from_millis(30));
        let started = Instant::now();
        let error = deadline
            .timeout_within(
                "bounded rpc",
                Duration::from_secs(30),
                std::future::pending::<()>(),
            )
            .await
            .expect_err("global deadline should win");
        assert_eq!(error.operation(), "bounded rpc");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn poll_until_returns_the_first_ready_value() {
        let deadline = Deadline::after(Duration::from_secs(1));
        let mut attempts = 0;
        let value = deadline
            .poll_until("readiness", Duration::from_millis(1), || {
                attempts += 1;
                let current = attempts;
                async move {
                    if current == 3 {
                        Ok(Some("ready"))
                    } else {
                        Ok(None)
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(value, "ready");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn timeout_ms_never_rounds_positive_time_to_zero() {
        let deadline = Deadline::after(Duration::from_nanos(1));
        if let Ok(timeout) = deadline.timeout_ms(1, "rpc") {
            assert_eq!(timeout, 1);
        }
    }
}
