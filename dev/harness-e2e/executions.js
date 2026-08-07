window.HARNESS_EXECUTIONS = {
  "executions": [
    {
      "actor": "ytallo",
      "attempt": 2,
      "availability": "unavailable",
      "completed_at": "2026-08-07T10:00:57Z",
      "conclusion": "failure",
      "detail_path": null,
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 2,
        "completed_at": "2026-08-07T10:00:57Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "abfadf473ecb33452372b9fa06f083639b6ba19a",
        "id": "31153706659-2",
        "repository": "iii-hq/workers",
        "run_id": "31153706659",
        "started_at": "2026-08-07T09:57:30Z",
        "workflow_name": "Test · harness_registry · schedule",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31153706659"
      },
      "first_failure": {
        "job_name": "e2e / harness e2e build",
        "kind": "job",
        "message": "e2e / harness e2e build: Test E2E crate",
        "step_name": "Test E2E crate",
        "url": "https://github.com/iii-hq/workers/actions/runs/31153706659/job/92788543989"
      },
      "generated_at": "",
      "id": "31153706659-2",
      "lane": "daily",
      "release": {},
      "requested_runs": null,
      "run_id": "31153706659",
      "source": {
        "ref": "main",
        "repository": "iii-hq/workers",
        "sha": "abfadf473ecb33452372b9fa06f083639b6ba19a"
      },
      "started_at": "2026-08-07T09:57:30Z",
      "status": "infra_failed",
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
      "workflow_duration_seconds": 207.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31153706659"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "unavailable",
      "completed_at": "2026-08-07T06:25:08Z",
      "conclusion": "failure",
      "detail_path": null,
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-07T06:25:08Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "abfadf473ecb33452372b9fa06f083639b6ba19a",
        "id": "31153706659-1",
        "repository": "iii-hq/workers",
        "run_id": "31153706659",
        "started_at": "2026-08-07T06:22:24Z",
        "workflow_name": "Test · harness_registry · schedule",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31153706659"
      },
      "first_failure": {
        "job_name": "e2e / harness e2e build",
        "kind": "job",
        "message": "e2e / harness e2e build: Test E2E crate",
        "step_name": "Test E2E crate",
        "url": "https://github.com/iii-hq/workers/actions/runs/31153706659/job/92788543989"
      },
      "generated_at": "",
      "id": "31153706659-1",
      "lane": "daily",
      "release": {},
      "requested_runs": null,
      "run_id": "31153706659",
      "source": {
        "ref": "main",
        "repository": "iii-hq/workers",
        "sha": "abfadf473ecb33452372b9fa06f083639b6ba19a"
      },
      "started_at": "2026-08-07T06:22:24Z",
      "status": "infra_failed",
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
      "workflow_duration_seconds": 164.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31153706659"
    },
    {
      "actor": "ytallo",
      "attempt": 2,
      "availability": "full",
      "completed_at": "2026-08-07T02:02:58Z",
      "conclusion": "failure",
      "detail_path": "runs/31133924989-2.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 2,
        "completed_at": "2026-08-07T02:02:58Z",
        "conclusion": "failure",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "8c0c1ae621c439a49c097b073a85c1b13f5cc2e8",
        "id": "31133924989-2",
        "repository": "iii-hq/workers",
        "run_id": "31133924989",
        "started_at": "2026-08-07T01:39:46Z",
        "workflow_name": "Test · harness_source · workflow_dispatch",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31133924989"
      },
      "first_failure": {
        "id": "single_database_completion_wake",
        "kind": "hard_gate",
        "message": "watch_before_spawn=false, wake_records=1, notifications=1, no_polling=true, root_did_not_signal=true",
        "scenario_id": "receiving_operation",
        "subject_id": "deepseek-v4-flash"
      },
      "generated_at": "2026-08-07T02:02:21.203199+00:00",
      "id": "31133924989-2",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-07",
        "url": "https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7",
        "version": "2026-08-07",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "31133924989",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0011100749333333333,
            "duration_seconds": 9.994666666666665,
            "function_call_errors": 0.0,
            "function_calls": 2.0,
            "sessions": 1.0,
            "tokens": 6714.0,
            "turns": 4.666666666666667
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
            "cost_usd": 0.0054768792,
            "duration_seconds": 59.984,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5558.333333333333,
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
            "cost_usd": 0.0015038192,
            "duration_seconds": 5.0776666666666666,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 1840.6666666666667,
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
            "cost_usd": 0.004666285866666666,
            "duration_seconds": 69.18233333333333,
            "function_call_errors": 0.0,
            "function_calls": 17.666666666666668,
            "sessions": 1.0,
            "tokens": 23342.333333333332,
            "turns": 10.333333333333334
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
            "cost_usd": 0.008293570133333332,
            "duration_seconds": 107.09066666666666,
            "function_call_errors": 0.0,
            "function_calls": 26.333333333333332,
            "sessions": 3.0,
            "tokens": 41524.333333333336,
            "turns": 19.666666666666668
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
            "cost_usd": 0.0008332352000000001,
            "duration_seconds": 7.334666666666667,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 4930.0,
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
            "cost_usd": 0.014124620533333335,
            "duration_seconds": 195.42666666666665,
            "function_call_errors": 0.0,
            "function_calls": 65.0,
            "sessions": 5.0,
            "tokens": 65490.666666666664,
            "turns": 36.666666666666664
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
            "cost_usd": 0.021275951200000004,
            "duration_seconds": 433.526,
            "function_call_errors": 3.3333333333333335,
            "function_calls": 43.333333333333336,
            "sessions": 4.0,
            "tokens": 90767.0,
            "turns": 27.666666666666668
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
            "cost_usd": 0.01603615626666667,
            "duration_seconds": 211.81833333333336,
            "function_call_errors": 0.0,
            "function_calls": 21.333333333333332,
            "sessions": 3.0,
            "tokens": 81390.33333333333,
            "turns": 15.333333333333334
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
            "cost_usd": 0.0030116992,
            "duration_seconds": 14.104,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2718.3333333333335,
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
            "cost_usd": 0.003645085866666667,
            "duration_seconds": 18.584666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 3304.6666666666665,
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
            "cost_usd": 0.008982952533333333,
            "duration_seconds": 90.74,
            "function_call_errors": 0.0,
            "function_calls": 23.666666666666668,
            "sessions": 1.0,
            "tokens": 46127.0,
            "turns": 19.666666666666668
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
            "cost_usd": 0.0051507344,
            "duration_seconds": 64.41566666666667,
            "function_call_errors": 0.0,
            "function_calls": 17.333333333333332,
            "sessions": 2.0,
            "tokens": 27387.666666666668,
            "turns": 14.0
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
            "cost_usd": 0.0060450376,
            "duration_seconds": 181.77700000000002,
            "function_call_errors": 0.0,
            "function_calls": 19.0,
            "sessions": 2.0,
            "tokens": 30278.666666666668,
            "turns": 16.666666666666668
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
            "cost_usd": 0.002313104266666667,
            "duration_seconds": 40.99433333333334,
            "function_call_errors": 0.0,
            "function_calls": 7.0,
            "sessions": 1.0,
            "tokens": 11925.0,
            "turns": 7.0
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
            "cost_usd": 0.004530396266666667,
            "duration_seconds": 58.185,
            "function_call_errors": 0.0,
            "function_calls": 13.666666666666666,
            "sessions": 1.0,
            "tokens": 23445.666666666668,
            "turns": 10.666666666666666
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
            "cost_usd": 0.0033647376000000002,
            "duration_seconds": 65.53166666666667,
            "function_call_errors": 0.0,
            "function_calls": 11.333333333333334,
            "sessions": 1.0,
            "tokens": 15815.333333333334,
            "turns": 9.0
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
            "cost_usd": 0.002745997333333333,
            "duration_seconds": 40.278666666666666,
            "function_call_errors": 1.0,
            "function_calls": 6.666666666666667,
            "sessions": 1.0,
            "tokens": 13832.333333333334,
            "turns": 7.0
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
            "cost_usd": 0.002789598933333333,
            "duration_seconds": 38.92366666666667,
            "function_call_errors": 0.0,
            "function_calls": 10.666666666666666,
            "sessions": 1.0,
            "tokens": 14698.0,
            "turns": 10.0
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
        "ref": "daily/2026-08-07",
        "repository": "iii-hq/workers",
        "sha": "450995a8672a43416c09c73c193ab2b035dc8dd7"
      },
      "started_at": "2026-08-07T01:39:46Z",
      "status": "hard_gate_failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 19,
          "hard_gate_failures": 3,
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
              "total_cost_usd": 0.0045114576,
              "wall_time_seconds": 15.233
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
              "total_cost_usd": 0.0024997056000000003,
              "wall_time_seconds": 22.004
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
              "total_cost_usd": 0.0090350976,
              "wall_time_seconds": 42.312
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
              "total_cost_usd": 0.04237386160000001,
              "wall_time_seconds": 586.28
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0269488576,
              "wall_time_seconds": 272.22
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
              "total_cost_usd": 0.0164306376,
              "wall_time_seconds": 179.952
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
              "total_cost_usd": 0.010935257600000001,
              "wall_time_seconds": 55.754
            },
            {
              "hard_gate_failures": 0,
              "id": "research_pipeline",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.048108468800000005,
              "wall_time_seconds": 635.455
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
              "total_cost_usd": 0.0139988576,
              "wall_time_seconds": 207.547
            },
            {
              "hard_gate_failures": 0,
              "id": "timer_wake",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.006939312800000002,
              "wall_time_seconds": 122.983
            },
            {
              "hard_gate_failures": 2,
              "id": "receiving_operation",
              "median_score": 75.0,
              "pass_rate": 0.3333333333333333,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.06382785360000001,
              "wall_time_seconds": 1300.578
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
              "total_cost_usd": 0.0100942128,
              "wall_time_seconds": 196.595
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
              "total_cost_usd": 0.0154522032,
              "wall_time_seconds": 193.247
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
              "total_cost_usd": 0.0248807104,
              "wall_time_seconds": 321.272
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
              "total_cost_usd": 0.0181351128,
              "wall_time_seconds": 545.331
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
              "total_cost_usd": 0.0033302248000000004,
              "wall_time_seconds": 29.984
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
              "total_cost_usd": 0.008368796799999998,
              "wall_time_seconds": 116.771
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
              "total_cost_usd": 0.008237992,
              "wall_time_seconds": 120.836
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
              "total_cost_usd": 0.0135911888,
              "wall_time_seconds": 174.555
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 0.3476998096,
          "wall_time_seconds": 5138.909
        }
      ],
      "totals": {
        "average_score": 98.15789473684211,
        "expected_reports": 19,
        "function_calls": 864.0,
        "hard_gate_failures": 3,
        "missing_reports": 0,
        "passed_scenarios": 18,
        "received_reports": 19,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 94.73684210526315,
        "technical_failures": 0,
        "total_cost_usd": 0.3476998096,
        "total_tokens": 1533271.0,
        "wall_time_seconds": 5138.909
      },
      "workflow_duration_seconds": 1392.0,
      "workflow_name": "Test · harness_source · workflow_dispatch",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31133924989"
    },
    {
      "actor": "iii-release-control-dev[bot]",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-07T02:01:13Z",
      "conclusion": "failure",
      "detail_path": "runs/31134926293-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "iii-release-control-dev[bot]",
        "attempt": 1,
        "completed_at": "2026-08-07T02:01:13Z",
        "conclusion": "failure",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "8c0c1ae621c439a49c097b073a85c1b13f5cc2e8",
        "id": "31134926293-1",
        "repository": "iii-hq/workers",
        "run_id": "31134926293",
        "started_at": "2026-08-07T00:31:07Z",
        "workflow_name": "Test · harness_source · f78ebe5e-3366-4848-a562-ba3af2401b0f",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31134926293"
      },
      "first_failure": {
        "kind": "technical",
        "message": "scenario subagent_validation made no observable progress for 420s while waiting for session e2e_60055530f850461b8c678b20765de906 (last active turn t_d440d14359a1444ebc8a87f9d50d9715)",
        "phase": "execute",
        "scenario_id": "subagent_validation",
        "subject_id": "deepseek-v4-flash"
      },
      "generated_at": "2026-08-07T02:00:39.284301+00:00",
      "id": "31134926293-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-07",
        "url": "https://github.com/iii-hq/workers/commit/8c0c1ae621c439a49c097b073a85c1b13f5cc2e8",
        "version": "2026-08-07",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "31134926293",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0019272381333333335,
            "duration_seconds": 18.92866666666667,
            "function_call_errors": 0.0,
            "function_calls": 3.6666666666666665,
            "sessions": 1.0,
            "tokens": 11433.0,
            "turns": 5.333333333333333
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
            "cost_usd": 0.003978012533333333,
            "duration_seconds": 52.14366666666666,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5307.666666666667,
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
            "cost_usd": 0.0015190658666666666,
            "duration_seconds": 4.943666666666666,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 1834.3333333333333,
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
            "cost_usd": 0.003683967466666667,
            "duration_seconds": 55.053666666666665,
            "function_call_errors": 0.0,
            "function_calls": 13.0,
            "sessions": 1.0,
            "tokens": 18543.333333333332,
            "turns": 9.0
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
            "cost_usd": 0.007792297333333334,
            "duration_seconds": 95.99900000000001,
            "function_call_errors": 0.0,
            "function_calls": 24.0,
            "sessions": 3.0,
            "tokens": 39601.333333333336,
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
            "cost_usd": 0.0009353698666666668,
            "duration_seconds": 7.882000000000001,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 5624.0,
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
            "cost_usd": 0.021812629066666662,
            "duration_seconds": 333.706,
            "function_call_errors": 0.6666666666666666,
            "function_calls": 65.66666666666667,
            "sessions": 5.0,
            "tokens": 99377.0,
            "turns": 39.666666666666664
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
            "cost_usd": 0.024343269066666662,
            "duration_seconds": 517.0003333333334,
            "function_call_errors": 5.0,
            "function_calls": 59.333333333333336,
            "sessions": 4.0,
            "tokens": 102114.66666666667,
            "turns": 33.666666666666664
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
            "cost_usd": 0.013992759200000001,
            "duration_seconds": 214.29600000000002,
            "function_call_errors": 0.6666666666666666,
            "function_calls": 26.666666666666668,
            "sessions": 3.0,
            "tokens": 67650.0,
            "turns": 19.0
          },
          "contract_fingerprint": "fnv1a32:642af481",
          "run_count": 3,
          "samples": {
            "cost_usd": 2,
            "duration_seconds": 3,
            "function_call_errors": 3,
            "function_calls": 3,
            "sessions": 3,
            "tokens": 2,
            "turns": 3
          },
          "scenario_id": "research_pipeline",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0029459325333333337,
            "duration_seconds": 14.438666666666668,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2594.0,
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
            "cost_usd": 0.0036825658666666664,
            "duration_seconds": 17.923333333333336,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 3262.6666666666665,
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
            "cost_usd": 0.009413876266666668,
            "duration_seconds": 91.88499999999999,
            "function_call_errors": 0.0,
            "function_calls": 24.0,
            "sessions": 1.0,
            "tokens": 48350.333333333336,
            "turns": 19.333333333333332
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
            "cost_usd": 0.0041243906666666675,
            "duration_seconds": 190.745,
            "function_call_errors": 0.0,
            "function_calls": 15.0,
            "sessions": 2.0,
            "tokens": 21393.666666666668,
            "turns": 15.0
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
            "cost_usd": 0.005337352533333333,
            "duration_seconds": 181.812,
            "function_call_errors": 0.0,
            "function_calls": 17.0,
            "sessions": 2.0,
            "tokens": 27225.0,
            "turns": 17.0
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
            "cost_usd": 0.0026477378666666667,
            "duration_seconds": 39.663666666666664,
            "function_call_errors": 0.0,
            "function_calls": 8.0,
            "sessions": 1.0,
            "tokens": 13918.0,
            "turns": 8.0
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
            "cost_usd": 0.003835678933333334,
            "duration_seconds": 54.690333333333335,
            "function_call_errors": 0.0,
            "function_calls": 16.666666666666668,
            "sessions": 1.0,
            "tokens": 18355.666666666668,
            "turns": 13.333333333333334
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
            "cost_usd": 0.003955128800000001,
            "duration_seconds": 60.64166666666667,
            "function_call_errors": 0.0,
            "function_calls": 9.0,
            "sessions": 1.0,
            "tokens": 19941.666666666668,
            "turns": 9.333333333333334
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
            "cost_usd": 0.0030213586666666668,
            "duration_seconds": 46.25133333333333,
            "function_call_errors": 1.0,
            "function_calls": 6.333333333333333,
            "sessions": 1.0,
            "tokens": 15464.0,
            "turns": 5.666666666666667
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
            "cost_usd": 0.002800662666666667,
            "duration_seconds": 34.97466666666667,
            "function_call_errors": 0.0,
            "function_calls": 8.666666666666666,
            "sessions": 1.0,
            "tokens": 15569.0,
            "turns": 8.0
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
        "ref": "daily/2026-08-07",
        "repository": "iii-hq/workers",
        "sha": "8c0c1ae621c439a49c097b073a85c1b13f5cc2e8"
      },
      "started_at": "2026-08-07T00:31:07Z",
      "status": "technical_failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 19,
          "hard_gate_failures": 5,
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
              "total_cost_usd": 0.0045571976,
              "wall_time_seconds": 14.831
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
              "total_cost_usd": 0.0028061096000000004,
              "wall_time_seconds": 23.646
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
              "total_cost_usd": 0.008837797599999999,
              "wall_time_seconds": 43.316
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.06543788719999999,
              "wall_time_seconds": 1001.118
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0282416288,
              "wall_time_seconds": 275.655
            },
            {
              "hard_gate_failures": 1,
              "id": "design_tradeoff",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.011934037599999999,
              "wall_time_seconds": 156.431
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
              "total_cost_usd": 0.0110476976,
              "wall_time_seconds": 53.77
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
              "total_cost_usd": null,
              "wall_time_seconds": 642.888
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
              "total_cost_usd": 0.0110519024,
              "wall_time_seconds": 165.161
            },
            {
              "hard_gate_failures": 0,
              "id": "timer_wake",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0079432136,
              "wall_time_seconds": 118.991
            },
            {
              "hard_gate_failures": 1,
              "id": "receiving_operation",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.07302980719999999,
              "wall_time_seconds": 1551.001
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
              "total_cost_usd": 0.011865386400000004,
              "wall_time_seconds": 181.925
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "failed",
              "technical_failures": 1,
              "threshold": 90.0,
              "total_cost_usd": 0.012373172000000002,
              "wall_time_seconds": 572.235
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
              "total_cost_usd": 0.023376892000000003,
              "wall_time_seconds": 287.997
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
              "total_cost_usd": 0.0160120576,
              "wall_time_seconds": 545.436
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
              "total_cost_usd": 0.0057817144000000004,
              "wall_time_seconds": 56.786
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
              "total_cost_usd": 0.008401988,
              "wall_time_seconds": 104.924
            },
            {
              "hard_gate_failures": 1,
              "id": "validation_scope_enforcement",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.009064076,
              "wall_time_seconds": 138.754
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
              "total_cost_usd": 0.011507036800000002,
              "wall_time_seconds": 164.071
            }
          ],
          "technical_failures": 1,
          "total_cost_usd": null,
          "wall_time_seconds": 6098.936
        }
      ],
      "totals": {
        "average_score": 99.47368421052632,
        "expected_reports": 19,
        "function_calls": 900.0,
        "hard_gate_failures": 5,
        "missing_reports": 0,
        "passed_scenarios": 18,
        "received_reports": 19,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 94.73684210526315,
        "technical_failures": 1,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": 6098.936
      },
      "workflow_duration_seconds": 5406.0,
      "workflow_name": "Test · harness_source · f78ebe5e-3366-4848-a562-ba3af2401b0f",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31134926293"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-07T01:36:49Z",
      "conclusion": "failure",
      "detail_path": "runs/31133924989-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-07T01:36:49Z",
        "conclusion": "failure",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "8c0c1ae621c439a49c097b073a85c1b13f5cc2e8",
        "id": "31133924989-1",
        "repository": "iii-hq/workers",
        "run_id": "31133924989",
        "started_at": "2026-08-07T00:14:12Z",
        "workflow_name": "Test · harness_source · workflow_dispatch",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31133924989"
      },
      "generated_at": "2026-08-07T01:36:09.722560+00:00",
      "id": "31133924989-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-07",
        "url": "https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7",
        "version": "2026-08-07",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "31133924989",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0011100749333333333,
            "duration_seconds": 9.994666666666665,
            "function_call_errors": 0.0,
            "function_calls": 2.0,
            "sessions": 1.0,
            "tokens": 6714.0,
            "turns": 4.666666666666667
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
            "cost_usd": 0.0054768792,
            "duration_seconds": 59.984,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5558.333333333333,
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
            "cost_usd": 0.0015038192,
            "duration_seconds": 5.0776666666666666,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 1840.6666666666667,
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
            "cost_usd": 0.004666285866666666,
            "duration_seconds": 69.18233333333333,
            "function_call_errors": 0.0,
            "function_calls": 17.666666666666668,
            "sessions": 1.0,
            "tokens": 23342.333333333332,
            "turns": 10.333333333333334
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
            "cost_usd": 0.008293570133333332,
            "duration_seconds": 107.09066666666666,
            "function_call_errors": 0.0,
            "function_calls": 26.333333333333332,
            "sessions": 3.0,
            "tokens": 41524.333333333336,
            "turns": 19.666666666666668
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
            "cost_usd": 0.0008332352000000001,
            "duration_seconds": 7.334666666666667,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 4930.0,
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
            "cost_usd": 0.014124620533333335,
            "duration_seconds": 195.42666666666665,
            "function_call_errors": 0.0,
            "function_calls": 65.0,
            "sessions": 5.0,
            "tokens": 65490.666666666664,
            "turns": 36.666666666666664
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
            "cost_usd": 0.019805271733333338,
            "duration_seconds": 378.7763333333333,
            "function_call_errors": 1.6666666666666667,
            "function_calls": 46.0,
            "sessions": 4.0,
            "tokens": 84809.33333333333,
            "turns": 35.333333333333336
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
            "cost_usd": 0.01603615626666667,
            "duration_seconds": 211.81833333333336,
            "function_call_errors": 0.0,
            "function_calls": 21.333333333333332,
            "sessions": 3.0,
            "tokens": 81390.33333333333,
            "turns": 15.333333333333334
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
            "cost_usd": 0.0030116992,
            "duration_seconds": 14.104,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2718.3333333333335,
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
            "cost_usd": 0.003645085866666667,
            "duration_seconds": 18.584666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 3304.6666666666665,
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
            "cost_usd": 0.008982952533333333,
            "duration_seconds": 90.74,
            "function_call_errors": 0.0,
            "function_calls": 23.666666666666668,
            "sessions": 1.0,
            "tokens": 46127.0,
            "turns": 19.666666666666668
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
            "cost_usd": 0.0051507344,
            "duration_seconds": 64.41566666666667,
            "function_call_errors": 0.0,
            "function_calls": 17.333333333333332,
            "sessions": 2.0,
            "tokens": 27387.666666666668,
            "turns": 14.0
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
            "cost_usd": 0.0060450376,
            "duration_seconds": 181.77700000000002,
            "function_call_errors": 0.0,
            "function_calls": 19.0,
            "sessions": 2.0,
            "tokens": 30278.666666666668,
            "turns": 16.666666666666668
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
            "cost_usd": 0.002313104266666667,
            "duration_seconds": 40.99433333333334,
            "function_call_errors": 0.0,
            "function_calls": 7.0,
            "sessions": 1.0,
            "tokens": 11925.0,
            "turns": 7.0
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
            "cost_usd": 0.004530396266666667,
            "duration_seconds": 58.185,
            "function_call_errors": 0.0,
            "function_calls": 13.666666666666666,
            "sessions": 1.0,
            "tokens": 23445.666666666668,
            "turns": 10.666666666666666
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
            "cost_usd": 0.0033647376000000002,
            "duration_seconds": 65.53166666666667,
            "function_call_errors": 0.0,
            "function_calls": 11.333333333333334,
            "sessions": 1.0,
            "tokens": 15815.333333333334,
            "turns": 9.0
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
            "cost_usd": 0.002745997333333333,
            "duration_seconds": 40.278666666666666,
            "function_call_errors": 1.0,
            "function_calls": 6.666666666666667,
            "sessions": 1.0,
            "tokens": 13832.333333333334,
            "turns": 7.0
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
            "cost_usd": 0.002789598933333333,
            "duration_seconds": 38.92366666666667,
            "function_call_errors": 0.0,
            "function_calls": 10.666666666666666,
            "sessions": 1.0,
            "tokens": 14698.0,
            "turns": 10.0
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
        "ref": "daily/2026-08-07",
        "repository": "iii-hq/workers",
        "sha": "450995a8672a43416c09c73c193ab2b035dc8dd7"
      },
      "started_at": "2026-08-07T00:14:12Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 19,
          "hard_gate_failures": 3,
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
              "total_cost_usd": 0.0045114576,
              "wall_time_seconds": 15.233
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
              "total_cost_usd": 0.0024997056000000003,
              "wall_time_seconds": 22.004
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
              "total_cost_usd": 0.0090350976,
              "wall_time_seconds": 42.312
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
              "total_cost_usd": 0.04237386160000001,
              "wall_time_seconds": 586.28
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0269488576,
              "wall_time_seconds": 272.22
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
              "total_cost_usd": 0.0164306376,
              "wall_time_seconds": 179.952
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
              "total_cost_usd": 0.010935257600000001,
              "wall_time_seconds": 55.754
            },
            {
              "hard_gate_failures": 0,
              "id": "research_pipeline",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.048108468800000005,
              "wall_time_seconds": 635.455
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
              "total_cost_usd": 0.0139988576,
              "wall_time_seconds": 207.547
            },
            {
              "hard_gate_failures": 0,
              "id": "timer_wake",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.006939312800000002,
              "wall_time_seconds": 122.983
            },
            {
              "hard_gate_failures": 2,
              "id": "receiving_operation",
              "median_score": 75.0,
              "pass_rate": 0.3333333333333333,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.05941581520000001,
              "wall_time_seconds": 1136.329
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
              "total_cost_usd": 0.0100942128,
              "wall_time_seconds": 196.595
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
              "total_cost_usd": 0.0154522032,
              "wall_time_seconds": 193.247
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
              "total_cost_usd": 0.0248807104,
              "wall_time_seconds": 321.272
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
              "total_cost_usd": 0.0181351128,
              "wall_time_seconds": 545.331
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
              "total_cost_usd": 0.0033302248000000004,
              "wall_time_seconds": 29.984
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
              "total_cost_usd": 0.008368796799999998,
              "wall_time_seconds": 116.771
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
              "total_cost_usd": 0.008237992,
              "wall_time_seconds": 120.836
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
              "total_cost_usd": 0.0135911888,
              "wall_time_seconds": 174.555
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 0.34328777120000004,
          "wall_time_seconds": 4974.66
        }
      ],
      "totals": {
        "average_score": 98.15789473684211,
        "expected_reports": 19,
        "function_calls": 872.0,
        "hard_gate_failures": 3,
        "missing_reports": 0,
        "passed_scenarios": 18,
        "received_reports": 19,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 94.73684210526315,
        "technical_failures": 0,
        "total_cost_usd": 0.34328777120000004,
        "total_tokens": 1515398.0,
        "wall_time_seconds": 4974.66
      },
      "workflow_duration_seconds": 4957.0,
      "workflow_name": "Test · harness_source · workflow_dispatch",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31133924989"
    },
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
  "last_update": "2026-08-07T10:00:57Z",
  "mode": "published",
  "repo_url": "https://github.com/iii-hq/workers",
  "retention": {
    "details": 30,
    "summaries": 100
  },
  "schema_version": 3
};
