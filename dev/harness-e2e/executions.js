window.HARNESS_EXECUTIONS = {
  "executions": [
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-11T07:04:27Z",
      "conclusion": "success",
      "detail_path": "runs/31464501923-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-11T07:04:27Z",
        "conclusion": "success",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "3b2f20401689d2aa70a25c69bc820f35a9e68976",
        "id": "31464501923-1",
        "repository": "iii-hq/workers",
        "run_id": "31464501923",
        "started_at": "2026-08-11T06:16:47Z",
        "workflow_name": "Test · harness_registry · schedule",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31464501923"
      },
      "first_failure": {
        "id": "source_capture",
        "kind": "hard_gate",
        "message": "armed_before_fetch=true, source_order=false, article_valid=true, exact_write=false",
        "scenario_id": "research_pipeline",
        "subject_id": "deepseek-v4-flash"
      },
      "generated_at": "2026-08-11T07:04:23.231851+00:00",
      "id": "31464501923-1",
      "lane": "daily",
      "release": {
        "registry_tag": "latest",
        "tag": "daily/2026-08-11",
        "url": "https://github.com/iii-hq/workers/commit/3b2f20401689d2aa70a25c69bc820f35a9e68976",
        "version": "1.7.4",
        "worker": "harness"
      },
      "requested_runs": 3,
      "run_id": "31464501923",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0018930277333333335,
            "duration_seconds": 19.44633333333333,
            "function_call_errors": 0.0,
            "function_calls": 4.666666666666667,
            "sessions": 1.0,
            "tokens": 10539.0,
            "turns": 7.0
          },
          "contract_fingerprint": "fnv1a32:0295802c",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0015862992,
            "duration_seconds": 7.878666666666668,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2144.6666666666665,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:e66be2c8",
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
          "scenario_version": 2,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.004179647733333334,
            "duration_seconds": 55.80666666666667,
            "function_call_errors": 0.0,
            "function_calls": 15.666666666666666,
            "sessions": 1.0,
            "tokens": 20174.666666666668,
            "turns": 10.333333333333334
          },
          "contract_fingerprint": "fnv1a32:f95ec173",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.006648303200000001,
            "duration_seconds": 73.74866666666667,
            "function_call_errors": 0.0,
            "function_calls": 20.0,
            "sessions": 3.0,
            "tokens": 31786.5,
            "turns": 19.333333333333332
          },
          "contract_fingerprint": "fnv1a32:4fa46005",
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
          "scenario_id": "multi_subagent_validation",
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0007100258666666667,
            "duration_seconds": 8.073333333333332,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 3996.0,
            "turns": 4.0
          },
          "contract_fingerprint": "fnv1a32:3346e4ed",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.012446418133333336,
            "duration_seconds": 131.53033333333335,
            "function_call_errors": 0.0,
            "function_calls": 63.0,
            "sessions": 5.0,
            "tokens": 59283.0,
            "turns": 36.666666666666664
          },
          "contract_fingerprint": "fnv1a32:645b9d93",
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
          "scenario_version": 4,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.022234969866666662,
            "duration_seconds": 323.532,
            "function_call_errors": 2.3333333333333335,
            "function_calls": 65.0,
            "sessions": 4.0,
            "tokens": 91041.33333333333,
            "turns": 42.0
          },
          "contract_fingerprint": "fnv1a32:ccaa6f74",
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
          "scenario_version": 4,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.016642634400000005,
            "duration_seconds": 193.07633333333334,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 28.666666666666668,
            "sessions": 3.0,
            "tokens": 80773.0,
            "turns": 20.0
          },
          "contract_fingerprint": "fnv1a32:24d21247",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0125540856,
            "duration_seconds": 77.324,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 24.0,
            "sessions": 1.0,
            "tokens": 70567.66666666667,
            "turns": 19.666666666666668
          },
          "contract_fingerprint": "fnv1a32:85829578",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.004074586133333333,
            "duration_seconds": 39.312999999999995,
            "function_call_errors": 0.0,
            "function_calls": 14.0,
            "sessions": 2.0,
            "tokens": 21323.333333333332,
            "turns": 14.0
          },
          "contract_fingerprint": "fnv1a32:6f45dd8a",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.006104268800000001,
            "duration_seconds": 177.41366666666667,
            "function_call_errors": 0.0,
            "function_calls": 18.666666666666668,
            "sessions": 2.0,
            "tokens": 30682.666666666668,
            "turns": 16.333333333333332
          },
          "contract_fingerprint": "fnv1a32:69f50761",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0024487624,
            "duration_seconds": 32.175333333333334,
            "function_call_errors": 0.0,
            "function_calls": 8.333333333333334,
            "sessions": 1.0,
            "tokens": 12621.333333333334,
            "turns": 7.666666666666667
          },
          "contract_fingerprint": "fnv1a32:6ecbf11f",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0039518024,
            "duration_seconds": 46.50966666666667,
            "function_call_errors": 0.0,
            "function_calls": 16.0,
            "sessions": 1.0,
            "tokens": 18889.0,
            "turns": 13.0
          },
          "contract_fingerprint": "fnv1a32:deb3403d",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.003470743733333334,
            "duration_seconds": 43.80466666666666,
            "function_call_errors": 0.0,
            "function_calls": 11.333333333333334,
            "sessions": 1.0,
            "tokens": 16938.0,
            "turns": 9.333333333333334
          },
          "contract_fingerprint": "fnv1a32:addaa57e",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0021203578666666666,
            "duration_seconds": 35.51333333333333,
            "function_call_errors": 1.0,
            "function_calls": 5.666666666666667,
            "sessions": 1.0,
            "tokens": 9769.0,
            "turns": 6.333333333333333
          },
          "contract_fingerprint": "fnv1a32:8a795841",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0032190536000000006,
            "duration_seconds": 38.89633333333333,
            "function_call_errors": 0.0,
            "function_calls": 11.0,
            "sessions": 1.0,
            "tokens": 16769.666666666668,
            "turns": 9.333333333333334
          },
          "contract_fingerprint": "fnv1a32:e41c8856",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        }
      ],
      "source": {
        "ref": "daily/2026-08-11",
        "repository": "iii-hq/workers",
        "sha": "3b2f20401689d2aa70a25c69bc820f35a9e68976"
      },
      "started_at": "2026-08-11T06:16:47Z",
      "status": "hard_gate_failed",
      "subjects": [
        {
          "engine_revision": "0.22.1",
          "expected_reports": 16,
          "hard_gate_failures": 3,
          "id": "deepseek-v4-flash",
          "infra_failures": 0,
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "deepseek-v4-flash",
          "passed": true,
          "provider": "deepseek",
          "received_reports": 16,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 1.0,
          "scenarios": [
            {
              "hard_gate_failures": 0,
              "id": "direct_answer",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0047588976,
              "wall_time_seconds": 23.636
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "infra_failures": 0,
              "median_score": 90.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0021300776,
              "wall_time_seconds": 24.22
            },
            {
              "hard_gate_failures": 0,
              "id": "reactive_automation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.03733925440000001,
              "wall_time_seconds": 394.591
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0376622568,
              "wall_time_seconds": 231.972
            },
            {
              "hard_gate_failures": 1,
              "id": "research_pipeline",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "total_cost_usd": 0.04992790320000001,
              "wall_time_seconds": 579.229
            },
            {
              "hard_gate_failures": 0,
              "id": "mechanical_reaction",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.012538943200000003,
              "wall_time_seconds": 167.42
            },
            {
              "hard_gate_failures": 1,
              "id": "timer_wake",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "total_cost_usd": 0.007346287199999999,
              "wall_time_seconds": 96.526
            },
            {
              "hard_gate_failures": 1,
              "id": "receiving_operation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "total_cost_usd": 0.06670490959999999,
              "wall_time_seconds": 970.596
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_loop",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.010412231200000002,
              "wall_time_seconds": 131.414
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0122237584,
              "wall_time_seconds": 117.939
            },
            {
              "hard_gate_failures": 0,
              "id": "multi_subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": null,
              "wall_time_seconds": 221.246
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation_failure",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.018312806400000003,
              "wall_time_seconds": 532.241
            },
            {
              "hard_gate_failures": 0,
              "id": "custom_validator",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.005679083200000001,
              "wall_time_seconds": 58.339
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_self_repair",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.009657160800000002,
              "wall_time_seconds": 116.689
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_scope_enforcement",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.006361073599999999,
              "wall_time_seconds": 106.54
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_chain",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.011855407200000001,
              "wall_time_seconds": 139.529
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": 3912.127
        }
      ],
      "totals": {
        "average_score": 99.375,
        "expected_reports": 16,
        "function_calls": 927.0,
        "hard_gate_failures": 3,
        "missing_reports": 0,
        "passed_scenarios": 16,
        "received_reports": 16,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 100.0,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": 3912.127
      },
      "workflow_duration_seconds": 2860.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31464501923"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-10T07:06:13Z",
      "conclusion": "failure",
      "detail_path": "runs/31361902698-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-10T07:06:13Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "53275f249ced776220b1bb4aca2344cd600d6531",
        "id": "31361902698-1",
        "repository": "iii-hq/workers",
        "run_id": "31361902698",
        "started_at": "2026-08-10T06:25:09Z",
        "workflow_name": "Test · harness_registry · schedule",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31361902698"
      },
      "first_failure": {
        "job_name": "e2e / harness e2e / deepseek-v4-flash / research_pipeline",
        "kind": "job",
        "message": "e2e / harness e2e / deepseek-v4-flash / research_pipeline: Run deployed-stack quality scenario",
        "step_name": "Run deployed-stack quality scenario",
        "url": "https://github.com/iii-hq/workers/actions/runs/31361902698/job/93372924028"
      },
      "generated_at": "2026-08-10T07:06:09.286238+00:00",
      "id": "31361902698-1",
      "lane": "daily",
      "release": {
        "registry_tag": "latest",
        "tag": "daily/2026-08-10",
        "url": "https://github.com/iii-hq/workers/commit/53275f249ced776220b1bb4aca2344cd600d6531",
        "version": "latest",
        "worker": "harness"
      },
      "requested_runs": 3,
      "run_id": "31361902698",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0013921301333333334,
            "duration_seconds": 10.001,
            "function_call_errors": 0.0,
            "function_calls": 2.3333333333333335,
            "sessions": 1.0,
            "tokens": 8461.333333333334,
            "turns": 4.666666666666667
          },
          "contract_fingerprint": "fnv1a32:0295802c",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0015705725333333333,
            "duration_seconds": 9.231333333333334,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2136.3333333333335,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:e66be2c8",
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
          "scenario_version": 2,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0051826245333333335,
            "duration_seconds": 57.80166666666667,
            "function_call_errors": 0.0,
            "function_calls": 13.0,
            "sessions": 1.0,
            "tokens": 26546.0,
            "turns": 9.0
          },
          "contract_fingerprint": "fnv1a32:f95ec173",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0078269996,
            "duration_seconds": 79.16933333333334,
            "function_call_errors": 0.0,
            "function_calls": 24.333333333333332,
            "sessions": 3.0,
            "tokens": 39473.5,
            "turns": 21.0
          },
          "contract_fingerprint": "fnv1a32:4fa46005",
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
          "scenario_id": "multi_subagent_validation",
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0006494226666666666,
            "duration_seconds": 7.476333333333334,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 3598.0,
            "turns": 4.0
          },
          "contract_fingerprint": "fnv1a32:3346e4ed",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.014693578666666665,
            "duration_seconds": 167.34366666666668,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 64.0,
            "sessions": 5.0,
            "tokens": 68963.66666666667,
            "turns": 36.333333333333336
          },
          "contract_fingerprint": "fnv1a32:645b9d93",
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
          "scenario_version": 4,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.017674124533333332,
            "duration_seconds": 223.83333333333334,
            "function_call_errors": 1.0,
            "function_calls": 27.666666666666668,
            "sessions": 3.0,
            "tokens": 83044.66666666667,
            "turns": 20.0
          },
          "contract_fingerprint": "fnv1a32:24d21247",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.011465049866666665,
            "duration_seconds": 68.16566666666667,
            "function_call_errors": 0.0,
            "function_calls": 18.666666666666668,
            "sessions": 1.0,
            "tokens": 65346.666666666664,
            "turns": 17.0
          },
          "contract_fingerprint": "fnv1a32:85829578",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.003963715466666667,
            "duration_seconds": 40.413,
            "function_call_errors": 0.0,
            "function_calls": 13.0,
            "sessions": 2.0,
            "tokens": 20389.0,
            "turns": 14.0
          },
          "contract_fingerprint": "fnv1a32:6f45dd8a",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0044629685333333335,
            "duration_seconds": 177.17533333333336,
            "function_call_errors": 0.0,
            "function_calls": 16.333333333333332,
            "sessions": 2.0,
            "tokens": 22490.0,
            "turns": 16.666666666666668
          },
          "contract_fingerprint": "fnv1a32:69f50761",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0019773450666666667,
            "duration_seconds": 26.56233333333333,
            "function_call_errors": 0.0,
            "function_calls": 7.333333333333333,
            "sessions": 1.0,
            "tokens": 10276.333333333334,
            "turns": 7.666666666666667
          },
          "contract_fingerprint": "fnv1a32:6ecbf11f",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.004077766933333333,
            "duration_seconds": 47.60166666666667,
            "function_call_errors": 0.0,
            "function_calls": 14.666666666666666,
            "sessions": 1.0,
            "tokens": 19800.333333333332,
            "turns": 11.0
          },
          "contract_fingerprint": "fnv1a32:deb3403d",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.0019334261333333335,
            "duration_seconds": 27.128,
            "function_call_errors": 1.0,
            "function_calls": 6.333333333333333,
            "sessions": 1.0,
            "tokens": 9424.333333333334,
            "turns": 6.333333333333333
          },
          "contract_fingerprint": "fnv1a32:8a795841",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.002822989866666667,
            "duration_seconds": 30.616,
            "function_call_errors": 0.0,
            "function_calls": 8.0,
            "sessions": 1.0,
            "tokens": 15281.666666666666,
            "turns": 8.666666666666666
          },
          "contract_fingerprint": "fnv1a32:e41c8856",
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
          "scenario_version": 3,
          "subject_id": "deepseek-v4-flash"
        }
      ],
      "source": {
        "ref": "daily/2026-08-10",
        "repository": "iii-hq/workers",
        "sha": "53275f249ced776220b1bb4aca2344cd600d6531"
      },
      "started_at": "2026-08-10T06:25:09Z",
      "status": "incomplete",
      "subjects": [
        {
          "engine_revision": "0.22.1",
          "expected_reports": 16,
          "hard_gate_failures": 2,
          "id": "deepseek-v4-flash",
          "infra_failures": 2,
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
          "received_reports": 14,
          "report_coverage": 0.875,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.8125,
          "scenarios": [
            {
              "hard_gate_failures": 0,
              "id": "direct_answer",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0047117176,
              "wall_time_seconds": 27.694
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "infra_failures": 0,
              "median_score": 90.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.001948268,
              "wall_time_seconds": 22.429
            },
            {
              "hard_gate_failures": 0,
              "id": "reactive_automation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.044080735999999995,
              "wall_time_seconds": 502.031
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0343951496,
              "wall_time_seconds": 204.497
            },
            {
              "hard_gate_failures": 2,
              "id": "research_pipeline",
              "infra_failures": 0,
              "median_score": 55.0,
              "pass_rate": 0.3333333333333333,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "total_cost_usd": 0.05302237359999999,
              "wall_time_seconds": 671.5
            },
            {
              "hard_gate_failures": 0,
              "id": "mechanical_reaction",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.015547873600000001,
              "wall_time_seconds": 173.405
            },
            {
              "hard_gate_failures": 0,
              "id": "timer_wake",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0059320352,
              "wall_time_seconds": 79.687
            },
            {
              "hard_gate_failures": null,
              "id": "receiving_operation",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": null,
              "id": "validation_loop",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0118911464,
              "wall_time_seconds": 121.239
            },
            {
              "hard_gate_failures": 0,
              "id": "multi_subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": null,
              "wall_time_seconds": 237.508
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation_failure",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0133889056,
              "wall_time_seconds": 531.526
            },
            {
              "hard_gate_failures": 0,
              "id": "custom_validator",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.0041763904000000004,
              "wall_time_seconds": 30.003
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_self_repair",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.008468969600000001,
              "wall_time_seconds": 91.848
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_scope_enforcement",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.005800278400000001,
              "wall_time_seconds": 81.384
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_chain",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "total_cost_usd": 0.012233300800000001,
              "wall_time_seconds": 142.805
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": null
        }
      ],
      "totals": {
        "average_score": 96.07142857142857,
        "expected_reports": 16,
        "function_calls": 656.0,
        "hard_gate_failures": 2,
        "missing_reports": 2,
        "passed_scenarios": 13,
        "received_reports": 14,
        "report_coverage": 87.5,
        "retries": 0,
        "scenario_pass_rate": 81.25,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": null
      },
      "workflow_duration_seconds": 2464.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31361902698"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-09T07:01:49Z",
      "conclusion": "failure",
      "detail_path": "runs/31298402184-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-09T07:01:49Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "a4bcd959e3eb782a57490f445b206b945522b92e",
        "id": "31298402184-1",
        "repository": "iii-hq/workers",
        "run_id": "31298402184",
        "started_at": "2026-08-09T06:13:31Z",
        "workflow_name": "Test · harness_registry · schedule",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31298402184"
      },
      "first_failure": {
        "kind": "missing_report",
        "message": "Missing report for deepseek-v4-flash/custom_validator",
        "scenario_id": "custom_validator",
        "subject_id": "deepseek-v4-flash"
      },
      "generated_at": "2026-08-09T07:01:42.515172+00:00",
      "id": "31298402184-1",
      "lane": "daily",
      "release": {
        "registry_tag": "latest",
        "tag": "daily/2026-08-09",
        "url": "https://github.com/iii-hq/workers/commit/a4bcd959e3eb782a57490f445b206b945522b92e",
        "version": "latest",
        "worker": "harness"
      },
      "requested_runs": 3,
      "run_id": "31298402184",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.006491365866666667,
            "duration_seconds": 45.260333333333335,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5817.0,
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
            "cost_usd": 0.0043525328,
            "duration_seconds": 57.69833333333333,
            "function_call_errors": 0.0,
            "function_calls": 13.666666666666666,
            "sessions": 1.0,
            "tokens": 21067.0,
            "turns": 9.666666666666666
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
            "cost_usd": 0.008072792,
            "duration_seconds": 95.18599999999999,
            "function_call_errors": 0.0,
            "function_calls": 27.0,
            "sessions": 3.0,
            "tokens": 38383.333333333336,
            "turns": 21.666666666666668
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
            "cost_usd": 0.014361405333333334,
            "duration_seconds": 139.86033333333333,
            "function_call_errors": 0.0,
            "function_calls": 62.0,
            "sessions": 5.0,
            "tokens": 68260.66666666667,
            "turns": 39.333333333333336
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
            "cost_usd": 0.0186775484,
            "duration_seconds": 316.09200000000004,
            "function_call_errors": 3.0,
            "function_calls": 46.0,
            "sessions": 4.0,
            "tokens": 84060.0,
            "turns": 31.333333333333332
          },
          "contract_fingerprint": "fnv1a32:4ac92d0d",
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
          "scenario_id": "receiving_operation",
          "scenario_version": 1,
          "subject_id": "deepseek-v4-flash"
        },
        {
          "averages": {
            "cost_usd": 0.014312007733333335,
            "duration_seconds": 143.446,
            "function_call_errors": 0.6666666666666666,
            "function_calls": 25.666666666666668,
            "sessions": 3.0,
            "tokens": 71631.66666666667,
            "turns": 17.666666666666668
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
            "cost_usd": 0.0032764458666666666,
            "duration_seconds": 13.429333333333332,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 3206.6666666666665,
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
            "cost_usd": 0.0040148792,
            "duration_seconds": 19.401666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 4106.333333333333,
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
            "cost_usd": 0.013359729600000002,
            "duration_seconds": 66.42466666666667,
            "function_call_errors": 0.0,
            "function_calls": 22.333333333333332,
            "sessions": 1.0,
            "tokens": 77942.33333333333,
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
            "cost_usd": 0.0041676506666666665,
            "duration_seconds": 175.727,
            "function_call_errors": 0.0,
            "function_calls": 15.666666666666666,
            "sessions": 2.0,
            "tokens": 21010.0,
            "turns": 15.666666666666666
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
            "cost_usd": 0.0019546912000000002,
            "duration_seconds": 27.496333333333336,
            "function_call_errors": 0.0,
            "function_calls": 7.333333333333333,
            "sessions": 1.0,
            "tokens": 9909.0,
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
            "cost_usd": 0.004255516533333333,
            "duration_seconds": 60.23466666666667,
            "function_call_errors": 0.0,
            "function_calls": 15.666666666666666,
            "sessions": 1.0,
            "tokens": 19691.333333333332,
            "turns": 12.666666666666666
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
            "cost_usd": 0.002994221066666667,
            "duration_seconds": 39.919333333333334,
            "function_call_errors": 0.0,
            "function_calls": 9.666666666666666,
            "sessions": 1.0,
            "tokens": 14517.0,
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
            "cost_usd": 0.0011758880000000002,
            "duration_seconds": 15.858333333333334,
            "function_call_errors": 1.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 5869.333333333333,
            "turns": 4.333333333333333
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
            "cost_usd": 0.002994831466666667,
            "duration_seconds": 27.225666666666665,
            "function_call_errors": 0.0,
            "function_calls": 10.666666666666666,
            "sessions": 1.0,
            "tokens": 16878.0,
            "turns": 8.666666666666666
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
        "ref": "daily/2026-08-09",
        "repository": "iii-hq/workers",
        "sha": "a4bcd959e3eb782a57490f445b206b945522b92e"
      },
      "started_at": "2026-08-09T06:13:31Z",
      "status": "incomplete",
      "subjects": [
        {
          "engine_revision": "0.22.1",
          "expected_reports": 19,
          "hard_gate_failures": 4,
          "id": "deepseek-v4-flash",
          "infra_failures": 3,
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
          "received_reports": 15,
          "report_coverage": 0.7894736842105263,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.7894736842105263,
          "scenarios": [
            {
              "hard_gate_failures": null,
              "id": "direct_answer",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": null,
              "id": "persistent_state",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": 0,
              "id": "security_review",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0098293376,
              "wall_time_seconds": 40.288
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.043084216,
              "wall_time_seconds": 419.581
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.040079188800000005,
              "wall_time_seconds": 199.274
            },
            {
              "hard_gate_failures": 0,
              "id": "design_tradeoff",
              "infra_failures": 0,
              "median_score": 95.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0194740976,
              "wall_time_seconds": 135.781
            },
            {
              "hard_gate_failures": 0,
              "id": "security_triage",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.012044637600000001,
              "wall_time_seconds": 58.205
            },
            {
              "hard_gate_failures": 1,
              "id": "research_pipeline",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.042936023200000006,
              "wall_time_seconds": 430.338
            },
            {
              "hard_gate_failures": 0,
              "id": "mechanical_reaction",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.013057598400000002,
              "wall_time_seconds": 173.095
            },
            {
              "hard_gate_failures": 0,
              "id": "timer_wake",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.005864073600000001,
              "wall_time_seconds": 82.489
            },
            {
              "hard_gate_failures": 1,
              "id": "receiving_operation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": null,
              "wall_time_seconds": 948.276
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_loop",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.008982663200000001,
              "wall_time_seconds": 119.758
            },
            {
              "hard_gate_failures": null,
              "id": "subagent_validation",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": 0,
              "id": "multi_subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.024218376,
              "wall_time_seconds": 285.558
            },
            {
              "hard_gate_failures": 1,
              "id": "subagent_validation_failure",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.012502952,
              "wall_time_seconds": 527.181
            },
            {
              "hard_gate_failures": null,
              "id": "custom_validator",
              "infra_failures": 0,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "missing_report",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_self_repair",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 80.0,
              "total_cost_usd": 0.008984494400000001,
              "wall_time_seconds": 81.677
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_scope_enforcement",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0035276640000000007,
              "wall_time_seconds": 47.575
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_chain",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0127665496,
              "wall_time_seconds": 180.704
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": null
        }
      ],
      "totals": {
        "average_score": 99.66666666666667,
        "expected_reports": 19,
        "function_calls": 776.0,
        "hard_gate_failures": 4,
        "missing_reports": 4,
        "passed_scenarios": 15,
        "received_reports": 15,
        "report_coverage": 78.94736842105263,
        "retries": 0,
        "scenario_pass_rate": 78.94736842105263,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": null
      },
      "workflow_duration_seconds": 2898.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31298402184"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-08T07:07:17Z",
      "conclusion": "failure",
      "detail_path": "runs/31243403120-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-08T07:07:17Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "a4bcd959e3eb782a57490f445b206b945522b92e",
        "id": "31243403120-1",
        "repository": "iii-hq/workers",
        "run_id": "31243403120",
        "started_at": "2026-08-08T06:12:35Z",
        "workflow_name": "Test · harness_registry · schedule",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31243403120"
      },
      "first_failure": {
        "id": "analysts_spawned_directly_in_parallel",
        "kind": "hard_gate",
        "message": "spawns=3, parallel_calls=false, overlapping_sessions=false, direct_sessions=true",
        "scenario_id": "research_pipeline",
        "subject_id": "deepseek-v4-flash"
      },
      "generated_at": "2026-08-08T07:07:12.656474+00:00",
      "id": "31243403120-1",
      "lane": "daily",
      "release": {
        "registry_tag": "latest",
        "tag": "daily/2026-08-08",
        "url": "https://github.com/iii-hq/workers/commit/a4bcd959e3eb782a57490f445b206b945522b92e",
        "version": "1.7.4",
        "worker": "harness"
      },
      "requested_runs": 3,
      "run_id": "31243403120",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0014318266666666667,
            "duration_seconds": 12.875333333333332,
            "function_call_errors": 0.0,
            "function_calls": 3.3333333333333335,
            "sessions": 1.0,
            "tokens": 8332.0,
            "turns": 5.666666666666667
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
            "cost_usd": 0.005175552533333333,
            "duration_seconds": 42.597,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5430.333333333333,
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
            "cost_usd": 0.0015362592000000002,
            "duration_seconds": 4.707333333333334,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2139.0,
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
            "cost_usd": 0.003559832266666667,
            "duration_seconds": 47.137,
            "function_call_errors": 0.0,
            "function_calls": 10.666666666666666,
            "sessions": 1.0,
            "tokens": 18181.333333333332,
            "turns": 8.0
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
            "cost_usd": 0.0061611218666666676,
            "duration_seconds": 61.43933333333334,
            "function_call_errors": 0.0,
            "function_calls": 23.333333333333332,
            "sessions": 3.0,
            "tokens": 31720.666666666668,
            "turns": 18.666666666666668
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
            "cost_usd": 0.0006679232,
            "duration_seconds": 7.328666666666667,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 3694.6666666666665,
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
            "cost_usd": 0.015232798933333335,
            "duration_seconds": 166.27066666666667,
            "function_call_errors": 0.6666666666666666,
            "function_calls": 62.0,
            "sessions": 5.0,
            "tokens": 72235.33333333333,
            "turns": 37.333333333333336
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
            "cost_usd": 0.022789164533333336,
            "duration_seconds": 362.4026666666667,
            "function_call_errors": 2.0,
            "function_calls": 53.666666666666664,
            "sessions": 4.0,
            "tokens": 92320.33333333333,
            "turns": 36.666666666666664
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
            "cost_usd": 0.016054333866666666,
            "duration_seconds": 201.85500000000002,
            "function_call_errors": 0.6666666666666666,
            "function_calls": 24.0,
            "sessions": 3.0,
            "tokens": 75517.66666666667,
            "turns": 18.0
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
            "cost_usd": 0.0030146192,
            "duration_seconds": 12.331999999999999,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2888.6666666666665,
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
            "cost_usd": 0.0040890792,
            "duration_seconds": 19.388333333333332,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 4106.0,
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
            "cost_usd": 0.013468718666666666,
            "duration_seconds": 79.33800000000001,
            "function_call_errors": 0.0,
            "function_calls": 22.333333333333332,
            "sessions": 1.0,
            "tokens": 74545.0,
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
            "cost_usd": 0.0040371968,
            "duration_seconds": 41.227333333333334,
            "function_call_errors": 0.0,
            "function_calls": 15.666666666666666,
            "sessions": 2.0,
            "tokens": 21252.333333333332,
            "turns": 13.333333333333334
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
            "cost_usd": 0.004991686933333334,
            "duration_seconds": 176.91366666666667,
            "function_call_errors": 0.0,
            "function_calls": 18.333333333333332,
            "sessions": 2.0,
            "tokens": 24693.0,
            "turns": 16.333333333333332
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
            "cost_usd": 0.0020998301333333335,
            "duration_seconds": 31.588666666666665,
            "function_call_errors": 0.0,
            "function_calls": 7.0,
            "sessions": 1.0,
            "tokens": 10663.333333333334,
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
            "cost_usd": 0.0040412568,
            "duration_seconds": 54.785,
            "function_call_errors": 0.0,
            "function_calls": 16.666666666666668,
            "sessions": 1.0,
            "tokens": 19152.666666666668,
            "turns": 11.333333333333334
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
            "cost_usd": 0.0025515541333333337,
            "duration_seconds": 34.12733333333333,
            "function_call_errors": 0.0,
            "function_calls": 8.666666666666666,
            "sessions": 1.0,
            "tokens": 12466.666666666666,
            "turns": 9.666666666666666
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
            "cost_usd": 0.0013237541333333332,
            "duration_seconds": 19.834333333333333,
            "function_call_errors": 1.0,
            "function_calls": 4.333333333333333,
            "sessions": 1.0,
            "tokens": 6325.666666666667,
            "turns": 6.0
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
            "cost_usd": 0.0030949837333333337,
            "duration_seconds": 41.031,
            "function_call_errors": 0.0,
            "function_calls": 12.0,
            "sessions": 1.0,
            "tokens": 15903.333333333334,
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
        "ref": "daily/2026-08-08",
        "repository": "iii-hq/workers",
        "sha": "a4bcd959e3eb782a57490f445b206b945522b92e"
      },
      "started_at": "2026-08-08T06:12:35Z",
      "status": "hard_gate_failed",
      "subjects": [
        {
          "engine_revision": "0.22.1",
          "expected_reports": 19,
          "hard_gate_failures": 4,
          "id": "deepseek-v4-flash",
          "infra_failures": 0,
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
          "scenario_pass_rate": 0.8947368421052632,
          "scenarios": [
            {
              "hard_gate_failures": 0,
              "id": "direct_answer",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0046087776,
              "wall_time_seconds": 14.122
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "infra_failures": 0,
              "median_score": 90.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0020037696,
              "wall_time_seconds": 21.986
            },
            {
              "hard_gate_failures": 0,
              "id": "security_review",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0090438576,
              "wall_time_seconds": 36.996
            },
            {
              "hard_gate_failures": 0,
              "id": "reactive_automation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.04569839680000001,
              "wall_time_seconds": 498.812
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.04040615599999999,
              "wall_time_seconds": 238.014
            },
            {
              "hard_gate_failures": 0,
              "id": "design_tradeoff",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0155266576,
              "wall_time_seconds": 127.791
            },
            {
              "hard_gate_failures": 0,
              "id": "security_triage",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0122672376,
              "wall_time_seconds": 58.165
            },
            {
              "hard_gate_failures": 2,
              "id": "research_pipeline",
              "infra_failures": 0,
              "median_score": 55.0,
              "pass_rate": 0.3333333333333333,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.048163001600000005,
              "wall_time_seconds": 605.565
            },
            {
              "hard_gate_failures": 0,
              "id": "mechanical_reaction",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0106794968,
              "wall_time_seconds": 141.411
            },
            {
              "hard_gate_failures": 0,
              "id": "timer_wake",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0062994904,
              "wall_time_seconds": 94.766
            },
            {
              "hard_gate_failures": 2,
              "id": "receiving_operation",
              "infra_failures": 0,
              "median_score": 75.0,
              "pass_rate": 0.3333333333333333,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.06836749360000001,
              "wall_time_seconds": 1087.208
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_loop",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0076546624,
              "wall_time_seconds": 102.382
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0121115904,
              "wall_time_seconds": 123.682
            },
            {
              "hard_gate_failures": 0,
              "id": "multi_subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.018483365600000003,
              "wall_time_seconds": 184.318
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation_failure",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.014975060799999999,
              "wall_time_seconds": 530.741
            },
            {
              "hard_gate_failures": 0,
              "id": "custom_validator",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.00429548,
              "wall_time_seconds": 38.626
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_self_repair",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 80.0,
              "total_cost_usd": 0.009284951199999999,
              "wall_time_seconds": 123.093
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_scope_enforcement",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0039712624,
              "wall_time_seconds": 59.503
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_chain",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0121237704,
              "wall_time_seconds": 164.355
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 0.3459644784,
          "wall_time_seconds": 4251.536
        }
      ],
      "totals": {
        "average_score": 95.78947368421052,
        "expected_reports": 19,
        "function_calls": 855.0,
        "hard_gate_failures": 4,
        "missing_reports": 0,
        "passed_scenarios": 17,
        "received_reports": 19,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 89.47368421052632,
        "technical_failures": 0,
        "total_cost_usd": 0.3459644784,
        "total_tokens": 1504704.0,
        "wall_time_seconds": 4251.536
      },
      "workflow_duration_seconds": 3282.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31243403120"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-07T12:00:57Z",
      "conclusion": "failure",
      "detail_path": "runs/31173225962-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-07T12:00:57Z",
        "conclusion": "failure",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "fc24e3cd5a3596b485b30511f2fa79db8551e851",
        "id": "31173225962-1",
        "repository": "iii-hq/workers",
        "run_id": "31173225962",
        "started_at": "2026-08-07T11:13:40Z",
        "workflow_name": "Test · harness_registry · workflow_dispatch",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31173225962"
      },
      "first_failure": {
        "job_name": "e2e / harness e2e / deepseek-v4-flash / reactive_automation",
        "kind": "job",
        "message": "e2e / harness e2e / deepseek-v4-flash / reactive_automation: Run deployed-stack quality scenario",
        "step_name": "Run deployed-stack quality scenario",
        "url": "https://github.com/iii-hq/workers/actions/runs/31173225962/job/92850800553"
      },
      "generated_at": "2026-08-07T12:00:51.083770+00:00",
      "id": "31173225962-1",
      "lane": "daily",
      "release": {
        "registry_tag": "latest",
        "tag": "daily/2026-08-07",
        "url": "https://github.com/iii-hq/workers/commit/fc24e3cd5a3596b485b30511f2fa79db8551e851",
        "version": "latest",
        "worker": "harness"
      },
      "requested_runs": 3,
      "run_id": "31173225962",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.0010744346666666667,
            "duration_seconds": 9.889333333333333,
            "function_call_errors": 0.0,
            "function_calls": 1.0,
            "sessions": 1.0,
            "tokens": 6440.666666666667,
            "turns": 4.0
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
            "cost_usd": 0.005372712533333333,
            "duration_seconds": 42.083,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 5616.666666666667,
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
            "cost_usd": 0.0015514392,
            "duration_seconds": 4.855666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2098.6666666666665,
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
            "cost_usd": 0.0037074986666666677,
            "duration_seconds": 46.64266666666666,
            "function_call_errors": 0.0,
            "function_calls": 12.0,
            "sessions": 1.0,
            "tokens": 19090.666666666668,
            "turns": 7.666666666666667
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
            "cost_usd": 0.008688887200000002,
            "duration_seconds": 87.67966666666666,
            "function_call_errors": 0.0,
            "function_calls": 26.0,
            "sessions": 3.0,
            "tokens": 43680.333333333336,
            "turns": 21.0
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
            "cost_usd": 0.0007673624000000002,
            "duration_seconds": 6.68,
            "function_call_errors": 0.0,
            "function_calls": 3.0,
            "sessions": 1.0,
            "tokens": 4404.0,
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
            "cost_usd": 0.015125622400000002,
            "duration_seconds": 181.13466666666667,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 58.333333333333336,
            "sessions": 5.0,
            "tokens": 69860.0,
            "turns": 37.0
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
            "cost_usd": 0.024681802133333333,
            "duration_seconds": 439.895,
            "function_call_errors": 2.3333333333333335,
            "function_calls": 52.333333333333336,
            "sessions": 4.0,
            "tokens": 99820.33333333333,
            "turns": 35.666666666666664
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
            "cost_usd": 0.0029822792,
            "duration_seconds": 11.444333333333333,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 2814.3333333333335,
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
            "cost_usd": 0.011935128800000002,
            "duration_seconds": 64.42133333333334,
            "function_call_errors": 0.0,
            "function_calls": 19.333333333333332,
            "sessions": 1.0,
            "tokens": 68757.0,
            "turns": 17.0
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
            "cost_usd": 0.004769648800000001,
            "duration_seconds": 54.041666666666664,
            "function_call_errors": 0.0,
            "function_calls": 16.333333333333332,
            "sessions": 2.0,
            "tokens": 24223.666666666668,
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
            "cost_usd": 0.0017315536000000002,
            "duration_seconds": 25.874333333333336,
            "function_call_errors": 0.0,
            "function_calls": 6.333333333333333,
            "sessions": 1.0,
            "tokens": 8999.0,
            "turns": 6.333333333333333
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
            "cost_usd": 0.004135021333333333,
            "duration_seconds": 52.007666666666665,
            "function_call_errors": 0.0,
            "function_calls": 16.666666666666668,
            "sessions": 1.0,
            "tokens": 19891.0,
            "turns": 12.666666666666666
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
            "cost_usd": 0.0027140045333333336,
            "duration_seconds": 36.35666666666666,
            "function_call_errors": 0.0,
            "function_calls": 10.333333333333334,
            "sessions": 1.0,
            "tokens": 13414.0,
            "turns": 9.666666666666666
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
            "cost_usd": 0.0029948576000000007,
            "duration_seconds": 43.52533333333333,
            "function_call_errors": 1.0,
            "function_calls": 5.666666666666667,
            "sessions": 1.0,
            "tokens": 14696.666666666666,
            "turns": 6.666666666666667
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
            "cost_usd": 0.0028185808,
            "duration_seconds": 32.147999999999996,
            "function_call_errors": 0.0,
            "function_calls": 9.333333333333334,
            "sessions": 1.0,
            "tokens": 15487.666666666666,
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
        "sha": "fc24e3cd5a3596b485b30511f2fa79db8551e851"
      },
      "started_at": "2026-08-07T11:13:40Z",
      "status": "incomplete",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 19,
          "hard_gate_failures": 3,
          "id": "deepseek-v4-flash",
          "infra_failures": 3,
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
          "received_reports": 16,
          "report_coverage": 0.8421052631578947,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.7894736842105263,
          "scenarios": [
            {
              "hard_gate_failures": 0,
              "id": "direct_answer",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0046543176,
              "wall_time_seconds": 14.567
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0023020872000000005,
              "wall_time_seconds": 20.04
            },
            {
              "hard_gate_failures": 0,
              "id": "security_review",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0089468376,
              "wall_time_seconds": 34.333
            },
            {
              "hard_gate_failures": 2,
              "id": "reactive_automation",
              "infra_failures": 0,
              "median_score": 75.0,
              "pass_rate": 0.3333333333333333,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.045376867200000004,
              "wall_time_seconds": 543.404
            },
            {
              "hard_gate_failures": 0,
              "id": "shell_coder_sandbox",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.03580538640000001,
              "wall_time_seconds": 193.264
            },
            {
              "hard_gate_failures": 0,
              "id": "design_tradeoff",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 0.0161181376,
              "wall_time_seconds": 126.249
            },
            {
              "hard_gate_failures": null,
              "id": "security_triage",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": null,
              "id": "research_pipeline",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": 0,
              "id": "mechanical_reaction",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.011122496000000003,
              "wall_time_seconds": 139.928
            },
            {
              "hard_gate_failures": 1,
              "id": "timer_wake",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "hard_gate_failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0051946608000000005,
              "wall_time_seconds": 77.623
            },
            {
              "hard_gate_failures": 0,
              "id": "receiving_operation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0740454064,
              "wall_time_seconds": 1319.685
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_loop",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.008142013600000001,
              "wall_time_seconds": 109.07
            },
            {
              "hard_gate_failures": 0,
              "id": "subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.014308946400000001,
              "wall_time_seconds": 162.125
            },
            {
              "hard_gate_failures": 0,
              "id": "multi_subagent_validation",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.026066661600000006,
              "wall_time_seconds": 263.039
            },
            {
              "hard_gate_failures": null,
              "id": "subagent_validation_failure",
              "infra_failures": 1,
              "median_score": null,
              "pass_rate": null,
              "passed": false,
              "retries": null,
              "runs": 0,
              "status": "infra_failed",
              "technical_failures": null,
              "threshold": null,
              "total_cost_usd": null,
              "wall_time_seconds": null
            },
            {
              "hard_gate_failures": 0,
              "id": "custom_validator",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.003223304,
              "wall_time_seconds": 29.668
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_self_repair",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 80.0,
              "total_cost_usd": 0.0084557424,
              "wall_time_seconds": 96.444
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_scope_enforcement",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.008984572800000002,
              "wall_time_seconds": 130.576
            },
            {
              "hard_gate_failures": 0,
              "id": "validation_chain",
              "infra_failures": 0,
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.012405064,
              "wall_time_seconds": 156.023
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": null
        }
      ],
      "totals": {
        "average_score": 98.4375,
        "expected_reports": 19,
        "function_calls": 710.0,
        "hard_gate_failures": 3,
        "missing_reports": 3,
        "passed_scenarios": 15,
        "received_reports": 16,
        "report_coverage": 84.21052631578947,
        "retries": 0,
        "scenario_pass_rate": 78.94736842105263,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": 1257884.0,
        "wall_time_seconds": null
      },
      "workflow_duration_seconds": 2837.0,
      "workflow_name": "Test · harness_registry · workflow_dispatch",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31173225962"
    },
    {
      "actor": "ytallo",
      "attempt": 3,
      "availability": "unavailable",
      "completed_at": "2026-08-07T11:07:33Z",
      "conclusion": "failure",
      "detail_path": null,
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 3,
        "completed_at": "2026-08-07T11:07:33Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "abfadf473ecb33452372b9fa06f083639b6ba19a",
        "id": "31153706659-3",
        "repository": "iii-hq/workers",
        "run_id": "31153706659",
        "started_at": "2026-08-07T11:05:02Z",
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
      "id": "31153706659-3",
      "lane": "daily",
      "release": {},
      "requested_runs": null,
      "run_id": "31153706659",
      "source": {
        "ref": "main",
        "repository": "iii-hq/workers",
        "sha": "abfadf473ecb33452372b9fa06f083639b6ba19a"
      },
      "started_at": "2026-08-07T11:05:02Z",
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
      "workflow_duration_seconds": 151.0,
      "workflow_name": "Test · harness_registry · schedule",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/31153706659"
    },
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
  "last_update": "2026-08-11T07:04:27Z",
  "mode": "published",
  "repo_url": "https://github.com/iii-hq/workers",
  "retention": {
    "details": 30,
    "summaries": 100
  },
  "schema_version": 3
};
