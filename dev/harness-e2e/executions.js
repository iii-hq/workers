window.HARNESS_EXECUTIONS = {
  "executions": [
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "unavailable",
      "completed_at": "2026-08-06T07:17:38Z",
      "conclusion": "failure",
      "detail_path": null,
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-06T07:17:38Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "5b0ffa97a1416b40a189fe43771732597711ddea",
        "id": "31078732959-1",
        "repository": "iii-hq/workers",
        "run_id": "31078732959",
        "started_at": "2026-08-06T06:51:36Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31078732959"
      },
      "generated_at": "",
      "id": "31078732959-1",
      "lane": "daily",
      "release": {},
      "requested_runs": null,
      "run_id": "31078732959",
      "source": {
        "ref": "main",
        "repository": "iii-hq/workers",
        "sha": "5b0ffa97a1416b40a189fe43771732597711ddea"
      },
      "started_at": "2026-08-06T06:51:36Z",
      "status": "incomplete",
      "subjects": [],
      "totals": {
        "average_score": null,
        "expected_reports": 0,
        "hard_gate_failures": 0,
        "missing_reports": 0,
        "passed_scenarios": 0,
        "received_reports": 0,
        "report_coverage": 0,
        "retries": 0,
        "scenario_pass_rate": 0,
        "technical_failures": 0,
        "total_cost_usd": null,
        "wall_time_seconds": null
      },
      "workflow_duration_seconds": 1562.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31078732959"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-05T08:03:58Z",
      "conclusion": "failure",
      "detail_path": "runs/30982733353-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-05T08:03:58Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "a4b7b49463e78b2b588292d416cbbf2e7f23d1d6",
        "id": "30982733353-1",
        "repository": "iii-hq/workers",
        "run_id": "30982733353",
        "started_at": "2026-08-05T06:50:10Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30982733353"
      },
      "generated_at": "2026-08-05T08:03:26.284525+00:00",
      "id": "30982733353-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-05",
        "url": "https://github.com/iii-hq/workers/commit/a4b7b49463e78b2b588292d416cbbf2e7f23d1d6",
        "version": "2026-08-05",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30982733353",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0015545954666666667,
            "duration_seconds": 15.847333333333333,
            "function_call_errors": 0.0,
            "function_calls": 2.3333333333333335,
            "sessions": 1.0,
            "tokens": 8909.0,
            "turns": 5.0
          },
          "contract_fingerprint": "fnv1a32:4ed893e8",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "custom_validator",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0052853192,
            "duration_seconds": 41.495,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5200.666666666667,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:eea232b1",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "design_tradeoff",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0015067658666666668,
            "duration_seconds": 6.8180000000000005,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 1842.0,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:9ddecf33",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "direct_answer",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0043970677333333335,
            "duration_seconds": 52.73866666666667,
            "function_call_errors": 0.0,
            "function_calls": 15.333333333333334,
            "sessions": 1.0,
            "tokens": 22008.0,
            "turns": 10.666666666666666
          },
          "contract_fingerprint": "fnv1a32:8da5e3b5",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "mechanical_reaction",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.007514020266666668,
            "duration_seconds": 82.96633333333334,
            "function_call_errors": 0.0,
            "function_calls": 22.0,
            "sessions": 3.0,
            "tokens": 37855.0,
            "turns": 19.333333333333332
          },
          "contract_fingerprint": "fnv1a32:fad69b9f",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "multi_subagent_validation",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0007640789333333333,
            "duration_seconds": 6.919666666666667,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 4425.0,
            "turns": 4.0
          },
          "contract_fingerprint": "fnv1a32:79be8d47",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "persistent_state",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.015201732000000004,
            "duration_seconds": 185.40633333333335,
            "function_call_errors": 0.6666666666666666,
            "function_calls": 64.66666666666667,
            "sessions": 5.0,
            "tokens": 69473.0,
            "turns": 41.666666666666664
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.023269095733333328,
            "duration_seconds": 384.94966666666664,
            "function_call_errors": 3.6666666666666665,
            "function_calls": 48.333333333333336,
            "sessions": 4.0,
            "tokens": 99774.66666666667,
            "turns": 32.666666666666664
          },
          "contract_fingerprint": "fnv1a32:4ac92d0d",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "receiving_operation",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.016103587733333334,
            "duration_seconds": 198.78599999999997,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 25.666666666666668,
            "sessions": 3.0,
            "tokens": 76607.0,
            "turns": 17.333333333333332
          },
          "contract_fingerprint": "fnv1a32:642af481",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "research_pipeline",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.002993432533333333,
            "duration_seconds": 13.609,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2629.3333333333335,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:b82f2ff2",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "security_review",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0038583592,
            "duration_seconds": 19.174666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 3642.0,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:f9261bda",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "security_triage",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0091387352,
            "duration_seconds": 79.034,
            "function_call_errors": 0.0,
            "function_calls": 24.0,
            "sessions": 1.0,
            "tokens": 47484.0,
            "turns": 17.666666666666668
          },
          "contract_fingerprint": "fnv1a32:1e651e78",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "shell_coder_sandbox",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.004253186933333333,
            "duration_seconds": 44.222,
            "function_call_errors": 0.0,
            "function_calls": 13.333333333333334,
            "sessions": 2.0,
            "tokens": 22378.666666666668,
            "turns": 13.666666666666666
          },
          "contract_fingerprint": "fnv1a32:80ed5cc6",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "subagent_validation",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.004912166933333333,
            "duration_seconds": 182.06199999999998,
            "function_call_errors": 0.0,
            "function_calls": 18.333333333333332,
            "sessions": 2.0,
            "tokens": 25773.0,
            "turns": 16.0
          },
          "contract_fingerprint": "fnv1a32:d518ea13",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "subagent_validation_failure",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.002291824266666667,
            "duration_seconds": 29.063999999999997,
            "function_call_errors": 0.0,
            "function_calls": 7.333333333333333,
            "sessions": 1.0,
            "tokens": 12322.333333333334,
            "turns": 7.666666666666667
          },
          "contract_fingerprint": "fnv1a32:9faf1659",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "timer_wake",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.004341879733333333,
            "duration_seconds": 49.381,
            "function_call_errors": 0.0,
            "function_calls": 16.333333333333332,
            "sessions": 1.0,
            "tokens": 21978.333333333332,
            "turns": 12.0
          },
          "contract_fingerprint": "fnv1a32:c822dbb7",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "validation_chain",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0028593936000000006,
            "duration_seconds": 38.687000000000005,
            "function_call_errors": 0.0,
            "function_calls": 9.666666666666666,
            "sessions": 1.0,
            "tokens": 14128.666666666666,
            "turns": 10.0
          },
          "contract_fingerprint": "fnv1a32:ba1d2272",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "validation_loop",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0014142445333333335,
            "duration_seconds": 21.713000000000005,
            "function_call_errors": 1.0,
            "function_calls": 3.6666666666666665,
            "sessions": 1.0,
            "tokens": 7062.0,
            "turns": 5.0
          },
          "contract_fingerprint": "fnv1a32:3764aff3",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "validation_scope_enforcement",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0029015672000000004,
            "duration_seconds": 34.43033333333333,
            "function_call_errors": 0.0,
            "function_calls": 10.0,
            "sessions": 1.0,
            "tokens": 15351.333333333334,
            "turns": 9.666666666666666
          },
          "contract_fingerprint": "fnv1a32:99550b75",
          "run_count": 3,
          "samples": {
            "cost_usd": 3,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 3,
            "turns": 3
          },
          "scenario_id": "validation_self_repair",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        }
      ],
      "source": {
        "ref": "daily/2026-08-05",
        "repository": "iii-hq/workers",
        "sha": "a4b7b49463e78b2b588292d416cbbf2e7f23d1d6"
      },
      "started_at": "2026-08-05T06:50:10Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 19,
          "hard_gate_failures": 6,
          "id": "deepseek-v4-flash",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "deepseek-v4-flash",
          "passed": false,
          "provider": "deepseek",
          "received_reports": 19,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.9473684210526315,
          "scenarios": [
            {
              "hard_gate_failures": 0,
              "id": "direct_answer",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.004520297600000001,
              "wall_time_seconds": 20.454
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 90.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0022922368,
              "wall_time_seconds": 20.759
            },
            {
              "hard_gate_failures": 0,
              "id": "security_review",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0089802976,
              "wall_time_seconds": 40.827
            },
            {
              "hard_gate_failures": 0,
              "id": "reactive_automation",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.04560519600000002,
              "wall_time_seconds": 556.219
            },
            {
              "hard_gate_failures": 3,
              "id": "shell_coder_sandbox",
              "median_score": 80.0,
              "pass_rate": 0.0,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0274162056,
              "wall_time_seconds": 237.102
            },
            {
              "hard_gate_failures": 0,
              "id": "design_tradeoff",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0158559576,
              "wall_time_seconds": 124.485
            },
            {
              "hard_gate_failures": 0,
              "id": "security_triage",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0115750776,
              "wall_time_seconds": 57.524
            },
            {
              "hard_gate_failures": 1,
              "id": "research_pipeline",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0483107632,
              "wall_time_seconds": 596.358
            },
            {
              "hard_gate_failures": 0,
              "id": "mechanical_reaction",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.013191203200000001,
              "wall_time_seconds": 158.216
            },
            {
              "hard_gate_failures": 1,
              "id": "timer_wake",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.006875472800000001,
              "wall_time_seconds": 87.192
            },
            {
              "hard_gate_failures": 0,
              "id": "receiving_operation",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.06980728719999998,
              "wall_time_seconds": 1154.849
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_loop",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.008578180800000001,
              "wall_time_seconds": 116.061
            },
            {
              "hard_gate_failures": 1,
              "id": "subagent_validation",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0127595608,
              "wall_time_seconds": 132.666
            },
            {
              "hard_gate_failures": 0,
              "id": "multi_subagent_validation",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.022542060800000003,
              "wall_time_seconds": 248.899
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation_failure",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.014736500799999998,
              "wall_time_seconds": 546.186
            },
            {
              "hard_gate_failures": 0,
              "id": "custom_validator",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.004663786400000001,
              "wall_time_seconds": 47.542
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_self_repair",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 80.0,
              "total_cost_usd": 0.008704701600000002,
              "wall_time_seconds": 103.291
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_scope_enforcement",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.004242733600000001,
              "wall_time_seconds": 65.139
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_chain",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0130256392,
              "wall_time_seconds": 148.143
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 0.34368315920000003,
          "wall_time_seconds": 4461.912
        }
      ],
      "totals": {
        "average_score": 98.42105263157895,
        "expected_reports": 19,
        "function_calls": 852.0,
        "hard_gate_failures": 6,
        "missing_reports": 0,
        "passed_scenarios": 18,
        "received_reports": 19,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 94.73684210526315,
        "technical_failures": 0,
        "total_cost_usd": 0.34368315920000003,
        "total_tokens": 1496532.0,
        "wall_time_seconds": 4461.912
      },
      "workflow_duration_seconds": 4428.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30982733353"
    }
  ],
  "last_update": "2026-08-06T07:17:38Z",
  "mode": "published",
  "repo_url": "https://github.com/iii-hq/workers",
  "retention": {
    "details": 30,
    "summaries": 100
  },
  "schema_version": 2
};
