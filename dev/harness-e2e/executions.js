window.HARNESS_EXECUTIONS = {
  "executions": [
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
  "last_update": "2026-08-01T07:21:53Z",
  "mode": "published",
  "repo_url": "https://github.com/iii-hq/workers",
  "retention": {
    "details": 30,
    "summaries": 100
  },
  "schema_version": 2
};
