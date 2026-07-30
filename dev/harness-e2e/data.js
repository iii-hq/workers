window.BENCHMARK_DATA = {
  "lastUpdate": 1785406714635,
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
    ]
  }
}