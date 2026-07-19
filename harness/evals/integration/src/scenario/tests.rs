use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::types::scenario::Classification;

use super::report::{classify, ProcessState};

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
