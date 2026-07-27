use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::types::scenario::{Classification, IntegrationResultV1};
use crate::types::script::SchemaVersion1;

use super::report::{classify, rpc_failure, ProcessState};
use super::runner::{combine_teardown, result_digest};

#[test]
fn process_exit_is_combined_with_phase_failure_by_precedence() {
    let timeout = RunError::new(RunPhase::Await, RunErrorKind::Timeout, "deadline");
    assert_eq!(
        classify(Some(&timeout), ProcessState::Crashed),
        Classification::ProcessCrash
    );

    let runner = RunError::new(RunPhase::Collect, RunErrorKind::Runner, "artifact");
    assert_eq!(
        classify(Some(&runner), ProcessState::Crashed),
        Classification::RunnerError
    );
    assert_eq!(
        classify(None, ProcessState::Crashed),
        Classification::ProcessCrash
    );
}

#[test]
fn incomplete_teardown_always_becomes_runner_error() {
    assert_eq!(
        combine_teardown(Classification::Pass, false, Ok(())),
        Classification::RunnerError
    );
    assert_eq!(
        combine_teardown(Classification::ContractFailure, false, Ok(())),
        Classification::RunnerError
    );
}

#[test]
fn result_digest_matches_the_exact_persisted_bytes() {
    let result = IntegrationResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: "INT-DIGEST".into(),
        classification: Classification::Pass,
        failure: None,
        artifacts: vec!["scenarios/INT-DIGEST/transcript.json".into()],
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("result.json");
    crate::artifacts::write_json(temp.path(), &path, &result).unwrap();
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        result_digest(&result),
        crate::canonical::sha256_of_bytes(&bytes)
    );
}

#[test]
fn public_rpc_failures_do_not_invent_subject_timeouts() {
    let error = rpc_failure(
        RunPhase::Send,
        RunErrorKind::Contract,
        "harness::send failed",
        "transport deadline".into(),
    );
    assert_eq!(error.classification(), Classification::ContractFailure);
}
