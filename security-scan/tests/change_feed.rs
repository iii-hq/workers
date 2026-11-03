//! The change functions as consumers see them: the events a committed run or
//! action change produces, and an emitter that calls nothing while the
//! functions are unobserved.

use std::sync::Arc;

use iii_sdk::IIIClient;
use security_scan::events::{
    self, ActionChangedEventV1, ChangeFeed, ChangeKind, RunChangedEventV1, ON_ACTION_CHANGED_FN,
    ON_RECONCILIATION_CHANGED_FN, ON_RUN_CHANGED_FN,
};
use security_scan::{
    RunRecordV1, RunStatusV1, ScanModeV1, SecurityActionKindV1, SecurityActionRecordV1,
    SecurityActionStatusV1,
};
use serde_json::json;

fn run(repository: &str, run_id: &str, status: RunStatusV1, updated_at: i64) -> RunRecordV1 {
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: run_id.into(),
        repository: repository.into(),
        target_sha: "a".repeat(40),
        resolved_from_head: false,
        mode: ScanModeV1::Scan,
        model: None,
        provider: None,
        operation_nonce: "nonce".into(),
        status,
        attempt: 1,
        step: 0,
        step_failures: 0,
        materialized: None,
        harness: None,
        report: None,
        error: None,
        created_at: 1,
        updated_at,
        completed_at: None,
    }
}

fn action() -> SecurityActionRecordV1 {
    SecurityActionRecordV1 {
        schema_version: "1".into(),
        action_id: "act_1".into(),
        run_id: "sec_1".into(),
        finding_index: 0,
        action: SecurityActionKindV1::Issue,
        repository: "iii".into(),
        target_sha: "a".repeat(40),
        github_full_name: "iii-hq/iii".into(),
        operation_nonce: "nonce".into(),
        status: SecurityActionStatusV1::Queued,
        attempt: 1,
        step: 0,
        step_failures: 0,
        materialized: None,
        harness: None,
        result: None,
        error: None,
        created_at: 1,
        updated_at: 11,
        completed_at: None,
        cleanup_completed_at: None,
    }
}

#[test]
fn committed_changes_project_to_small_public_events() {
    let queued = run("iii", "sec_1", RunStatusV1::Queued, 10);
    assert_eq!(
        serde_json::to_value(RunChangedEventV1::from(&queued)).unwrap(),
        json!({
            "run_id": "sec_1",
            "repository": "iii",
            "status": "queued",
            "attempt": 1,
            "updated_at": 10,
            "completed_at": null,
        })
    );
    assert_eq!(
        serde_json::to_value(ActionChangedEventV1::from(&action())).unwrap(),
        json!({
            "action_id": "act_1",
            "run_id": "sec_1",
            "repository": "iii",
            "status": "queued",
            "updated_at": 11,
        })
    );
}

#[test]
fn each_kind_names_its_function() {
    assert_eq!(ChangeKind::Run.function_id(), ON_RUN_CHANGED_FN);
    assert_eq!(
        ChangeKind::Reconciliation.function_id(),
        ON_RECONCILIATION_CHANGED_FN
    );
    assert_eq!(ChangeKind::Action.function_id(), ON_ACTION_CHANGED_FN);
}

#[tokio::test]
async fn an_unobserved_change_is_never_called() {
    // `IIIClient::new` builds local state only: no engine, no observers.
    let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
    events::register_functions(&iii);
    let feed = ChangeFeed::new(iii.clone());

    for kind in ChangeKind::ALL {
        assert!(!iii.is_observed(kind.function_id()));
    }
    let queued = run("iii", "sec_1", RunStatusV1::Queued, 10);
    assert!(
        !feed
            .emit(ChangeKind::Run, &RunChangedEventV1::from(&queued))
            .await
    );
    assert!(
        !feed
            .emit(ChangeKind::Action, &ActionChangedEventV1::from(&action()))
            .await
    );
}
