//! Shared plumbing for the `judge-<provider>` workers: bounded HTTP with
//! retries and redacted diagnostics, caller-scoped cancellation, and `secret://`
//! API keys. The hosted providers use all of it; the local llama.cpp ones use
//! only `cancellation`. Provider wire formats stay in each worker.
pub mod cancellation;
pub mod secrets;
pub mod transport;
pub use transport::{ExecutionLimits, Failure, RetryPolicy, DEFAULT_RETRY};
