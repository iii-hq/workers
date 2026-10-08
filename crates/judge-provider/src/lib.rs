//! Shared plumbing for the hosted `judge-<provider>` workers: bounded HTTP with
//! retries and redacted diagnostics, caller-scoped cancellation, and `secret://`
//! API keys. Provider wire formats stay in each worker.
pub mod cancellation;
pub mod secrets;
pub mod transport;
pub use transport::{ExecutionLimits, Failure, RetryPolicy, DEFAULT_RETRY};
