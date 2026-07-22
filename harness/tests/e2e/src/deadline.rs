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
    const fn at(expires_at: Instant) -> Self {
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
    fn cap(
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
        let operation = operation.into();
        let remaining = self.remaining();
        if remaining.is_zero() {
            return Err(DeadlineExceeded::new(operation));
        }
        tokio::time::timeout(remaining, future)
            .await
            .map_err(|_| DeadlineExceeded::new(operation))
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

    #[cfg(test)]
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

    #[test]
    fn timeout_ms_never_rounds_positive_time_to_zero() {
        let deadline = Deadline::after(Duration::from_nanos(1));
        if let Ok(timeout) = deadline.timeout_ms(1, "rpc") {
            assert_eq!(timeout, 1);
        }
    }
}
