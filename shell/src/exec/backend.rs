//! Backend trait for exec — host and sandbox impls live in sibling
//! modules. The trait takes `argv`, a resolved `timeout_ms`, and the
//! validated per-call `overrides` (cwd/env); the handler is responsible
//! for argv parsing, allowlist checking, timeout resolution, and the
//! cwd/env gating before calling `run`.

use async_trait::async_trait;

use super::error::ExecError;
use super::policy::ExecOverrides;

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
    /// Run `argv` to completion. `overrides` carries the validated per-call
    /// cwd/env (host-only — the sandbox backend rejects a populated override
    /// with S210 since the in-VM exec protocol does not yet forward them).
    async fn run(
        &self,
        argv: &[String],
        timeout_ms: u64,
        overrides: &ExecOverrides,
    ) -> ExecCallResult<ExecOutcome>;
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
