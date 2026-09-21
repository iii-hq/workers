//! Group state transitions — pure, and the only place they are decided.
//!
//! Two rules shape everything here. **Only a human resolves or ignores**: the
//! agent's single write moves a group forward and never backward. And a
//! transition is computed from the state read *in the transaction that
//! writes it*, so the real race — a person clicking Resolve while a
//! diagnosis request is still in flight — resolves in the person's favour:
//! the diagnosis lands as a new version and the group stays resolved.

use crate::{
    GroupChangeReasonV1 as Reason, GroupStatusV1 as Status, IgnoreBaselineV1, IgnoreRuleV1,
    SentinelError,
};

/// What a transition reads. Mirrors the columns of `sentinel_groups` that
/// participate in a decision, so the SQL and this module cannot drift.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupState {
    pub status: Status,
    /// Occurrences recorded *before* the one being handled.
    pub occurrence_count: u64,
    pub previous_status: Option<Status>,
    pub resolved_version: Option<String>,
    pub resolve_until_version_change: bool,
    pub ignore_rule: Option<IgnoreRuleV1>,
    pub ignore_baseline: Option<IgnoreBaselineV1>,
    pub has_diagnosis: bool,
}

/// The decision: where the group lands, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub status: Status,
    pub reason: Option<Reason>,
    /// The ignore rule and its baseline are dropped from the row.
    pub clear_ignore: bool,
}

impl Transition {
    fn to(status: Status, reason: Reason) -> Self {
        Self {
            status,
            reason: Some(reason),
            clear_ignore: false,
        }
    }

    fn stay(status: Status) -> Self {
        Self {
            status,
            reason: None,
            clear_ignore: false,
        }
    }

    fn clearing(mut self) -> Self {
        self.clear_ignore = true;
        self
    }

    /// Whether the status itself moved.
    pub fn moved(&self, from: Status) -> bool {
        self.status != from
    }
}

/// A new occurrence arrived. Counting always happens; this decides whether
/// the group also changes state.
pub fn on_occurrence(state: &GroupState, worker_version: Option<&str>) -> Transition {
    match state.status {
        Status::Resolved => {
            if regresses(state, worker_version) {
                Transition::to(Status::Regressed, Reason::Regression)
            } else {
                // The fix is not deployed here yet: keep counting without
                // reopening, which is what `until version change` buys.
                Transition::stay(Status::Resolved)
            }
        }
        Status::Ignored => {
            if ignore_expired(state, worker_version) {
                Transition::to(Status::New, Reason::IgnoreExpired).clearing()
            } else {
                Transition::stay(Status::Ignored)
            }
        }
        other => Transition::stay(other),
    }
}

fn regresses(state: &GroupState, worker_version: Option<&str>) -> bool {
    if !state.resolve_until_version_change {
        return true;
    }
    match (worker_version, state.resolved_version.as_deref()) {
        // A different version is running: the fix shipped and the failure is
        // back.
        (Some(seen), Some(resolved)) => seen != resolved,
        // Nothing to compare against. Claiming a regression on an unknown
        // version would fire on every occurrence in a dev stack, which is
        // exactly where versions are unknown.
        _ => false,
    }
}

fn ignore_expired(state: &GroupState, worker_version: Option<&str>) -> bool {
    let Some(rule) = &state.ignore_rule else {
        // Ignored with no rule is a broken row, not a permanent ignore.
        return true;
    };
    let baseline = state.ignore_baseline.clone().unwrap_or_default();
    match rule {
        IgnoreRuleV1::Forever => false,
        IgnoreRuleV1::Occurrences { count } => {
            let since = (state.occurrence_count + 1).saturating_sub(baseline.occurrence_count);
            since >= *count
        }
        IgnoreRuleV1::VersionChange => match (worker_version, baseline.worker_version.as_deref()) {
            (Some(seen), Some(base)) => seen != base,
            _ => false,
        },
    }
}

/// A human resolved the group.
pub fn resolve(state: &GroupState) -> Result<Transition, SentinelError> {
    match state.status {
        Status::New | Status::Diagnosed | Status::Regressed => {
            Ok(Transition::to(Status::Resolved, Reason::Resolved).clearing())
        }
        // Idempotent: a second click re-states the rule rather than failing.
        Status::Resolved => Ok(Transition::stay(Status::Resolved)),
        from => Err(invalid(from, Status::Resolved)),
    }
}

/// A human ignored the group, with a scope.
pub fn ignore(state: &GroupState) -> Result<Transition, SentinelError> {
    match state.status {
        Status::New | Status::Diagnosed | Status::Regressed => {
            Ok(Transition::to(Status::Ignored, Reason::Ignored))
        }
        // Changing the rule on an already-ignored group.
        Status::Ignored => Ok(Transition::stay(Status::Ignored)),
        from => Err(invalid(from, Status::Ignored)),
    }
}

