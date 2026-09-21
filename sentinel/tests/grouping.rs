//! The grouping rules, as data.
//!
//! Normalization and redaction decide what a group *is* and what leaves the
//! machine, so they are reviewed as a table of real messages rather than as
//! regular expressions. The fixture file is versioned with the rules: change
//! a rule and every existing group's fingerprint changes, so the version
//! moves and the old file stays as the record of what the old fingerprints
//! meant.

use sentinel::normalize::NORMALIZER_VERSION;
use sentinel::{FingerprintConfigV1, Normalizer, Redactor};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct NormalizeCase {
    note: String,
    input: String,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct RedactCase {
    note: String,
    input: String,
    expected: String,
    hits: u64,
}

fn fixture<T: serde::de::DeserializeOwned>(relative: &str) -> Vec<T> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

#[test]
fn the_fixture_file_tracks_the_normalizer_version() {
    // A rule change without a new fixture file would silently re-group every
    // existing error under the old version's name.
    assert_eq!(NORMALIZER_VERSION, 1);
    assert!(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!(
                "tests/fixtures/normalize/v{NORMALIZER_VERSION}.json"
            ))
            .exists(),
        "every normalizer version keeps its own fixture file"
    );
}

#[test]
fn normalization_matches_the_versioned_fixtures() {
    let normalizer = Normalizer::new(&FingerprintConfigV1::default().identity_numbers);
    for case in fixture::<NormalizeCase>(&format!("normalize/v{NORMALIZER_VERSION}.json")) {
        assert_eq!(
            normalizer.normalize(&case.input),
            case.expected,
            "{}",
            case.note
        );
    }
}

#[test]
fn redaction_matches_the_versioned_fixtures() {
    let redactor = Redactor::default();
    for case in fixture::<RedactCase>("redact/v1.json") {
        let (redacted, hits) = redactor.redact(&case.input);
        assert_eq!(redacted, case.expected, "{}", case.note);
        assert_eq!(hits, case.hits, "{}", case.note);
    }
}

#[test]
fn no_fixture_leaves_a_secret_behind() {
    // The expectations above are hand-written, so this is the independent
    // check: whatever the rules did, none of the inputs' secrets survive.
    let redactor = Redactor::default();
    for case in fixture::<RedactCase>("redact/v1.json") {
        if case.hits == 0 {
            continue;
        }
        let (redacted, _) = redactor.redact(&case.input);
        for secret in [
            "hunter2",
            "s3cr3t",
            "eyJhbGciOi-abc_def",
            "sk-proj0123456789abcdef",
            "ghp_0123456789abcdefghijABCD",
            "AKIAIOSFODNN7EXAMPLE",
            "MIIEowIBAAKCAQEA",
            "someone@example.com",
        ] {
            assert!(
                !redacted.contains(secret),
                "{} left {secret} in: {redacted}",
                case.note
            );
        }
    }
}
