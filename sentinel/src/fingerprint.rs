//! Group identity.
//!
//! Deterministic, costing one hash per event, and stable across restarts —
//! no model ever decides which group an occurrence belongs to. The key is
//! built from the parts that identify *the failure* rather than the run:
//! whose code failed (namespace and the worker that **owns** the function,
//! never the `service_name` of the span, which can be the engine's own `call`
//! hop), which function, which exception type, and the normalized message.

use sha2::{Digest, Sha256};

use crate::ErrorSourceV1;

/// The separator between key parts. A unit separator cannot appear in any
/// part, so `["a", "b"]` and `["ab"]` can never hash alike.
const SEP: u8 = 0x1f;

/// The identity of a failure, as 32 hex characters.
pub fn fingerprint(source: ErrorSourceV1, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(source.as_str().as_bytes());
    for part in parts {
        digest.update([SEP]);
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())[..32].to_string()
}

/// A span-sourced failure: namespace, owning worker, function, exception
/// type, normalized message.
pub fn trace_fingerprint(
    namespace: &str,
    owner_worker: &str,
    function_or_span: &str,
    exception_type: Option<&str>,
    normalized_message: &str,
) -> String {
    fingerprint(
        ErrorSourceV1::Trace,
        &[
            namespace,
            owner_worker,
            function_or_span,
            exception_type.unwrap_or_default(),
            normalized_message,
        ],
    )
}

/// A log-sourced failure. Logs carry no exception type, so that slot stays
/// empty and the call site — `code.function`, the tracing target, or the
/// instrumentation scope — takes the place of the function id.
pub fn log_fingerprint(
    namespace: &str,
    service_name: &str,
    call_site: &str,
    normalized_message: &str,
) -> String {
    fingerprint(
        ErrorSourceV1::Log,
        &[namespace, service_name, call_site, "", normalized_message],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_is_32_hex_characters_and_stable() {
        let first = trace_fingerprint(
            "default",
            "harness",
            "state::cas",
            Some("Cas"),
            "expected <n>",
        );
        let second = trace_fingerprint(
            "default",
            "harness",
            "state::cas",
            Some("Cas"),
            "expected <n>",
        );
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn every_part_of_the_key_changes_the_identity() {
        let base = trace_fingerprint(
            "default",
            "harness",
            "state::cas",
            Some("Cas"),
            "expected <n>",
        );
        let variants = [
            trace_fingerprint(
                "other",
                "harness",
                "state::cas",
                Some("Cas"),
                "expected <n>",
            ),
            trace_fingerprint(
                "default",
                "queue",
                "state::cas",
                Some("Cas"),
                "expected <n>",
            ),
            trace_fingerprint(
                "default",
                "harness",
                "state::get",
                Some("Cas"),
                "expected <n>",
            ),
            trace_fingerprint(
                "default",
                "harness",
                "state::cas",
                Some("Other"),
                "expected <n>",
            ),
            trace_fingerprint(
                "default",
                "harness",
                "state::cas",
                Some("Cas"),
                "missing <n>",
            ),
            trace_fingerprint("default", "harness", "state::cas", None, "expected <n>"),
        ];
        for variant in variants {
            assert_ne!(base, variant);
        }
    }

    #[test]
    fn the_source_is_part_of_the_identity() {
        // The same text seen as a span and as a log is two groups: one has a
        // trace to investigate, the other does not.
        let as_trace = trace_fingerprint(
            "default",
            "queue",
            "queue::poll",
            None,
            "redelivery exhausted",
        );
        let as_log = log_fingerprint("default", "queue", "queue::poll", "redelivery exhausted");
        assert_ne!(as_trace, as_log);
    }

    #[test]
    fn the_separator_keeps_adjacent_parts_from_blurring() {
        assert_ne!(
            trace_fingerprint("default", "ab", "c", None, "m"),
            trace_fingerprint("default", "a", "bc", None, "m")
        );
    }
}
