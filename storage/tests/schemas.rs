//! This file owns the `mod e2e;` declaration so Cargo compiles the
//! per-provider env-gated e2e tests under `tests/e2e/` as part of one binary.

mod e2e;

#[test]
fn binary_name_matches_manifest() {
    assert_eq!(storage::worker_name(), "storage");
}
