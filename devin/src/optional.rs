//! Optional bus dependencies. The `stream::set` builtin (the `iii-stream`
//! worker) and the `session::*` surface (the `session-manager` worker) are
//! observability, not requirements: a Devin run must still work on an engine
//! that has neither. Without a guard, every stdout line of every run fires a
//! `stream::set` that fails with `function_not_found` and logs a warning.
//!
//! An [`OptionalDependency`] remembers that a function is not registered, logs
//! that ONCE, and skips further calls. It re-probes once per
//! [`RETRY_AFTER_MS`] so a worker installed later is picked up without a
//! restart; recovery is logged once as well. Any other failure (timeout, a
//! handler error) is not "missing" and is left to the caller to report.

use std::sync::atomic::{AtomicU64, Ordering};

use iii_sdk::errors::Error;

/// How long a missing dependency is skipped before one call re-probes it.
pub const RETRY_AFTER_MS: u64 = 60_000;

/// `true` when the engine reports the function is not registered — the
/// providing worker is not installed (or not connected).
pub fn is_function_not_found(error: &Error) -> bool {
    matches!(error, Error::Remote { code, .. } if code == "function_not_found")
}

pub struct OptionalDependency {
    /// What the log lines name, e.g. `stream::set`.
    what: &'static str,
    /// Epoch ms at which the dependency was last found missing; 0 = present.
    missing_since_ms: AtomicU64,
}

impl OptionalDependency {
    pub const fn new(what: &'static str) -> Self {
        Self {
            what,
            missing_since_ms: AtomicU64::new(0),
        }
    }

    /// Whether a call should be attempted at `now_ms`: always while the
    /// dependency is believed present; while it is missing, only the one
    /// caller that claims the re-probe once the retry window has elapsed.
    pub fn should_try(&self, now_ms: u64) -> bool {
        let since = self.missing_since_ms.load(Ordering::Relaxed);
        if since == 0 {
            return true;
        }
        if now_ms.saturating_sub(since) < RETRY_AFTER_MS {
            return false;
        }
        self.missing_since_ms
            .compare_exchange(since, now_ms.max(1), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    /// Record the outcome of an attempted call. Returns `true` when the call
    /// failed because the function is not registered (the caller should stay
    /// quiet — this already logged once); `false` otherwise.
    pub fn observe(&self, error: Option<&Error>, now_ms: u64) -> bool {
        match error {
            Some(e) if is_function_not_found(e) => {
                let previous = self.missing_since_ms.swap(now_ms.max(1), Ordering::Relaxed);
                if previous == 0 {
                    tracing::warn!(
                        dependency = self.what,
                        error = %e,
                        retry_after_ms = RETRY_AFTER_MS,
                        "optional dependency is not registered; skipping it until a later re-probe"
                    );
                }
                true
            }
            _ => {
                // The call reached a registered function (whether or not it
                // succeeded), so the dependency is present.
                if self.missing_since_ms.swap(0, Ordering::Relaxed) != 0 {
                    tracing::info!(
                        dependency = self.what,
                        "optional dependency is available again"
                    );
                }
                false
            }
        }
    }

    /// Whether the dependency is currently believed missing.
    pub fn is_missing(&self) -> bool {
        self.missing_since_ms.load(Ordering::Relaxed) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn not_found() -> Error {
        Error::Remote {
            code: "function_not_found".into(),
            message: "Function stream::set not found in namespace default.".into(),
            stacktrace: None,
        }
    }

    #[test]
    fn classifies_only_the_missing_function_code() {
        assert!(is_function_not_found(&not_found()));
        assert!(!is_function_not_found(&Error::Timeout));
        assert!(!is_function_not_found(&Error::Remote {
            code: "NOT_FOUND".into(),
            message: "session not found".into(),
            stacktrace: None,
        }));
    }

    // Prevents: one warning (and one failed bus call) per stdout line when the
    // stream worker is not installed.
    #[test]
    fn a_missing_function_is_skipped_until_the_retry_window() {
        let dep = OptionalDependency::new("stream::set");
        assert!(dep.should_try(1_000));
        assert!(dep.observe(Some(&not_found()), 1_000));
        assert!(dep.is_missing());
        assert!(!dep.should_try(1_001));
        assert!(!dep.should_try(1_000 + RETRY_AFTER_MS - 1));
        // Exactly one caller claims the re-probe once the window elapses.
        assert!(dep.should_try(1_000 + RETRY_AFTER_MS));
        assert!(!dep.should_try(1_000 + RETRY_AFTER_MS));
    }

    #[test]
    fn a_later_install_is_picked_up_on_the_re_probe() {
        let dep = OptionalDependency::new("stream::set");
        dep.observe(Some(&not_found()), 5);
        assert!(dep.should_try(5 + RETRY_AFTER_MS));
        assert!(!dep.observe(None, 5 + RETRY_AFTER_MS));
        assert!(!dep.is_missing());
        assert!(dep.should_try(6 + RETRY_AFTER_MS));
    }

    // Prevents: a transient failure (timeout, handler error) silencing a
    // dependency that IS installed.
    #[test]
    fn other_failures_do_not_mark_the_dependency_missing() {
        let dep = OptionalDependency::new("session::append");
        assert!(!dep.observe(Some(&Error::Timeout), 10));
        assert!(!dep.is_missing());
        assert!(dep.should_try(11));
    }
}
