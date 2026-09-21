//! The investigation's permission model, tested as a manifest.
//!
//! There is no approval gate in this design: nothing an investigation calls
//! is ever held for a human to wave through. The harness policy is the whole
//! boundary, and these two lists are that policy — so a function added to
//! this worker later must not become reachable from inside an investigation
//! just because nobody thought about it.

use sentinel::functions::{catalog, INVESTIGATION_ALLOW, INVESTIGATION_DENY};

fn denied(id: &str) -> bool {
    INVESTIGATION_DENY
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => id.starts_with(prefix),
            None => *pattern == id,
        })
}

#[test]
fn every_registered_function_is_decided_one_way_or_the_other() {
    let undecided: Vec<&str> = catalog()
        .iter()
        .map(|spec| spec.function_id)
        .filter(|id| !INVESTIGATION_ALLOW.contains(id) && !denied(id))
        .collect();
    assert!(
        undecided.is_empty(),
        "these are reachable from an investigation by default: {undecided:?}"
    );
}

#[test]
fn deny_wins_over_allow_so_nothing_allowed_may_also_be_denied() {
    // The harness evaluates deny first (`harness/src/policy.rs`), so an id on
    // both lists is refused — which would silently disable a read the
    // investigation depends on.
    let contradicted: Vec<&str> = INVESTIGATION_ALLOW
        .iter()
        .copied()
        .filter(|id| denied(id))
        .collect();
    assert!(
        contradicted.is_empty(),
        "deny wins, so these are dead entries on the allow list: {contradicted:?}"
    );
}

#[test]
fn the_allow_list_carries_no_write_outside_this_worker() {
    // Everything an investigation may call is a read, except the one write
    // that is this worker's own record of the diagnosis.
    for id in INVESTIGATION_ALLOW {
        let is_read = id.starts_with("coder::")
            || id.starts_with("engine::functions::")
            || id == "sentinel::evidence::get"
            || id == "sentinel::trace::get"
            || id == "sentinel::logs::list";
        assert!(
            is_read || id == "sentinel::diagnosis::record",
            "{id} is neither a read nor the one permitted write"
        );
    }
}

#[test]
fn the_engines_raw_telemetry_is_denied_so_the_redacting_proxies_are_the_only_way_in() {
    for id in [
        "engine::traces::tree",
        "engine::traces::spans",
        "engine::logs::list",
    ] {
        assert!(denied(id), "{id} would bypass the redactor");
    }
    assert!(INVESTIGATION_ALLOW.contains(&"sentinel::trace::get"));
    assert!(INVESTIGATION_ALLOW.contains(&"sentinel::logs::list"));
}

#[test]
fn an_investigation_cannot_open_or_steer_another_one() {
    for id in [
        "sentinel::investigate",
        "sentinel::investigations::get",
        "sentinel::investigations::list",
        "sentinel::investigations::cancel",
        "sentinel::groups::resolve",
        "sentinel::groups::ignore",
        "harness::send",
        "session::append",
    ] {
        assert!(
            denied(id),
            "{id} must not be reachable from an investigation"
        );
    }
}

#[test]
fn nothing_can_run_code_or_touch_the_store() {
    for id in [
        "shell::exec",
        "database::execute",
        "state::compare-and-set",
        "queue::define",
        "storage::write",
        "github::api",
        "worktree::create",
        "configuration::set",
    ] {
        assert!(
            denied(id),
            "{id} must not be reachable from an investigation"
        );
    }
}
