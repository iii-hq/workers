window.HARNESS_EXECUTIONS = {
  "executions": [
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-03T20:40:46Z",
      "conclusion": "success",
      "detail_path": "runs/30849845674-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-03T20:40:46Z",
        "conclusion": "success",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "1795404b4a1ea6c82f835b1df2bc04b64e63886f",
        "id": "30849845674-1",
        "repository": "iii-hq/workers",
        "run_id": "30849845674",
        "started_at": "2026-08-03T20:21:32Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30849845674"
      },
      "generated_at": "2026-08-03T20:40:11.358516+00:00",
      "id": "30849845674-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-03",
        "url": "https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b",
        "version": "2026-08-03",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30849845674",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.022471533333333335,
            "duration_seconds": 29.013666666666666,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 8517.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.01339792,
            "duration_seconds": 5.049,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6933.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.02810052,
            "duration_seconds": 5.092333333333333,
            "function_call_errors": 0.0,
            "function_calls": 1.3333333333333333,
            "sessions": 1.0,
            "tokens": 16681.666666666668,
            "turns": 2.3333333333333335
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 1.7166767999999997,
            "duration_seconds": 231.76333333333332,
            "function_call_errors": 2.6666666666666665,
            "function_calls": 149.0,
            "sessions": 6.666666666666667,
            "tokens": 1012843.5,
            "turns": 96.66666666666667
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
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
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.017251053333333332,
            "duration_seconds": 13.893,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7617.333333333333,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.019739946666666668,
            "duration_seconds": 15.430333333333332,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7798.333333333333,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.57966252,
            "duration_seconds": 66.74766666666666,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 18.333333333333332,
            "sessions": 1.0,
            "tokens": 350072.0,
            "turns": 17.333333333333332
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-08-03",
        "repository": "iii-hq/workers",
        "sha": "83966dc6acd3d53a54c73e73bc5faf1bf8a3511b"
      },
      "started_at": "2026-08-03T20:21:32Z",
      "status": "passed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 7,
          "hard_gate_failures": 1,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "glm-5.2",
          "passed": true,
          "provider": "zai",
          "received_reports": 7,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 1.0,
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
              "total_cost_usd": 0.040193759999999995,
              "wall_time_seconds": 15.147
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.08430156,
              "wall_time_seconds": 15.277
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
              "total_cost_usd": 0.051753160000000006,
              "wall_time_seconds": 41.679
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
              "total_cost_usd": null,
              "wall_time_seconds": 695.29
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
              "total_cost_usd": 1.73898756,
              "wall_time_seconds": 200.243
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
              "total_cost_usd": 0.0674146,
              "wall_time_seconds": 87.041
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
              "total_cost_usd": 0.05921984,
              "wall_time_seconds": 46.291
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": 1100.9679999999998
        }
      ],
      "totals": {
        "average_score": 100.0,
        "expected_reports": 7,
        "function_calls": 506.0,
        "hard_gate_failures": 1,
        "missing_reports": 0,
        "passed_scenarios": 7,
        "received_reports": 7,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 100.0,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": 1100.9679999999998
      },
      "workflow_duration_seconds": 1154.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30849845674"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-03T14:43:34Z",
      "conclusion": "cancelled",
      "detail_path": "runs/30820138874-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-03T14:43:34Z",
        "conclusion": "cancelled",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "a6439b373debe0a1024959196d7e3190225d3721",
        "id": "30820138874-1",
        "repository": "iii-hq/workers",
        "run_id": "30820138874",
        "started_at": "2026-08-03T13:54:32Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30820138874"
      },
      "generated_at": "2026-08-03T14:42:24.617600+00:00",
      "id": "30820138874-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-03",
        "url": "https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1",
        "version": "2026-08-03",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30820138874",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.022208066666666665,
            "duration_seconds": 33.994,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 8457.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.012277586666666666,
            "duration_seconds": 6.6819999999999995,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6930.666666666667,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.023361053333333336,
            "duration_seconds": 8.019666666666666,
            "function_call_errors": 0.0,
            "function_calls": 1.0,
            "sessions": 1.0,
            "tokens": 14198.666666666666,
            "turns": 2.0
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.016470933333333333,
            "duration_seconds": 15.674666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7489.333333333333,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.01836124,
            "duration_seconds": 17.594,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7816.666666666667,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.6029078933333333,
            "duration_seconds": 71.92999999999999,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 17.333333333333332,
            "sessions": 1.0,
            "tokens": 363629.6666666667,
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-08-03",
        "repository": "iii-hq/workers",
        "sha": "10a168a75dc2b7088d976621bd76ccaa4d146ff1"
      },
      "started_at": "2026-08-03T13:54:32Z",
      "status": "cancelled",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 7,
          "hard_gate_failures": 0,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 6,
          "report_coverage": 0.8571428571428571,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.8571428571428571,
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
              "total_cost_usd": 0.03683276,
              "wall_time_seconds": 20.046
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.07008316,
              "wall_time_seconds": 24.059
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
              "total_cost_usd": 0.0494128,
              "wall_time_seconds": 47.024
            },
            {
              "hard_gate_failures": null,
              "id": "reactive_automation",
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
              "id": "shell_coder_sandbox",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 1.80872368,
              "wall_time_seconds": 215.79
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
              "total_cost_usd": 0.0666242,
              "wall_time_seconds": 101.982
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
              "total_cost_usd": 0.05508372,
              "wall_time_seconds": 52.782
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": null
        }
      ],
      "totals": {
        "average_score": 100.0,
        "expected_reports": 7,
        "function_calls": 55.0,
        "hard_gate_failures": 0,
        "missing_reports": 1,
        "passed_scenarios": 6,
        "received_reports": 6,
        "report_coverage": 85.71428571428571,
        "retries": 0,
        "scenario_pass_rate": 85.71428571428571,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": 1225566.0,
        "wall_time_seconds": null
      },
      "workflow_duration_seconds": 2942.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30820138874"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-03T07:19:59Z",
      "conclusion": "failure",
      "detail_path": "runs/30792193792-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-03T07:19:59Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "4b52a11e730597f7e8743e17f4a12d387943946e",
        "id": "30792193792-1",
        "repository": "iii-hq/workers",
        "run_id": "30792193792",
        "started_at": "2026-08-03T07:02:41Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30792193792"
      },
      "generated_at": "2026-08-03T07:18:51.360492+00:00",
      "id": "30792193792-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-03",
        "url": "https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e",
        "version": "2026-08-03",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30792193792",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.012372866666666668,
            "duration_seconds": 12.054,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6935.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.032131066666666666,
            "duration_seconds": 11.693666666666667,
            "function_call_errors": 0.0,
            "function_calls": 1.6666666666666667,
            "sessions": 1.0,
            "tokens": 19100.666666666668,
            "turns": 2.6666666666666665
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": null,
            "duration_seconds": 351.918,
            "function_call_errors": 0.0,
            "function_calls": 56.0,
            "sessions": 6.0,
            "tokens": null,
            "turns": 39.0
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
          "run_count": 1,
          "samples": {
            "cost_usd": 0,
            "duration_seconds": 1,
            "function_call_errors": 1,
            "function_calls": 1,
            "sessions": 1,
            "tokens": 0,
            "turns": 1
          },
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.01690032,
            "duration_seconds": 16.911666666666665,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7615.666666666667,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.53378436,
            "duration_seconds": 124.05466666666666,
            "function_call_errors": 0.0,
            "function_calls": 16.0,
            "sessions": 1.0,
            "tokens": 321684.3333333333,
            "turns": 16.333333333333332
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-08-03",
        "repository": "iii-hq/workers",
        "sha": "4b52a11e730597f7e8743e17f4a12d387943946e"
      },
      "started_at": "2026-08-03T07:02:41Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 2,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.6,
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
              "total_cost_usd": 0.0371186,
              "wall_time_seconds": 36.162
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.0963932,
              "wall_time_seconds": 35.081
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
              "total_cost_usd": 0.05070096,
              "wall_time_seconds": 50.735
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 0.0,
              "pass_rate": 0.0,
              "passed": false,
              "retries": 0,
              "runs": 1,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": null,
              "wall_time_seconds": 351.918
            },
            {
              "hard_gate_failures": 1,
              "id": "shell_coder_sandbox",
              "median_score": 100.0,
              "pass_rate": 0.6666666666666666,
              "passed": false,
              "retries": 0,
              "runs": 3,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 50.0,
              "total_cost_usd": 1.60135308,
              "wall_time_seconds": 372.164
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": 846.06
        }
      ],
      "totals": {
        "average_score": 80.0,
        "expected_reports": 5,
        "function_calls": 109.0,
        "hard_gate_failures": 2,
        "missing_reports": 0,
        "passed_scenarios": 3,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 60.0,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": 846.06
      },
      "workflow_duration_seconds": 1038.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30792193792"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-02T07:13:59Z",
      "conclusion": "success",
      "detail_path": "runs/30736657213-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-02T07:13:59Z",
        "conclusion": "success",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "4b056e50720252afcb05ff4f5722149b7c5db1c6",
        "id": "30736657213-1",
        "repository": "iii-hq/workers",
        "run_id": "30736657213",
        "started_at": "2026-08-02T06:50:38Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30736657213"
      },
      "generated_at": "2026-08-02T07:06:40.367600+00:00",
      "id": "30736657213-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-02",
        "url": "https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6",
        "version": "2026-08-02",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30736657213",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.012688,
            "duration_seconds": 5.178333333333334,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6932.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.02363057333333333,
            "duration_seconds": 5.290333333333333,
            "function_call_errors": 0.0,
            "function_calls": 1.0,
            "sessions": 1.0,
            "tokens": 14216.666666666666,
            "turns": 2.0
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 2.2054987199999996,
            "duration_seconds": 249.1465,
            "function_call_errors": 1.5,
            "function_calls": 150.0,
            "sessions": 7.0,
            "tokens": 1307053.0,
            "turns": 116.5
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
          "run_count": 2,
          "samples": {
            "cost_usd": 2,
            "duration_seconds": 2,
            "function_call_errors": 2,
            "function_calls": 2,
            "sessions": 2,
            "tokens": 2,
            "turns": 2
          },
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.016691373333333332,
            "duration_seconds": 11.456666666666669,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7575.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.60127564,
            "duration_seconds": 58.41433333333333,
            "function_call_errors": 0.0,
            "function_calls": 18.333333333333332,
            "sessions": 1.0,
            "tokens": 363029.6666666667,
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-08-02",
        "repository": "iii-hq/workers",
        "sha": "4b056e50720252afcb05ff4f5722149b7c5db1c6"
      },
      "started_at": "2026-08-02T06:50:38Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 1,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.8,
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
              "total_cost_usd": 0.038064,
              "wall_time_seconds": 15.535
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.07089171999999999,
              "wall_time_seconds": 15.871
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
              "total_cost_usd": 0.05007412,
              "wall_time_seconds": 34.37
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 72.5,
              "pass_rate": 0.5,
              "passed": false,
              "retries": 0,
              "runs": 2,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 4.410997439999999,
              "wall_time_seconds": 498.293
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
              "total_cost_usd": 1.80382692,
              "wall_time_seconds": 175.243
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 6.373854199999999,
          "wall_time_seconds": 739.312
        }
      ],
      "totals": {
        "average_score": 94.5,
        "expected_reports": 5,
        "function_calls": 358.0,
        "hard_gate_failures": 1,
        "missing_reports": 0,
        "passed_scenarios": 4,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 80.0,
        "technical_failures": 0,
        "total_cost_usd": 6.373854199999999,
        "total_tokens": 3789366.0,
        "wall_time_seconds": 739.312
      },
      "workflow_duration_seconds": 1401.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30736657213"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-08-01T07:21:53Z",
      "conclusion": "failure",
      "detail_path": "runs/30688423602-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-08-01T07:21:53Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "12700ae892089d08ec5e30e447ce92407c75e8ec",
        "id": "30688423602-1",
        "repository": "iii-hq/workers",
        "run_id": "30688423602",
        "started_at": "2026-08-01T06:47:34Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30688423602"
      },
      "generated_at": "2026-08-01T07:21:24.697528+00:00",
      "id": "30688423602-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-08-01",
        "url": "https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec",
        "version": "2026-08-01",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30688423602",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.012112853333333333,
            "duration_seconds": 7.108666666666667,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6931.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.023445320000000002,
            "duration_seconds": 7.564333333333333,
            "function_call_errors": 0.0,
            "function_calls": 1.0,
            "sessions": 1.0,
            "tokens": 14231.0,
            "turns": 2.0
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 2.5270296,
            "duration_seconds": 607.067,
            "function_call_errors": 3.0,
            "function_calls": 131.0,
            "sessions": 6.0,
            "tokens": 1493294.0,
            "turns": 113.0
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
          "run_count": 1,
          "samples": {
            "cost_usd": 1,
            "duration_seconds": 1,
            "function_call_errors": 1,
            "function_calls": 1,
            "sessions": 1,
            "tokens": 1,
            "turns": 1
          },
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.01651224,
            "duration_seconds": 66.695,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7546.0,
            "turns": 1.0
          },
          "contract_fingerprint": "fnv1a32:b82f2ff2",
          "run_count": 2,
          "samples": {
            "cost_usd": 1,
            "duration_seconds": 2,
            "function_call_errors": 2,
            "function_calls": 2,
            "sessions": 2,
            "tokens": 1,
            "turns": 2
          },
          "scenario_id": "security_review",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.6154473066666666,
            "duration_seconds": 108.55266666666667,
            "function_call_errors": 0.0,
            "function_calls": 18.666666666666668,
            "sessions": 1.0,
            "tokens": 371471.6666666667,
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-08-01",
        "repository": "iii-hq/workers",
        "sha": "12700ae892089d08ec5e30e447ce92407c75e8ec"
      },
      "started_at": "2026-08-01T06:47:34Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 1,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.6,
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
              "total_cost_usd": 0.03633856,
              "wall_time_seconds": 21.326
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.07033596,
              "wall_time_seconds": 22.693
            },
            {
              "hard_gate_failures": 0,
              "id": "security_review",
              "median_score": 100.0,
              "pass_rate": 0.5,
              "passed": false,
              "retries": 0,
              "runs": 2,
              "status": "failed",
              "technical_failures": 1,
              "threshold": 50.0,
              "total_cost_usd": null,
              "wall_time_seconds": 133.39
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 50.0,
              "pass_rate": 0.0,
              "passed": false,
              "retries": 0,
              "runs": 1,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 2.5270296,
              "wall_time_seconds": 607.067
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
              "total_cost_usd": 1.84634192,
              "wall_time_seconds": 325.658
            }
          ],
          "technical_failures": 1,
          "total_cost_usd": null,
          "wall_time_seconds": 1110.134
        }
      ],
      "totals": {
        "average_score": 90.0,
        "expected_reports": 5,
        "function_calls": 190.0,
        "hard_gate_failures": 1,
        "missing_reports": 0,
        "passed_scenarios": 3,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 60.0,
        "technical_failures": 1,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": 1110.134
      },
      "workflow_duration_seconds": 2059.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30688423602"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-07-31T07:38:50Z",
      "conclusion": "success",
      "detail_path": "runs/30611136043-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-07-31T07:38:50Z",
        "conclusion": "success",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "a3e47414203725c4c0df4917fd102f3b0d89706b",
        "id": "30611136043-1",
        "repository": "iii-hq/workers",
        "run_id": "30611136043",
        "started_at": "2026-07-31T06:55:52Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30611136043"
      },
      "generated_at": "2026-07-31T07:38:46.593165+00:00",
      "id": "30611136043-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-07-31",
        "url": "https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b",
        "version": "2026-07-31",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30611136043",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.012143386666666665,
            "duration_seconds": 5.558,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6934.666666666667,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.036188,
            "duration_seconds": 8.078333333333333,
            "function_call_errors": 0.0,
            "function_calls": 2.0,
            "sessions": 1.0,
            "tokens": 21540.0,
            "turns": 3.0
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 2.3619347399999997,
            "duration_seconds": 402.16366666666664,
            "function_call_errors": 2.6666666666666665,
            "function_calls": 147.0,
            "sessions": 6.333333333333333,
            "tokens": 1396783.0,
            "turns": 110.0
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
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
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.017122453333333332,
            "duration_seconds": 16.293000000000003,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7655.666666666667,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.6212315466666666,
            "duration_seconds": 67.28033333333333,
            "function_call_errors": 0.3333333333333333,
            "function_calls": 16.0,
            "sessions": 1.0,
            "tokens": 375004.3333333333,
            "turns": 16.666666666666668
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-07-31",
        "repository": "iii-hq/workers",
        "sha": "a3e47414203725c4c0df4917fd102f3b0d89706b"
      },
      "started_at": "2026-07-31T06:55:52Z",
      "status": "passed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 0,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "glm-5.2",
            "provider": "zai",
            "supports_tools": true,
            "supports_vision": false
          },
          "model": "glm-5.2",
          "passed": true,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 1.0,
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
              "total_cost_usd": 0.036430159999999996,
              "wall_time_seconds": 16.674
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.108564,
              "wall_time_seconds": 24.235
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
              "total_cost_usd": 0.05136736,
              "wall_time_seconds": 48.879
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
              "total_cost_usd": null,
              "wall_time_seconds": 1206.491
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
              "total_cost_usd": 1.8636946399999998,
              "wall_time_seconds": 201.841
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": null,
          "wall_time_seconds": 1498.12
        }
      ],
      "totals": {
        "average_score": 100.0,
        "expected_reports": 5,
        "function_calls": 495.0,
        "hard_gate_failures": 0,
        "missing_reports": 0,
        "passed_scenarios": 5,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 100.0,
        "technical_failures": 0,
        "total_cost_usd": null,
        "total_tokens": null,
        "wall_time_seconds": 1498.12
      },
      "workflow_duration_seconds": 2578.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30611136043"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-07-30T18:57:24Z",
      "conclusion": "failure",
      "detail_path": "runs/30570655128-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-07-30T18:57:24Z",
        "conclusion": "failure",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c",
        "id": "30570655128-1",
        "repository": "iii-hq/workers",
        "run_id": "30570655128",
        "started_at": "2026-07-30T18:29:55Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30570655128"
      },
      "generated_at": "2026-07-30T18:57:21.271393+00:00",
      "id": "30570655128-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-07-30",
        "url": "https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c",
        "version": "2026-07-30",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30570655128",
      "scenario_metrics": [
        {
          "averages": {
            "cost_usd": 0.020023906666666667,
            "duration_seconds": 16.386333333333333,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 6934.333333333333,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.02812872,
            "duration_seconds": 10.210666666666667,
            "function_call_errors": 0.0,
            "function_calls": 1.3333333333333333,
            "sessions": 1.0,
            "tokens": 16694.666666666668,
            "turns": 2.3333333333333335
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 1.5143800400000005,
            "duration_seconds": 325.029,
            "function_call_errors": 7.0,
            "function_calls": 100.0,
            "sessions": 6.0,
            "tokens": 889291.0,
            "turns": 83.0
          },
          "contract_fingerprint": "fnv1a32:3a5be718",
          "run_count": 1,
          "samples": {
            "cost_usd": 1,
            "duration_seconds": 1,
            "function_call_errors": 1,
            "function_calls": 1,
            "sessions": 1,
            "tokens": 1,
            "turns": 1
          },
          "scenario_id": "reactive_automation",
          "scenario_version": 1,
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.022894639999999997,
            "duration_seconds": 22.88033333333333,
            "function_call_errors": 0.0,
            "function_calls": 0.0,
            "sessions": 1.0,
            "tokens": 7607.0,
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
          "subject_id": "glm-5-2"
        },
        {
          "averages": {
            "cost_usd": 0.6644360666666667,
            "duration_seconds": 95.45166666666667,
            "function_call_errors": 0.0,
            "function_calls": 19.333333333333332,
            "sessions": 1.0,
            "tokens": 400247.0,
            "turns": 20.0
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
          "subject_id": "glm-5-2"
        }
      ],
      "source": {
        "ref": "daily/2026-07-30",
        "repository": "iii-hq/workers",
        "sha": "2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c"
      },
      "started_at": "2026-07-30T18:29:55Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 1,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "claude-sonnet-4-6",
            "provider": "anthropic",
            "supports_tools": true,
            "supports_vision": true
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.8,
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
              "total_cost_usd": 0.06007172,
              "wall_time_seconds": 49.159
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.08438616,
              "wall_time_seconds": 30.632
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
              "total_cost_usd": 0.06868392000000001,
              "wall_time_seconds": 68.641
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 80.0,
              "pass_rate": 0.0,
              "passed": false,
              "retries": 0,
              "runs": 1,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 1.5143800400000005,
              "wall_time_seconds": 325.029
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
              "total_cost_usd": 1.9933082,
              "wall_time_seconds": 286.355
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 3.7208300400000005,
          "wall_time_seconds": 759.816
        }
      ],
      "totals": {
        "average_score": 96.0,
        "expected_reports": 5,
        "function_calls": 162.0,
        "hard_gate_failures": 1,
        "missing_reports": 0,
        "passed_scenarios": 4,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 80.0,
        "technical_failures": 0,
        "total_cost_usd": 3.7208300400000005,
        "total_tokens": 2183740.0,
        "wall_time_seconds": 759.816
      },
      "workflow_duration_seconds": 1649.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30570655128"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-07-30T11:41:04Z",
      "conclusion": "failure",
      "detail_path": "runs/30537414252-1.json",
      "event": "workflow_dispatch",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-07-30T11:41:04Z",
        "conclusion": "failure",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": "70e8cdd40163d30b6a3ceadca19dd4a10bbadb46",
        "id": "30537414252-1",
        "repository": "iii-hq/workers",
        "run_id": "30537414252",
        "started_at": "2026-07-30T11:08:44Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30537414252"
      },
      "generated_at": "2026-07-30T11:41:00.077207+00:00",
      "id": "30537414252-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-07-30",
        "url": "https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46",
        "version": "2026-07-30",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30537414252",
      "source": {
        "ref": "daily/2026-07-30",
        "repository": "iii-hq/workers",
        "sha": "70e8cdd40163d30b6a3ceadca19dd4a10bbadb46"
      },
      "started_at": "2026-07-30T11:08:44Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 2,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "claude-sonnet-4-6",
            "provider": "anthropic",
            "supports_tools": true,
            "supports_vision": true
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.6,
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
              "total_cost_usd": 0.05859192,
              "wall_time_seconds": 35.151
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.07197107999999999,
              "wall_time_seconds": 14.356
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
              "total_cost_usd": 0.06760532,
              "wall_time_seconds": 55.639
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 87.5,
              "pass_rate": 0.5,
              "passed": false,
              "retries": 0,
              "runs": 2,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 4.13359844,
              "wall_time_seconds": 636.03
            },
            {
              "hard_gate_failures": 1,
              "id": "shell_coder_sandbox",
              "median_score": 77.5,
              "pass_rate": 0.5,
              "passed": false,
              "retries": 0,
              "runs": 2,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 95.0,
              "total_cost_usd": 1.08684288,
              "wall_time_seconds": 135.581
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 5.418609640000001,
          "wall_time_seconds": 876.757
        }
      ],
      "totals": {
        "average_score": 93.0,
        "expected_reports": 5,
        "hard_gate_failures": 2,
        "missing_reports": 0,
        "passed_scenarios": 3,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 60.0,
        "technical_failures": 0,
        "total_cost_usd": 5.418609640000001,
        "wall_time_seconds": 876.757
      },
      "workflow_duration_seconds": 1940.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30537414252"
    },
    {
      "actor": "ytallo",
      "attempt": 1,
      "availability": "full",
      "completed_at": "2026-07-30T07:27:56Z",
      "conclusion": "failure",
      "detail_path": "runs/30520846725-1.json",
      "event": "schedule",
      "execution": {
        "actor": "ytallo",
        "attempt": 1,
        "completed_at": "2026-07-30T07:27:56Z",
        "conclusion": "failure",
        "event": "schedule",
        "head_branch": "main",
        "head_sha": "86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a",
        "id": "30520846725-1",
        "repository": "iii-hq/workers",
        "run_id": "30520846725",
        "started_at": "2026-07-30T06:50:06Z",
        "workflow_name": "Harness E2E Daily",
        "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30520846725"
      },
      "generated_at": "2026-07-30T07:27:52.067718+00:00",
      "id": "30520846725-1",
      "lane": "daily",
      "release": {
        "registry_tag": "daily",
        "tag": "daily/2026-07-30",
        "url": "https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a",
        "version": "2026-07-30",
        "worker": "main"
      },
      "requested_runs": 3,
      "run_id": "30520846725",
      "source": {
        "ref": "daily/2026-07-30",
        "repository": "iii-hq/workers",
        "sha": "86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a"
      },
      "started_at": "2026-07-30T06:50:06Z",
      "status": "failed",
      "subjects": [
        {
          "engine_revision": "0.22.0",
          "expected_reports": 5,
          "hard_gate_failures": 1,
          "id": "glm-5-2",
          "judge": {
            "context_window": 1000000,
            "max_output_tokens": 128000,
            "model": "claude-sonnet-4-6",
            "provider": "anthropic",
            "supports_tools": true,
            "supports_vision": true
          },
          "model": "glm-5.2",
          "passed": false,
          "provider": "zai",
          "received_reports": 5,
          "report_coverage": 1.0,
          "retry_attempts": 0,
          "scenario_pass_rate": 0.8,
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
              "threshold": 80.0,
              "total_cost_usd": 0.055146280000000006,
              "wall_time_seconds": 34.208
            },
            {
              "hard_gate_failures": 0,
              "id": "persistent_state",
              "median_score": 100.0,
              "pass_rate": 1.0,
              "passed": true,
              "retries": 0,
              "runs": 3,
              "status": "passed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 0.07009779999999999,
              "wall_time_seconds": 20.798
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
              "threshold": 80.0,
              "total_cost_usd": 0.06829132,
              "wall_time_seconds": 55.083
            },
            {
              "hard_gate_failures": 1,
              "id": "reactive_automation",
              "median_score": 90.0,
              "pass_rate": 0.5,
              "passed": false,
              "retries": 0,
              "runs": 2,
              "status": "failed",
              "technical_failures": 0,
              "threshold": 90.0,
              "total_cost_usd": 3.942868459999999,
              "wall_time_seconds": 828.656
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
              "threshold": 95.0,
              "total_cost_usd": 1.7681006400000001,
              "wall_time_seconds": 259.034
            }
          ],
          "technical_failures": 0,
          "total_cost_usd": 5.904504499999999,
          "wall_time_seconds": 1197.779
        }
      ],
      "totals": {
        "average_score": 98.0,
        "expected_reports": 5,
        "hard_gate_failures": 1,
        "missing_reports": 0,
        "passed_scenarios": 4,
        "received_reports": 5,
        "report_coverage": 100.0,
        "retries": 0,
        "scenario_pass_rate": 80.0,
        "technical_failures": 0,
        "total_cost_usd": 5.904504499999999,
        "wall_time_seconds": 1197.779
      },
      "workflow_duration_seconds": 2270.0,
      "workflow_name": "Harness E2E Daily",
      "workflow_url": "https://github.com/iii-hq/workers/actions/runs/30520846725"
    }
  ],
  "last_update": "2026-08-03T20:40:46Z",
  "mode": "published",
  "repo_url": "https://github.com/iii-hq/workers",
  "retention": {
    "details": 30,
    "summaries": 100
  },
  "schema_version": 2
};
