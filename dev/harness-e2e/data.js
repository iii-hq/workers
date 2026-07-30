window.BENCHMARK_DATA = {
  "lastUpdate": 1785406718266,
  "repoUrl": "https://github.com/iii-hq/workers",
  "entries": {
    "Harness E2E Quality": [
      {
        "commit": {
          "author": {
            "name": "Ytallo",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a",
          "message": "(MOT-4277) feat(harness): add E2E benchmark dashboard\n\nPublish the daily Harness E2E benchmark dashboard with execution details, temporal trends, retained reports, and CI coverage.\\n\\nFixes MOT-4277",
          "timestamp": "2026-07-30T02:31:33Z",
          "url": "https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a"
        },
        "date": 1785406713478,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 90,
            "range": "80–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 50,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 80,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          }
        ]
      }
    ],
    "Harness E2E Efficiency and Reliability": [
      {
        "commit": {
          "author": {
            "name": "Ytallo",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a",
          "message": "(MOT-4277) feat(harness): add E2E benchmark dashboard\n\nPublish the daily Harness E2E benchmark dashboard with execution details, temporal trends, retained reports, and CI coverage.\\n\\nFixes MOT-4277",
          "timestamp": "2026-07-30T02:31:33Z",
          "url": "https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a"
        },
        "date": 1785406717056,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 34.208,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03309328,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.022053,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.055146280000000006,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 20.798,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.07009779999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.07009779999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 55.083,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.04318432,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.025106999999999997,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.06829132,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 828.656,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::subject_cost_usd",
            "value": 3.942868459999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::total_cost_usd",
            "value": 3.942868459999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 259.034,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.7681006400000001,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.7681006400000001,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::total_cost_usd",
            "value": 5.904504499999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 1197.779,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30520846725-1\",\"run_id\":\"30520846725\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T07:27:52.067718+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"86c80f7dd825af7a6fc93fd7d258b6a6bb9f116a\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30520846725\"}"
          }
        ]
      }
    ]
  }
}