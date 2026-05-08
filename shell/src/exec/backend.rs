//! Backend trait for exec — host and sandbox impls live in sibling
//! modules. The trait takes `argv` and a resolved `timeout_ms`; the
//! handler is responsible for argv parsing, allowlist checking, and
//! timeout resolution before calling `run`.

use async_trait::async_trait;

use super::error::ExecError;

pub type ExecCallResult<T> = Result<T, ExecError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[async_trait]
pub trait ExecBackend: Send + Sync {
    async fn run(&self, argv: &[String], timeout_ms: u64) -> ExecCallResult<ExecOutcome>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: trait is object-safe.
    #[test]
    fn backend_trait_is_object_safe() {
        fn _f(_: &dyn ExecBackend) {}
    }

    #[test]
    fn outcome_round_trips_via_clone() {
        let o = ExecOutcome {
            stdout: "hi".into(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 5,
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
        };
        assert_eq!(o.clone(), o);
    }
}