/// A human took the group off the ignore list.
pub fn unignore(state: &GroupState) -> Result<Transition, SentinelError> {
    match state.status {
        Status::Ignored => Ok(Transition::to(Status::New, Reason::Reopened).clearing()),
        from => Err(invalid(from, Status::New)),
    }
}

/// A human put a resolved or ignored group back in the open list.
pub fn reopen(state: &GroupState) -> Result<Transition, SentinelError> {
    match state.status {
        Status::Resolved | Status::Ignored => {
            Ok(Transition::to(Status::New, Reason::Reopened).clearing())
        }
        from => Err(invalid(from, Status::New)),
    }
}

/// An investigation's first pass started.
pub fn on_investigate(state: &GroupState) -> Result<Transition, SentinelError> {
    match state.status {
        Status::New | Status::Diagnosed | Status::Regressed => {
            Ok(Transition::to(Status::Investigating, Reason::Investigating))
        }
        from => Err(invalid(from, Status::Investigating)),
    }
}

/// The agent recorded a diagnosis.
///
/// It moves the group forward from `investigating` or `new` and **never**
/// undoes a decision: on a resolved, ignored or regressed group the
/// diagnosis is attached and the status stays. `new` is here for the session
/// that outlived its first pass — stopped, or ended without recording — and
/// records later.
pub fn on_record(state: &GroupState) -> Transition {
    match state.status {
        Status::Investigating | Status::New => Transition::to(Status::Diagnosed, Reason::Diagnosed),
        Status::Diagnosed => Transition::stay(Status::Diagnosed),
        kept => Transition::stay(kept),
    }
}

/// The first pass ended. Without a diagnosis the group goes back where it
/// came from, so a failed or empty pass leaves no trace in the lifecycle.
pub fn on_first_pass_end(state: &GroupState, recorded: bool) -> Transition {
    if state.status != Status::Investigating || recorded || state.has_diagnosis {
        return Transition::stay(state.status);
    }
    let back_to = state.previous_status.unwrap_or(Status::New);
    Transition {
        status: back_to,
        reason: None,
        clear_ignore: false,
    }
}

