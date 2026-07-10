//! Call-site span hiding: mark ONE outbound call's spans as internal.
//!
//! The `iii.tag.hidden` baggage entry (workers/docs/sops/
//! trace-hidden-functions.md § internal spans) rides the call like the
//! other `iii.tag.*` keys: the engine and the callee's span processor stamp
//! it onto every span started inside the scope — the engine `call <fn>`
//! span, the callee's `execute <fn>` handler span, and their descendants.
//! Trace UIs group spans carrying it into the span filter's INTERNAL
//! section, hidden by default; the value is the section's family label, so
//! pick a short human-readable one ("harness state", "session updates").
//!
//! Function-level `trace_hidden` registration metadata hides a whole
//! function; this hides one CALL SITE — use it when the same function is
//! meaningful from some callers and bookkeeping from others, or when the
//! callee (an engine built-in like `state::*`) is not ours to tag.

pub(crate) const HIDDEN_TAG_KEY: &str = "iii.tag.hidden";

/// Run `fut` with `iii.tag.hidden = family` in baggage. Scoped: the entry
/// does not leak past the call (see `run_with_baggage`).
pub(crate) async fn run_hidden<F, T>(family: &'static str, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    iii_helpers::observability::run_with_baggage(&[(HIDDEN_TAG_KEY, family)], fut).await
}
