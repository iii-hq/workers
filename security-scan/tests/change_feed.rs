//! Drives the change trigger types the way the engine does: register
//! callbacks, a committed change, then unregister callbacks.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use security_scan::events::{
    ActionChangedEventV1, ChangeDeliverer, ChangeFeed, ChangeKind, ChangeTriggerHandler, Delivery,
    RunChangedEventV1, ACTION_CHANGED, RUN_CHANGED,
};
use security_scan::{
    RunRecordV1, RunStatusV1, ScanModeV1, SecurityActionKindV1, SecurityActionRecordV1,
    SecurityActionStatusV1,
};
use serde_json::{json, Value};

#[derive(Default)]
struct Recorder(Mutex<Vec<Delivery>>);

#[async_trait]
impl ChangeDeliverer for Recorder {
    async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
        self.0.lock().unwrap().push(delivery);
        Ok(())
    }
}

fn binding(id: &str, function_id: &str, config: Value, namespace: Option<&str>) -> TriggerConfig {
    TriggerConfig {
        id: id.into(),
        function_id: function_id.into(),
        config,
        metadata: None,
        namespace: namespace.map(str::to_owned),
    }
}

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

#[tokio::test]
async fn ui_bindings_receive_committed_changes_until_unregistered() {
    let recorder = Arc::new(Recorder::default());
    let feed = Arc::new(ChangeFeed::new(recorder.clone()));
    let runs = ChangeTriggerHandler::new(ChangeKind::Run, feed.clone());
    let actions = ChangeTriggerHandler::new(ChangeKind::Action, feed.clone());

    // A console tab binds every run change; another consumer scopes to one repository.
    runs.register_trigger(binding(
        "tab",
        "iii::security-scan-ui::runs::browser-1",
        json!({}),
        Some("default"),
    ))
    .await
    .unwrap();
    runs.register_trigger(binding(
        "scoped",
        "audit::on-run",
        json!({ "repository": "workers" }),
        None,
    ))
    .await
    .unwrap();
    actions
        .register_trigger(binding(
            "tab-actions",
            "iii::security-scan-ui::actions::r1::browser-1",
            Value::Null,
            Some("default"),
        ))
        .await
        .unwrap();

    let queued = run("iii", "sec_1", RunStatusV1::Queued, 10);
    let sent = feed
        .emit(
            ChangeKind::Run,
            &queued.repository,
            &queued.run_id,
            &RunChangedEventV1::from(&queued),
        )
        .await;
    assert_eq!(sent, 1, "the repository-scoped binding must not see iii");

    let action = SecurityActionRecordV1 {
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
    };
    assert_eq!(
        feed.emit(
            ChangeKind::Action,
            &action.repository,
            &action.run_id,
            &ActionChangedEventV1::from(&action),
        )
        .await,
        1
    );

    let seen = recorder.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].trigger_type, RUN_CHANGED);
    assert_eq!(
        seen[0].function_id,
        "iii::security-scan-ui::runs::browser-1"
    );
    assert_eq!(seen[0].namespace.as_deref(), Some("default"));
    assert_eq!(seen[0].payload["run_id"], "sec_1");
    assert_eq!(seen[0].payload["status"], "queued");
    assert_eq!(seen[0].payload["updated_at"], 10);
    assert_eq!(seen[1].trigger_type, ACTION_CHANGED);
    assert_eq!(seen[1].payload["action_id"], "act_1");

    // The tab closes: the engine sends unregister callbacks; nothing more is sent to it.
    runs.unregister_trigger(binding("tab", "", Value::Null, None))
        .await
        .unwrap();
    actions
        .unregister_trigger(binding("tab-actions", "", Value::Null, None))
        .await
        .unwrap();
    let workers_run = run("workers", "sec_2", RunStatusV1::Completed, 12);
    assert_eq!(
        feed.emit(
            ChangeKind::Run,
            &queued.repository,
            &queued.run_id,
            &RunChangedEventV1::from(&queued),
        )
        .await,
        0
    );
    assert_eq!(
        feed.emit(
            ChangeKind::Run,
            &workers_run.repository,
            &workers_run.run_id,
            &RunChangedEventV1::from(&workers_run),
        )
        .await,
        1
    );
    assert_eq!(
        feed.emit(
            ChangeKind::Action,
            &action.repository,
            &action.run_id,
            &ActionChangedEventV1::from(&action),
        )
        .await,
        0
    );
    let seen = recorder.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2].function_id, "audit::on-run");
    assert_eq!(seen[2].payload["repository"], "workers");
}

#[tokio::test]
async fn legacy_stream_shaped_configs_are_rejected() {
    let feed = Arc::new(ChangeFeed::new(Arc::new(Recorder::default())));
    let runs = ChangeTriggerHandler::new(ChangeKind::Run, feed.clone());
    let error = runs
        .register_trigger(binding(
            "old",
            "ui::runs",
            json!({ "stream_name": "security-scan:runs", "group_id": "all" }),
            None,
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown field"), "{error}");
    assert_eq!(feed.binding_count(ChangeKind::Run), 0);
}
