use security_scan::{
    build_analysis_plan, AnalysisConfigV1, RunRecordV1, RunStatusV1, ScanModeV1,
    ANALYSIS_READ_FUNCTIONS,
};

fn analysis_config() -> AnalysisConfigV1 {
    AnalysisConfigV1 {
        model: "security-review-model".into(),
        provider: Some("router".into()),
        max_turns: 4,
        max_output_tokens: 8_000,
        max_total_tokens: 50_000,
        max_cost_usd: Some(2.0),
    }
}

fn queued_run() -> RunRecordV1 {
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: "sec_0123456789abcdef01234567".into(),
        repository: "iii-hq/iii".into(),
        target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
        resolved_from_head: false,
        mode: ScanModeV1::Suggest,
        model: None,
        provider: None,
        operation_nonce: "private_nonce".into(),
        status: RunStatusV1::Queued,
        attempt: 1,
        step: 0,
        step_failures: 0,
        materialized: None,
        harness: None,
        report: None,
        error: None,
        created_at: 1,
        updated_at: 1,
        completed_at: None,
    }
}

#[test]
fn analysis_plan_is_scoped_to_an_isolated_worktree_and_read_only_functions() {
    let plan = build_analysis_plan(
        &queued_run(),
        "/private/tmp/wt_security_scan",
        &analysis_config(),
    );

    assert_eq!(plan.filesystem_root, "/private/tmp/wt_security_scan");
    assert!(plan.unattended);
    assert_eq!(plan.allowed_functions, ANALYSIS_READ_FUNCTIONS);
    assert!(plan.allowed_functions.iter().all(|function| {
        !function.starts_with("shell::")
            && !function.contains("create")
            && !function.contains("update")
            && !function.contains("delete")
            && !function.contains("move")
    }));
    assert!(plan.system_prompt.contains("untrusted review data"));
    assert!(plan.system_prompt.contains("Never execute repository code"));
    assert!(plan
        .denied_functions
        .iter()
        .any(|function| function == "github::*"));
    assert!(plan
        .denied_functions
        .iter()
        .any(|function| function == "shell::*"));
    assert!(plan.system_prompt.contains("concrete remediation plan"));
    assert!(plan.message.contains("dependencies and packages"));
    assert!(plan.message.contains("secrets and credentials"));
    assert!(plan.message.contains("CI/release weaknesses"));
    assert!(plan
        .message
        .contains("Populate the assessments object for every area"));
    assert!(plan.output_schema.get("properties").is_some());
    assert!(plan
        .output_schema
        .get("required")
        .and_then(|required| required.as_array())
        .is_some_and(|required| required.iter().any(|field| field == "assessments")));
    assert_eq!(plan.model, "security-review-model");
    assert_eq!(plan.provider.as_deref(), Some("router"));
    assert_eq!(plan.max_turns, 4);
    assert_eq!(plan.max_total_tokens, 50_000);
}

#[test]
fn analysis_plan_is_deterministic_for_queue_redelivery() {
    let run = queued_run();
    let first = build_analysis_plan(&run, "/private/tmp/wt_security_scan", &analysis_config());
    let second = build_analysis_plan(&run, "/private/tmp/wt_security_scan", &analysis_config());

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.idempotency_key, second.idempotency_key);
    assert_eq!(first.idempotency_key, "private_nonce:attempt:1:analysis");

    let mut retry = run;
    retry.attempt = 2;
    let retried = build_analysis_plan(&retry, "/private/tmp/wt_security_scan", &analysis_config());
    assert_ne!(first.session_id, retried.session_id);
    assert_ne!(first.idempotency_key, retried.idempotency_key);
}

#[test]
fn analysis_plan_splits_a_composer_catalog_model_and_does_not_keep_the_operator_provider() {
    let mut run = queued_run();
    run.model = Some("deepseek::deepseek-v4-flash".into());
    run.provider = None;
    let plan = build_analysis_plan(&run, "/private/tmp/wt_security_scan", &analysis_config());
    assert_eq!(plan.model, "deepseek-v4-flash");
    assert_eq!(plan.provider.as_deref(), Some("deepseek"));
}

#[test]
fn analysis_plan_keeps_an_explicit_provider_on_a_request_model() {
    let mut run = queued_run();
    run.model = Some("gpt-5.6-terra".into());
    run.provider = Some("openai-codex".into());
    let plan = build_analysis_plan(&run, "/private/tmp/wt_security_scan", &analysis_config());
    assert_eq!(plan.model, "gpt-5.6-terra");
    assert_eq!(plan.provider.as_deref(), Some("openai-codex"));
}
