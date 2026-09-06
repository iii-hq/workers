#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnalysisConfigV1, HarnessRunV1, ScanModeV1, SecurityActionKindV1, SecurityActionRecordV1,
        SecurityActionStatusV1, SecurityFindingV1, SecurityReportV1, SeverityV1,
    };

    fn private_run(status: RunStatusV1) -> RunRecordV1 {
        RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_history".into(),
            repository: "iii-hq/iii".into(),
            target_sha: "a".repeat(40),
            resolved_from_head: false,
            mode: ScanModeV1::Scan,
            model: None,
            provider: None,
            operation_nonce: "private_nonce".into(),
            status,
            attempt: 1,
            step: 2,
            step_failures: 0,
            materialized: Some(MaterializedTargetV1 {
                worktree_id: "wt_private".into(),
                path: "/private/checkout".into(),
                base_sha: "a".repeat(40),
            }),
            harness: Some(HarnessRunV1 {
                session_id: "session_private".into(),
                turn_id: "turn_private".into(),
            }),
            report: None,
            error: None,
            created_at: 1,
            updated_at: 2,
            completed_at: None,
        }
    }

    fn dependabot_api_alert(number: u64) -> Value {
        json!({
            "number": number,
            "state": "open",
            "dependency": {
                "package": { "ecosystem": "cargo", "name": "demo" },
                "manifest_path": "Cargo.lock"
            },
            "security_advisory": {
                "ghsa_id": "GHSA-demo",
                "cve_id": "CVE-2026-1",
                "summary": "short summary",
                "severity": "high"
            },
            "security_vulnerability": {
                "vulnerable_version_range": "< 2.0.0"
            },
            "updated_at": "2026-01-02T00:00:00Z"
        })
    }

    #[test]
    fn run_queue_uses_the_existing_durable_fifo_worker() {
        let definition = queue_definition();
        assert_eq!(definition["queue"], RUN_QUEUE);
        assert_eq!(definition["config"]["type"], "fifo");
        assert_eq!(definition["config"]["message_group_field"], "repository");
        assert_eq!(definition["config"]["redeliver_on_engine_restart"], true);
    }

    #[test]
    fn action_queue_groups_by_action_id() {
        let definition = action_queue_definition();
        assert_eq!(definition["queue"], ACTION_QUEUE);
        assert_eq!(definition["config"]["type"], "fifo");
        assert_eq!(definition["config"]["message_group_field"], "action_id");
        assert_eq!(definition["config"]["redeliver_on_engine_restart"], true);
    }

    #[test]
    fn action_worktree_uses_the_exact_sha_and_a_distinct_nonce() {
        let action = SecurityActionRecordV1 {
            schema_version: "1".into(),
            action_id: "seca_fix".into(),
            run_id: "sec_completed".into(),
            finding_index: 0,
            action: SecurityActionKindV1::FixPr,
            repository: "iii-hq/iii".into(),
            target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
            github_full_name: "iii-hq/iii".into(),
            operation_nonce: "action_nonce".into(),
            status: SecurityActionStatusV1::Preparing,
            attempt: 1,
            step: 0,
            step_failures: 0,
            materialized: None,
            harness: None,
            result: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
            cleanup_completed_at: None,
        };
        let request = MaterializationRequest::for_action(&action);
        assert_eq!(request.target_sha, action.target_sha);
        assert_eq!(
            request.session_id,
            "security-scan-worktree-action-action_nonce-attempt-1"
        );
    }

    #[test]
    fn issue_harness_request_omits_filesystem_scope() {
        let action = SecurityActionRecordV1 {
            schema_version: "1".into(),
            action_id: "seca_issue".into(),
            run_id: "sec_completed".into(),
            finding_index: 0,
            action: SecurityActionKindV1::Issue,
            repository: "iii-hq/iii".into(),
            target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
            github_full_name: "iii-hq/iii".into(),
            operation_nonce: "action_nonce".into(),
            status: SecurityActionStatusV1::Queued,
            attempt: 1,
            step: 0,
            step_failures: 0,
            materialized: None,
            harness: None,
            result: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
            cleanup_completed_at: None,
        };
        let finding = SecurityFindingV1 {
            rule_id: "SEC-001".into(),
            severity: SeverityV1::High,
            title: "Unsafe default".into(),
            description: "Details".into(),
            evidence: "Evidence".into(),
            location: None,
            remediation: "Fix it".into(),
            suggested_patch: None,
        };
        let plan = crate::build_issue_plan(
            &action,
            &finding,
            &AnalysisConfigV1 {
                model: "model".into(),
                provider: None,
                max_turns: 4,
                max_output_tokens: 8_000,
                max_total_tokens: 50_000,
                max_cost_usd: Some(2.0),
            },
        );
        assert!(!plan.unattended);
        let request = harness_request(&plan);
        assert!(request["options"]["metadata"]
            .as_object()
            .expect("metadata object")
            .get("fs_scope")
            .is_none());
        let allow = request["options"]["functions"]["allow"]
            .as_array()
            .expect("allow array");
        assert_eq!(allow, &vec![json!("github::issue::create")]);
        let deny = request["options"]["functions"]["deny"]
            .as_array()
            .expect("deny array");
        assert!(deny.iter().any(|value| value == "github::pr::merge"));
    }

    #[test]
    fn harness_request_is_read_only_and_scoped_to_the_materialized_checkout() {
        let run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_123".into(),
            repository: "repo".into(),
            target_sha: "a".repeat(40),
            resolved_from_head: false,
            mode: ScanModeV1::Scan,
            model: None,
            provider: None,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Materialized,
            attempt: 1,
            step: 1,
            step_failures: 0,
            materialized: None,
            harness: None,
            report: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
        };
        let plan = crate::build_analysis_plan(
            &run,
            "/isolated/repo",
            &AnalysisConfigV1 {
                model: "model".into(),
                provider: None,
                max_turns: 4,
                max_output_tokens: 8_000,
                max_total_tokens: 50_000,
                max_cost_usd: Some(2.0),
            },
        );
        let request = harness_request(&plan);
        assert_eq!(
            request["session"]["metadata"],
            json!({
                "security_scan": true,
                "security_scan_run_id": "sec_123",
            })
        );
        assert_eq!(
            request["options"]["metadata"]["fs_scope"]["root"],
            "/isolated/repo"
        );
        assert!(plan.unattended);
        // The harness dropped the ask/agent `mode` option; the request must
        // not carry it any more.
        assert!(request["options"].get("mode").is_none());
        assert_eq!(request["options"]["output"]["type"], "json");
        assert!(request.get("permission_mode").is_none());
        let allow = request["options"]["functions"]["allow"]
            .as_array()
            .expect("allow array");
        assert!(allow
            .iter()
            .all(|value| !value.as_str().unwrap_or_default().contains("shell")));
        assert!(allow
            .iter()
            .all(|value| !value.as_str().unwrap_or_default().contains("create-file")));
        let deny = request["options"]["functions"]["deny"]
            .as_array()
            .expect("deny array");
        assert!(deny.iter().any(|value| value == "github::*"));
        assert!(deny.iter().any(|value| value == "shell::*"));
        assert_eq!(request["options"]["system_prompt_strategy"], "override");
    }

    #[test]
    fn private_state_list_parser_accepts_supported_worker_shapes() {
        let record = json!({
            "schema_version": "1",
            "run_id": "sec_x",
            "repository": "repo",
            "target_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "mode": "scan",
            "operation_nonce": "private_nonce",
            "status": "queued",
            "attempt": 1,
            "step": 0,
            "created_at": 1,
            "updated_at": 1
        });
        assert_eq!(
            parse_state_list::<RunRecordV1>(&json!([record.clone()]), "run")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunRecordV1>(&json!({ "values": [record.clone()] }), "run")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunRecordV1>(&json!({ "sec_x": record }), "run")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunRecordV1>(
                &json!([Value::Null, record.clone(), Value::Null]),
                "run"
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn state_deletion_uses_null_tombstones_that_can_be_listed_and_recreated() {
        let existing = serde_json::to_value(private_run(RunStatusV1::Completed)).unwrap();
        let delete = state_compare_and_set_payload(
            RUN_SCOPE,
            "sec_history",
            Some(existing.clone()),
            None,
        );
        assert_eq!(STATE_CAS_ID, "security-scan::state::compare-and-set");
        assert_eq!(delete["expected"], existing);
        assert!(delete["value"].is_null());
        assert!(parse_optional_run(Value::Null, "sec_history")
            .unwrap()
            .is_none());
        assert!(parse_state_list::<RunRecordV1>(
            &json!({ "sec_history": null }),
            "private run"
        )
        .unwrap()
        .is_empty());

        let replacement = serde_json::to_value(private_run(RunStatusV1::Queued)).unwrap();
        let recreate =
            state_compare_and_set_payload(RUN_SCOPE, "sec_history", None, Some(replacement.clone()));
        assert!(recreate.get("expected").is_none());
        assert_eq!(recreate["value"], replacement);
    }

    #[test]
    fn top_level_history_list_and_parse_failures_keep_backfill_retry_pending() {
        let pending = AtomicBool::new(true);
        let list_failure: Result<(), SecurityScanError> = Err(SecurityScanError::Dependency(
            "private state list temporarily unavailable".into(),
        ));
        mark_backfill_complete(&pending, &list_failure);
        assert!(pending.load(Ordering::Acquire));

        let parse_failure =
            parse_state_list::<RunRecordV1>(&json!({ "values": [{ "invalid": true }] }), "run");
        mark_backfill_complete(&pending, &parse_failure);
        assert!(pending.load(Ordering::Acquire));

        let successful_parse = parse_state_list::<RunRecordV1>(&Value::Null, "run");
        mark_backfill_complete(&pending, &successful_parse);
        assert!(!pending.load(Ordering::Acquire));
    }

    #[test]
    fn run_index_backfills_previous_results_without_copying_full_reports() {
        let mut run = private_run(RunStatusV1::Completed);
        run.completed_at = Some(2);
        run.report = Some(SecurityReportV1 {
            summary: "One actionable finding".into(),
            assessments: crate::SecurityAssessmentsV1::default(),
            findings: vec![SecurityFindingV1 {
                rule_id: "SEC-001".into(),
                severity: SeverityV1::High,
                title: "Unsafe default".into(),
                description: "Details".into(),
                evidence: "Evidence".into(),
                location: None,
                remediation: "Fix it".into(),
                suggested_patch: Some("large patch contents".into()),
            }],
        });

        let index = RunIndexRecordV1::from(&run);
        let encoded = serde_json::to_value(&index).unwrap();

        assert_eq!(index.summary.finding_count, 1);
        assert_eq!(index.summary.status, RunStatusV1::Completed);
        assert_eq!(index.harness_session_id.as_deref(), Some("session_private"));
        assert!(index.has_materialized);
        let encoded = encoded.to_string();
        for private in [
            "private_nonce",
            "wt_private",
            "/private/checkout",
            "turn_private",
            "large patch contents",
            "One actionable finding",
        ] {
            assert!(!encoded.contains(private), "history index copied {private}");
        }
    }

    #[test]
    fn run_index_projection_tracks_authoritative_lifecycle_updates() {
        let queued = private_run(RunStatusV1::Queued);
        let queued_index = RunIndexRecordV1::from(&queued);
        assert_eq!(queued_index.summary.status, RunStatusV1::Queued);

        let mut completed = queued;
        completed.status = RunStatusV1::Completed;
        completed.materialized = None;
        completed.harness = None;
        completed.updated_at = 3;
        completed.completed_at = Some(3);
        completed.report = Some(SecurityReportV1 {
            summary: "No findings returned".into(),
            assessments: crate::SecurityAssessmentsV1::default(),
            findings: Vec::new(),
        });
        let completed_index = RunIndexRecordV1::from(&completed);

        assert_eq!(completed_index.summary.status, RunStatusV1::Completed);
        assert_eq!(completed_index.summary.finding_count, 0);
        assert_eq!(completed_index.summary.updated_at, 3);
        assert!(!completed_index.has_materialized);
        assert!(completed_index.harness_session_id.is_none());
        assert_ne!(queued_index, completed_index);
    }

    #[test]
    fn recovery_index_selects_only_active_or_dirty_terminal_runs() {
        let analyzing = RunIndexRecordV1::from(&private_run(RunStatusV1::Analyzing));
        let dirty_terminal = RunIndexRecordV1::from(&private_run(RunStatusV1::Failed));
        let mut clean_terminal = private_run(RunStatusV1::Completed);
        clean_terminal.materialized = None;
        let clean_terminal = RunIndexRecordV1::from(&clean_terminal);
        let queued = RunIndexRecordV1::from(&private_run(RunStatusV1::Queued));

        assert!(needs_full_reconciliation(&analyzing));
        assert!(needs_full_reconciliation(&dirty_terminal));
        assert!(!needs_full_reconciliation(&clean_terminal));
        assert!(!needs_full_reconciliation(&queued));
        assert!(is_queueable(queued.summary.status));
        assert!(!is_queueable(clean_terminal.summary.status));
    }

    #[test]
    fn run_index_parser_accepts_durable_state_list_shapes() {
        let index =
            serde_json::to_value(RunIndexRecordV1::from(&private_run(RunStatusV1::Analyzing)))
                .unwrap();
        assert_eq!(
            parse_state_list::<RunIndexRecordV1>(&json!([index.clone()]), "run index")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunIndexRecordV1>(&json!({ "sec_history": index }), "run index")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn harness_status_reconciliation_ignores_running_and_recovers_terminal_results() {
        let harness = crate::HarnessRunV1 {
            session_id: "s1".into(),
            turn_id: "t1".into(),
        };
        assert!(completion_event(
            HarnessStatusWire {
                turn_id: Some("t1".into()),
                status: "running".into(),
                expects_wake: false,
                result: None,
                result_error: None,
            },
            &harness,
        )
        .unwrap()
        .is_none());

        let completed = completion_event(
            HarnessStatusWire {
                turn_id: Some("t1".into()),
                status: "completed".into(),
                expects_wake: false,
                result: Some(json!({ "summary": "ok", "findings": [] })),
                result_error: None,
            },
            &harness,
        )
        .unwrap()
        .expect("terminal event");
        assert!(completed.terminal);
        assert_eq!(completed.status, "completed");
    }

    #[test]
    fn missing_worktree_record_is_an_idempotent_cleanup_success() {
        assert!(worktree_is_missing(&SecurityScanError::Dependency(
            "worktree::remove failed: W200 no record".into()
        )));
        assert!(!worktree_is_missing(&SecurityScanError::Dependency(
            "worktree::remove failed: W300 state unavailable".into()
        )));
    }

    #[test]
    fn materialization_identity_is_attempt_scoped() {
        let mut run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_retry".into(),
            repository: "repo".into(),
            target_sha: "a".repeat(40),
            resolved_from_head: false,
            mode: ScanModeV1::Scan,
            model: None,
            provider: None,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Queued,
            attempt: 2,
            step: 0,
            step_failures: 0,
            materialized: None,
            harness: None,
            report: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
        };
        assert_eq!(
            MaterializationRequest::for_run(&run).session_id,
            "security-scan-worktree-private_nonce-attempt-2"
        );
        run.attempt = 3;
        assert_ne!(
            MaterializationRequest::for_run(&run).session_id,
            "security-scan-worktree-private_nonce-attempt-2"
        );
    }

    #[test]
    fn run_update_doorbell_contains_only_the_public_status_projection() {
        let run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_live".into(),
            repository: "iii-hq/iii".into(),
            target_sha: "a".repeat(40),
            resolved_from_head: false,
            mode: ScanModeV1::Suggest,
            model: None,
            provider: None,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Analyzing,
            attempt: 2,
            step: 2,
            step_failures: 0,
            materialized: Some(MaterializedTargetV1 {
                worktree_id: "wt_private".into(),
                path: "/private/checkout".into(),
                base_sha: "a".repeat(40),
            }),
            harness: Some(crate::HarnessRunV1 {
                session_id: "session_private".into(),
                turn_id: "turn_private".into(),
            }),
            report: None,
            error: None,
            created_at: 1,
            updated_at: 2,
            completed_at: None,
        };

        assert_eq!(
            run_update_payload(&run),
            json!({
                "stream_name": "security-scan:runs",
                "group_id": "all",
                "type": "security-scan:updated",
                "data": {
                    "run_id": "sec_live",
                    "repository": "iii-hq/iii",
                    "status": "analyzing",
                    "attempt": 2,
                    "updated_at": 2,
                    "completed_at": null,
                },
            })
        );
    }

    #[test]
    fn code_alert_for_another_commit_remains_a_repository_snapshot() {
        let target_sha = "a".repeat(40);
        let alert = CodeScanningAlertWire {
            number: 7,
            state: "open".into(),
            rule_id: "rust/sql-injection".into(),
            rule_name: Some("SQL injection".into()),
            rule_description: "Untrusted input reaches a query".into(),
            security_severity: Some("high".into()),
            severity: "error".into(),
            tool_name: "CodeQL".into(),
            commit_sha: Some("b".repeat(40)),
            path: Some("src/main.rs".into()),
            start_line: Some(10),
            end_line: Some(12),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: None,
        };

        let normalized = normalize_code_scanning_alert("iii-hq/iii", &target_sha, alert).unwrap();

        assert_eq!(normalized.scope, ReconciliationScopeV1::RepositorySnapshot);
        assert_eq!(
            normalized.public_url,
            "https://github.com/iii-hq/iii/security/code-scanning/7"
        );
    }

    #[test]
    fn reconciliation_snapshot_and_doorbell_exclude_dependency_diagnostics() {
        let target_sha = "a".repeat(40);
        let response: CodeScanningAlertsResponseWire = serde_json::from_value(json!({
            "repository": "iii-hq/iii",
            "completeness": "complete",
            "availability": "available",
            "collected_count": 1,
            "truncation_reason": null,
            "alerts": [{
                "number": 9,
                "state": "open",
                "rule_id": "rust/sql-injection",
                "rule_name": "SQL injection",
                "rule_description": "Untrusted input reaches a query",
                "security_severity": "high",
                "severity": "error",
                "tool_name": "CodeQL",
                "html_url": "https://internal.invalid/token-secret",
                "commit_sha": target_sha,
                "message": "raw diagnostic token-secret",
                "path": "src/main.rs",
                "start_line": 10,
                "end_line": 12,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": null
            }],
            "latest_analysis": {
                "availability": "available",
                "tool_name": "Trivy",
                "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "git_ref": "refs/heads/main",
                "created_at": "2026-01-02T00:00:00Z",
                "error": "configuration failed token-secret",
                "warning": null
            }
        }))
        .unwrap();
        let collection =
            normalize_code_scanning_response("iii-hq/iii", &"a".repeat(40), 100, response).unwrap();
        assert_eq!(
            collection.summary.health.status,
            ReconciliationHealthStatusV1::Error
        );
        let snapshot = ReconciliationSnapshotV1 {
            schema_version: "1".into(),
            run_id: "sec_live".into(),
            repository: "iii".into(),
            target_sha: "a".repeat(40),
            harness: crate::HarnessReconciliationSummaryV1 {
                status: crate::HarnessReconciliationStatusV1::Verified,
                verified_count: Some(3),
                verified_at: Some(90),
                scope: ReconciliationScopeV1::ExactCommit,
            },
            github_repository: Some("iii-hq/iii".into()),
            sources: vec![collection.summary],
            matching: crate::ReconciliationMatchingV1 {
                status: crate::ReconciliationMatchingStatusV1::Unavailable,
                matched_records: None,
            },
            records: collection.records,
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("internal.invalid"));
        assert!(!encoded.contains("raw diagnostic"));
        assert!(!encoded.contains("configuration failed"));
        assert!(!encoded.contains("token-secret"));

        let mut older = snapshot.clone();
        older.sources[0].collected_at = Some(99);
        assert!(snapshot_is_newer(&snapshot, &older));
        let mut newer = snapshot.clone();
        newer.sources[0].collected_at = Some(101);
        assert!(!snapshot_is_newer(&snapshot, &newer));

        let payload = reconciliation_update_payload("sec_live");
        assert_eq!(
            payload,
            json!({
                "stream_name": "security-scan:runs",
                "group_id": "all",
                "type": "security-scan:reconciliation-updated",
                "data": { "run_id": "sec_live" },
            })
        );
        assert!(serde_json::to_string(&payload).unwrap().len() < 256);
    }

    #[test]
    fn github_alert_requests_use_the_existing_read_only_api_contract() {
        let request = serde_json::to_value(dependabot_api_request("iii-hq/iii").unwrap()).unwrap();
        assert_eq!(request["path"], "repos/iii-hq/iii/dependabot/alerts");
        assert_eq!(request["method"], "GET");
        assert_eq!(request["fields"]["state"], "open");
        assert_eq!(request["fields"]["per_page"], "100");
        assert_eq!(request["paginate"], true);
        assert_eq!(request["timeout_ms"], RPC_TIMEOUT_MS);
    }

    #[test]
    fn github_api_pages_are_flattened_bounded_and_safely_classified() {
        let pages = format!(
            "{}\n{}",
            json!([dependabot_api_alert(1)]),
            json!([dependabot_api_alert(2)])
        );
        let response = dependabot_api_response(
            "iii-hq/iii",
            Ok(GithubApiResponseWire {
                value: Value::String(pages),
            }),
        );
        assert_eq!(response.completeness, GithubCompletenessWire::Complete);
        assert_eq!(response.collected_count, 2);

        let response = dependabot_api_response(
            "iii-hq/iii",
            Ok(GithubApiResponseWire {
                value: Value::Array(
                    (0..=GITHUB_ALERT_LIMIT as u64)
                        .map(dependabot_api_alert)
                        .collect(),
                ),
            }),
        );
        assert_eq!(response.completeness, GithubCompletenessWire::Partial);
        assert_eq!(response.collected_count, GITHUB_ALERT_LIMIT);

        let response = dependabot_api_response(
            "iii-hq/iii",
            Err(SecurityScanError::Dependency(
                "github::api failed: HTTP 401 bad credentials token-secret".into(),
            )),
        );
        assert_eq!(
            response.availability,
            GithubAvailabilityWire::AuthenticationRequired
        );
        assert!(response.alerts.is_empty());
    }
}