fn invalid(from: Status, to: Status) -> SentinelError {
    SentinelError::InvalidTransition {
        from: from.as_str().to_string(),
        to: to.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(status: Status) -> GroupState {
        GroupState {
            status,
            ..GroupState::default()
        }
    }

    #[test]
    fn an_occurrence_on_an_open_group_only_counts() {
        for status in [
            Status::New,
            Status::Investigating,
            Status::Diagnosed,
            Status::Regressed,
        ] {
            let transition = on_occurrence(&state(status), Some("1.2.3"));
            assert_eq!(transition.status, status);
            assert_eq!(transition.reason, None);
        }
    }

    #[test]
    fn a_plain_resolve_reopens_on_the_next_occurrence() {
        let resolved = GroupState {
            status: Status::Resolved,
            resolved_version: Some("1.2.3".into()),
            ..GroupState::default()
        };
        let transition = on_occurrence(&resolved, Some("1.2.3"));
        assert_eq!(transition.status, Status::Regressed);
        assert_eq!(transition.reason, Some(Reason::Regression));
    }

    #[test]
    fn resolve_until_version_change_waits_for_the_fix_to_ship() {
        let resolved = GroupState {
            status: Status::Resolved,
            resolved_version: Some("1.2.3".into()),
            resolve_until_version_change: true,
            ..GroupState::default()
        };

        let same = on_occurrence(&resolved, Some("1.2.3"));
        assert_eq!(same.status, Status::Resolved, "the fix is not deployed yet");
        assert_eq!(same.reason, None);

        let shipped = on_occurrence(&resolved, Some("1.3.0"));
        assert_eq!(shipped.status, Status::Regressed);

        let unknown = on_occurrence(&resolved, None);
        assert_eq!(
            unknown.status,
            Status::Resolved,
            "an unknown version must not manufacture a regression"
        );
    }

    #[test]
    fn an_ignore_by_occurrences_reopens_exactly_when_it_promised() {
        let ignored = GroupState {
            status: Status::Ignored,
            occurrence_count: 108,
            ignore_rule: Some(IgnoreRuleV1::Occurrences { count: 50 }),
            ignore_baseline: Some(IgnoreBaselineV1 {
                occurrence_count: 60,
                worker_version: None,
            }),
            ..GroupState::default()
        };
        // 109th occurrence is the 49th since the baseline: still ignored.
        assert_eq!(on_occurrence(&ignored, None).status, Status::Ignored);

        let one_more = GroupState {
            occurrence_count: 109,
            ..ignored
        };
        let transition = on_occurrence(&one_more, None);
        assert_eq!(transition.status, Status::New);
        assert_eq!(transition.reason, Some(Reason::IgnoreExpired));
        assert!(transition.clear_ignore);
    }

    #[test]
    fn an_ignore_until_version_change_reopens_only_on_a_new_version() {
        let ignored = GroupState {
            status: Status::Ignored,
            ignore_rule: Some(IgnoreRuleV1::VersionChange),
            ignore_baseline: Some(IgnoreBaselineV1 {
                occurrence_count: 3,
                worker_version: Some("0.23.0".into()),
            }),
            ..GroupState::default()
        };
        assert_eq!(
            on_occurrence(&ignored, Some("0.23.0")).status,
            Status::Ignored
        );
        assert_eq!(on_occurrence(&ignored, None).status, Status::Ignored);
        assert_eq!(on_occurrence(&ignored, Some("0.23.1")).status, Status::New);
    }

    #[test]
    fn an_ignore_forever_never_expires_on_its_own() {
        let ignored = GroupState {
            status: Status::Ignored,
            occurrence_count: 10_000,
            ignore_rule: Some(IgnoreRuleV1::Forever),
            ignore_baseline: Some(IgnoreBaselineV1::default()),
            ..GroupState::default()
        };
        assert_eq!(
            on_occurrence(&ignored, Some("9.9.9")).status,
            Status::Ignored
        );
    }

    #[test]
    fn human_decisions_are_allowed_from_the_open_states_and_refused_elsewhere() {
        for status in [Status::New, Status::Diagnosed, Status::Regressed] {
            assert_eq!(resolve(&state(status)).unwrap().status, Status::Resolved);
            assert_eq!(ignore(&state(status)).unwrap().status, Status::Ignored);
        }
        // An investigation is running: stop it first, so a first pass cannot
        // keep working on a group somebody already closed.
        let error = resolve(&state(Status::Investigating)).expect_err("refused");
        assert!(error.to_string().contains("investigating"));
        assert_eq!(error.code(), "invalid_transition");

        assert!(resolve(&state(Status::Ignored)).is_err());
        assert!(unignore(&state(Status::New)).is_err());
    }

    #[test]
    fn repeating_a_decision_is_not_an_error() {
        assert_eq!(resolve(&state(Status::Resolved)).unwrap().reason, None);
        assert_eq!(ignore(&state(Status::Ignored)).unwrap().reason, None);
    }

    #[test]
    fn reopen_covers_both_closed_states_and_unignore_only_the_one() {
        assert_eq!(
            reopen(&state(Status::Resolved)).unwrap().status,
            Status::New
        );
        assert_eq!(reopen(&state(Status::Ignored)).unwrap().status, Status::New);
        assert!(reopen(&state(Status::Diagnosed)).is_err());
        assert_eq!(
            unignore(&state(Status::Ignored)).unwrap().status,
            Status::New
        );
    }

    #[test]
    fn a_record_moves_a_group_forward_but_never_undoes_a_person() {
        assert_eq!(
            on_record(&state(Status::Investigating)).status,
            Status::Diagnosed
        );
        assert_eq!(on_record(&state(Status::New)).status, Status::Diagnosed);

        for kept in [Status::Resolved, Status::Ignored, Status::Regressed] {
            let transition = on_record(&state(kept));
            assert_eq!(
                transition.status, kept,
                "a diagnosis must not reopen {kept:?}"
            );
            assert_eq!(transition.reason, None);
        }
    }

    #[test]
    fn a_first_pass_that_recorded_nothing_leaves_no_trace() {
        let investigating = GroupState {
            status: Status::Investigating,
            previous_status: Some(Status::Regressed),
            ..GroupState::default()
        };
        assert_eq!(
            on_first_pass_end(&investigating, false).status,
            Status::Regressed,
            "the group goes back where it came from"
        );
        assert_eq!(
            on_first_pass_end(&investigating, true).status,
            Status::Investigating,
            "a recorded diagnosis already moved it"
        );

        let no_previous = state(Status::Investigating);
        assert_eq!(on_first_pass_end(&no_previous, false).status, Status::New);
    }

    #[test]
    fn investigating_is_refused_while_the_group_is_closed() {
        assert_eq!(
            on_investigate(&state(Status::New)).unwrap().status,
            Status::Investigating
        );
        assert!(on_investigate(&state(Status::Resolved)).is_err());
        assert!(on_investigate(&state(Status::Ignored)).is_err());
    }
}
