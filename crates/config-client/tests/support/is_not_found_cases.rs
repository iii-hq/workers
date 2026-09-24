// Shared regression cases for the configuration missing-entry classifier.
//
// Each worker keeps its own private `is_not_found` (the workers deliberately do
// not take a shared dependency just for this), so this file is `include!`d into
// every worker's `#[cfg(test)]` module and exercised against that worker's real
// helper. The classification contract is asserted once here, from a single set
// of inputs, without copying the algorithm into each crate.
//
// A missing entry is the configuration worker's standalone `NOT_FOUND` code
// inside the SDK's `remote error ({code}): {message}` envelope, optionally
// behind this worker's own `configuration::get failed after N attempts:` retry
// wrapper. Everything else -- an unrelated/compound code, a `NOT_FOUND` buried
// in a message, a nested or doubled envelope, a foreign wrapper, a wrong
// attempt count, or a malformed envelope -- is a real failure that must
// propagate, so a service error never seeds a default over a stored value.

/// Inputs a correct classifier MUST read as "nothing stored yet" (seed a default).
const MISSING_ENTRY_INPUTS: &[&str] = &[
    // Bare code, tolerant of surrounding whitespace (back-compat).
    "NOT_FOUND",
    "   NOT_FOUND   ",
    // The SDK envelope, with and without a message.
    "remote error (NOT_FOUND): configuration 'x' not found",
    "remote error (NOT_FOUND):",
    // The exact retry wrapper this worker produces around the get envelope.
    "configuration::get failed after 3 attempts: remote error (NOT_FOUND): missing",
];

/// Inputs a correct classifier MUST reject and let propagate as a failure.
const FAILURE_INPUTS: &[&str] = &[
    // Unrelated or compound codes.
    "function_not_found",
    "statement_not_found",
    "RESOURCE_NOT_FOUND",
    "remote error (NOT_FOUND_EXTRA): missing",
    // Right token, wrong place: a different code whose message mentions NOT_FOUND.
    "remote error (ADAPTER_ERROR): NOT_FOUND",
    // Nested or doubled envelopes.
    "remote error (OTHER): remote error (NOT_FOUND): nested",
    "configuration::get failed after 3 attempts: configuration::get failed after 3 attempts: remote error (NOT_FOUND): missing",
    // The envelope embedded behind an unrelated wrapper.
    "handler failed: remote error (NOT_FOUND): detail",
    // A foreign retry wrapper (different function) or a wrong attempt count.
    "configuration::set failed after 3 attempts: remote error (NOT_FOUND): missing",
    "configuration::get failed after 5 attempts: remote error (NOT_FOUND): missing",
    // Malformed envelope missing the `): ` terminator.
    "remote error (NOT_FOUND)",
    // The retry wrapper around a non-NOT_FOUND code.
    "configuration::get failed after 3 attempts: function_not_found",
];

/// Assert a `&str`-based `is_not_found` classifier honours the contract above.
///
/// Passed the worker's real private helper (or, for the typed workers, a thin
/// closure over it) so the shared inputs test the shipped code path.
fn assert_missing_entry_contract(is_not_found: impl Fn(&str) -> bool) {
    for input in MISSING_ENTRY_INPUTS {
        assert!(is_not_found(input), "should read as a missing entry: {input:?}");
    }
    for input in FAILURE_INPUTS {
        assert!(!is_not_found(input), "must propagate as a failure: {input:?}");
    }
}
