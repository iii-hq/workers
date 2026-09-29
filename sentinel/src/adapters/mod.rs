//! Turning engine telemetry into one shape.
//!
//! Everything downstream — fingerprint, group, evidence, investigation —
//! works on [`ErrorEvent`], never on a span or a log record. That is what
//! makes a third source (a harness turn, a reported crash) an adapter rather
//! than a second pipeline.

pub mod log;
pub mod trace;

use crate::{evidence::EvidenceBundleV1, ErrorSourceV1};

/// A failure, normalized and source-agnostic: who failed, what failed, where,
/// when, in which version, in whose session.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorEvent {
    pub source: ErrorSourceV1,
    /// Unique per physical event, so the same span arriving twice is one
    /// occurrence.
    pub dedupe_key: String,
    pub at_ms: i64,
    /// The worker that **owns** the failing function, resolved from the
    /// registry — not the `service_name` on the span, which can be the
    /// engine's own `call` hop when the callee's span has not arrived.
    pub service_name: String,
    pub namespace: String,
    pub function_id: Option<String>,
    pub span_name: Option<String>,
    pub exception_type: Option<String>,
    /// Already redacted.
    pub message: String,
    pub stacktrace: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub worker_version: Option<String>,
    pub namespace_ambiguous: bool,
    pub evidence: Option<EvidenceBundleV1>,
}

/// The function a span is about: the attribute when the worker set one, else
/// the name the engine gave the dispatch.
///
/// Worker-side `execute <fn>` spans carry no `function_id` attribute — the
/// engine derives it from the span name — so every consumer has to do this
/// the same way or attribution silently differs between them.
pub fn function_of(name: &str, attributes: &impl AttributeSource) -> Option<String> {
    if let Some(function_id) = attributes.attribute("function_id") {
        return Some(function_id);
    }
    for prefix in ["execute ", "call "] {
        if let Some(rest) = name.strip_prefix(prefix) {
            let function_id = rest.trim();
            if !function_id.is_empty() {
                return Some(function_id.to_string());
            }
        }
    }
    None
}

/// Read one attribute, whatever shape the source stores them in: spans use an
/// array of pairs, logs use an object.
pub trait AttributeSource {
    fn attribute(&self, key: &str) -> Option<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Attributes(BTreeMap<String, String>);

    impl AttributeSource for Attributes {
        fn attribute(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    fn attributes(pairs: &[(&str, &str)]) -> Attributes {
        Attributes(
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        )
    }

    #[test]
    fn an_explicit_attribute_wins_over_the_span_name() {
        assert_eq!(
            function_of(
                "execute other::fn",
                &attributes(&[("function_id", "state::cas")])
            ),
            Some("state::cas".into())
        );
    }

    #[test]
    fn a_worker_span_carries_its_function_only_in_its_name() {
        assert_eq!(
            function_of("execute harness::send", &attributes(&[])),
            Some("harness::send".into())
        );
        assert_eq!(
            function_of("call state::compare-and-set", &attributes(&[])),
            Some("state::compare-and-set".into())
        );
    }

    #[test]
    fn a_span_that_is_not_a_dispatch_has_no_function() {
        assert_eq!(function_of("harness::turn step", &attributes(&[])), None);
        assert_eq!(function_of("execute ", &attributes(&[])), None);
    }
}
