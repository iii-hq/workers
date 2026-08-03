window.BENCHMARK_DATA = {
  "lastUpdate": 1785789733509,
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
      },
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
          "id": "70e8cdd40163d30b6a3ceadca19dd4a10bbadb46",
          "message": "(MOT-4277) feat(harness): treat judged scores as quality signal (#639)",
          "timestamp": "2026-07-30T10:39:32Z",
          "url": "https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46"
        },
        "date": 1785411683413,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 87.5,
            "range": "75–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 50,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 77.5,
            "range": "55–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 50,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 60,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          }
        ]
      },
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
          "id": "2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c",
          "message": "(MOT-4277) feat(harness): expand benchmark execution insights (#644)\n\n* (MOT-4277) feat(harness): expand benchmark execution diagnostics\n\n* (MOT-4277) feat(harness): add scenario efficiency history\n\n* (MOT-4277) style(harness): simplify benchmark header\n\n* (MOT-4277) feat(harness): open transcripts in chat dialog",
          "timestamp": "2026-07-30T18:22:08Z",
          "url": "https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c"
        },
        "date": 1785437859561,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 80,
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 0,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 80,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          }
        ]
      },
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
          "id": "a3e47414203725c4c0df4917fd102f3b0d89706b",
          "message": "(MOT-4107) fix(harness): recover calls safely after engine restarts\n\n* (MOT-4107) fix(harness): recover calls safely after engine restarts\n\n* (MOT-4107) test(harness): rename engine restart scenario\n\n* (MOT-4107) test(harness): cover dependency boot retry\n\n* (MOT-4107) test(harness): exercise dependency boot race\n\n* (MOT-4107) test(harness): keep boot race within retry window",
          "timestamp": "2026-07-31T03:57:05Z",
          "url": "https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b"
        },
        "date": 1785483556053,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "55–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          }
        ]
      },
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
          "id": "12700ae892089d08ec5e30e447ce92407c75e8ec",
          "message": "(MOT-4295) feat(ci): publish harness integration and e2e coverage to the metrics site (#664)\n\n* feat(ci): publish harness integration and e2e coverage to the metrics site\n\nInstrument the harness stack with LLVM source-based coverage in the daily\nE2E cycle and publish browsable HTML reports plus a line-coverage trend\nto the gh-pages metrics site under dev/harness-e2e/coverage/.\n\n- coverage input on _harness-integration.yml and _harness-e2e.yml:\n  instrumented builds (-Cinstrument-coverage with runtime counter\n  relocation), continuous-mode LLVM_PROFILE_FILE so profiles survive the\n  SIGTERM/SIGKILL stack teardown, dedicated rust-cache keys, and\n  coverage report artifacts\n- new e2e coverage job merges per-matrix profraw artifacts against the\n  packaged stack binaries; the integration job reports in-job\n- harness-e2e-daily.yml enables coverage on both suites so the reports\n  ride the workflow_run the benchmark publisher already consumes\n- harness-e2e-benchmark.yml installs the reports with replace-latest\n  semantics, emits a coverage summary manifest, and records a\n  line-coverage time series via github-action-benchmark\n- shared .github/scripts/coverage_report.sh (llvm-profdata merge +\n  llvm-cov show/report/export) reused by CI and the new local\n  'make -C harness integration-coverage' target\n- the integration process supervisor now passes LLVM_PROFILE_FILE\n  through env_clear so instrumented children keep writing profiles\n\n* refactor(benchmark-site): keep coverage summary on the dedicated landing page\n\n* refactor(benchmark-site): progressive-disclosure IA for the execution detail page\n\nThe detail page rendered everything eagerly: ~40 metric tiles and 15 run\naccordions before any interaction, an unbounded failure list, the full\nprompt expanded per run, and a multi-megabyte JSON.stringify of the whole\ndetail at page load. Reorganize it failure-first:\n\n- failure strip becomes a grouped triage: top-5 chips per failing run\n  (count badge + first message), rest behind 'Show all'; the reliability\n  KPI folds into the strip title (5 -> 4 KPIs)\n- deep links now open every collapsed ancestor <details> (hashchange +\n  delegated clicks), so triage chips land expanded on the right run\n- Overview and Configuration collapse into disclosures with one-line\n  digests; scenario cards become one-line accordions (open only when\n  failed or sole scenario) with the metric tiles moved into the body\n- expanded run bodies switch to five lazily-rendered tabs (Evaluation,\n  Usage, Prompt, Sessions, Raw); prompt clamps at 260px with expand;\n  trace/run/page JSON and the download blob serialize on demand\n- content-visibility: auto on scenario and run accordions\n\nNew additive helper HarnessExecutionData.groupRunFailures with unit\ntests; no existing module APIs changed (23/23 site tests pass).",
          "timestamp": "2026-08-01T01:50:37Z",
          "url": "https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec"
        },
        "date": 1785568930670,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "90–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 50,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 50,
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 0,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 60,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          }
        ]
      },
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
          "id": "4b056e50720252afcb05ff4f5722149b7c5db1c6",
          "message": "(MOT-3748) fix(harness): preserve registrations across engine reloads (#666)\n\n* (MOT-3748) fix(harness): preserve registrations across engine reloads\n\n* (MOT-3748) fix(harness): align queue dependency with recovery contract\n\n* (MOT-3748) fix(harness): fall back for legacy queue schemas\n\n* (MOT-3748) test(harness): update queue manifest contract\n\n* (MOT-3748) ci(harness): run quickstart after release\n\n* (MOT-3748) ci(harness): smoke mandatory dependencies",
          "timestamp": "2026-08-01T17:50:47Z",
          "url": "https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6"
        },
        "date": 1785654860657,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 72.5,
            "range": "45–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 50,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 80,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Guilherme de S. Vieira Beira",
            "username": "guibeira",
            "email": "guilherme.vieira.beira@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4b52a11e730597f7e8743e17f4a12d387943946e",
          "message": "fix(ci): build the tag being released, not the dispatch branch (#669)\n\n`actions/checkout` with no `ref` resolves the ref that started the run. On\na `push: tags:` trigger that is the tag, so the release path was correct by\naccident. On a `workflow_dispatch` it is the branch the dispatch was started\nfrom — `alpha-release.yml` dispatches with `--ref main` — while the tag\nreaches the jobs only as metadata: the version parsed from its name and\n`tag_name` on the GitHub Release.\n\nSo a dispatched release compiles one commit and labels the artifacts with\nanother commit's version, silently. `state/v0.21.4-alpha.2` shipped this way:\nthe tag points at a commit pinning `iii-sdk = \"=0.22.0-alpha.3\"`, but the\nbinary was built from main, which pins `=0.21.6`. The published artifact\ncarries the prerelease version number and none of the code it names — it\ndoes not send `namespace` on `engine::workers::register`, so no worker\nbuilt this way can join a namespaced project.\n\nEvery checkout in the release path now takes the ref being released. The\nreusable workflows get an optional `ref` input defaulting to `''`, which is\n`actions/checkout`'s own default, so nothing changes for a tag push. The\ndispatch keeps using `--ref main`: the pipeline definition should come from\nmain, only the source it compiles should follow the tag.",
          "timestamp": "2026-08-02T18:17:00Z",
          "url": "https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e"
        },
        "date": 1785741797592,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 0,
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 0,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "80–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 66.66666666666666,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 60,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "10a168a75dc2b7088d976621bd76ccaa4d146ff1",
          "message": "feat(harness): add discriminative judge-backed scenarios with anchored rubrics\n\nAdd design_tradeoff (contested scaling decision that punishes non-committal\nanswers) and security_triage (subtle real vulnerabilities plus safe decoys\nthat punish invented findings) to break the ceiling effect of the existing\nsubjective scenarios. Anchor every judge-backed criterion description with\nexplicit full/half/zero score bands to reduce judge variance.",
          "timestamp": "2026-08-02T21:00:44Z",
          "url": "https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1"
        },
        "date": 1785768282153,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "55–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::design_tradeoff::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::design_tradeoff::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::security_triage::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::security_triage::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 85.71428571428571,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":6,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 85.71428571428571,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":6,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "83966dc6acd3d53a54c73e73bc5faf1bf8a3511b",
          "message": "fix(dashboard): filter efficiency sparklines to the comparable cohort\n\nThe sparklines summed raw per-scenario averages across every reported\nscenario, so a missing report or a newly added scenario moved the line for\nstructural reasons while the delta chip on the same card honestly compared\nonly the comparable cohort. Sum the same cohort the chip uses and skip\nexecutions that lack any cohort contract instead of fabricating a dip.",
          "timestamp": "2026-08-03T20:11:12Z",
          "url": "https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b"
        },
        "date": 1785789732461,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::glm-5-2::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::median_score",
            "value": 100,
            "range": "75–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::reactive_automation::pass_rate",
            "value": 66.66666666666666,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "55–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::design_tradeoff::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::design_tradeoff::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::security_triage::median_score",
            "value": 100,
            "range": "83–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::security_triage::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::suite::scenario_pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":7,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
          },
          {
            "name": "quality::glm-5-2::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30849845674-1\",\"run_id\":\"30849845674\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T20:40:11.358516+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":7,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"83966dc6acd3d53a54c73e73bc5faf1bf8a3511b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30849845674\"}"
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
      },
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
          "id": "70e8cdd40163d30b6a3ceadca19dd4a10bbadb46",
          "message": "(MOT-4277) feat(harness): treat judged scores as quality signal (#639)",
          "timestamp": "2026-07-30T10:39:32Z",
          "url": "https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46"
        },
        "date": 1785411686053,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 35.151,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03489492,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.023697000000000003,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.05859192,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 14.356,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.07197107999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.07197107999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 55.639,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.04299932000000001,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.024606,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.06760532,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 636.03,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::subject_cost_usd",
            "value": 4.13359844,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::total_cost_usd",
            "value": 4.13359844,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 135.581,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.08684288,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.08684288,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":95.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 2,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::total_cost_usd",
            "value": 5.418609640000001,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 876.757,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30537414252-1\",\"run_id\":\"30537414252\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T11:41:00.077207+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"70e8cdd40163d30b6a3ceadca19dd4a10bbadb46\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30537414252\"}"
          }
        ]
      },
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
          "id": "2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c",
          "message": "(MOT-4277) feat(harness): expand benchmark execution insights (#644)\n\n* (MOT-4277) feat(harness): expand benchmark execution diagnostics\n\n* (MOT-4277) feat(harness): add scenario efficiency history\n\n* (MOT-4277) style(harness): simplify benchmark header\n\n* (MOT-4277) feat(harness): open transcripts in chat dialog",
          "timestamp": "2026-07-30T18:22:08Z",
          "url": "https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c"
        },
        "date": 1785437862880,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 49.159,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03486572,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.025206,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.06007172,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 30.632,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.08438616,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.08438616,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 68.641,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.04326192,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.025422,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.06868392000000001,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 325.029,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::subject_cost_usd",
            "value": 1.5143800400000005,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::total_cost_usd",
            "value": 1.5143800400000005,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 286.355,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.9933082,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.9933082,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::total_cost_usd",
            "value": 3.7208300400000005,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 759.816,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30570655128-1\",\"run_id\":\"30570655128\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"},\"expected_reports\":5,\"generated_at\":\"2026-07-30T18:57:21.271393+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"claude-sonnet-4-6\",\"provider\":\"anthropic\",\"supports_tools\":true,\"supports_vision\":true},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-30\",\"url\":\"https://github.com/iii-hq/workers/commit/2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\",\"version\":\"2026-07-30\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-30\",\"repository\":\"iii-hq/workers\",\"sha\":\"2ab08fbce06e2bed2a83fcba56a5f67c0e16f16c\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30570655128\"}"
          }
        ]
      },
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
          "id": "a3e47414203725c4c0df4917fd102f3b0d89706b",
          "message": "(MOT-4107) fix(harness): recover calls safely after engine restarts\n\n* (MOT-4107) fix(harness): recover calls safely after engine restarts\n\n* (MOT-4107) test(harness): rename engine restart scenario\n\n* (MOT-4107) test(harness): cover dependency boot retry\n\n* (MOT-4107) test(harness): exercise dependency boot race\n\n* (MOT-4107) test(harness): keep boot race within retry window",
          "timestamp": "2026-07-31T03:57:05Z",
          "url": "https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b"
        },
        "date": 1785483559669,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 16.674,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03310928,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.00332088,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.036430159999999996,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 24.235,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.108564,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.108564,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 48.879,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.04390432,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.00746304,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.05136736,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 1206.491,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 201.841,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.8636946399999998,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.8636946399999998,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 1498.12,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30611136043-1\",\"run_id\":\"30611136043\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"},\"expected_reports\":5,\"generated_at\":\"2026-07-31T07:38:46.593165+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-07-31\",\"url\":\"https://github.com/iii-hq/workers/commit/a3e47414203725c4c0df4917fd102f3b0d89706b\",\"version\":\"2026-07-31\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-07-31\",\"repository\":\"iii-hq/workers\",\"sha\":\"a3e47414203725c4c0df4917fd102f3b0d89706b\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30611136043\"}"
          }
        ]
      },
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
          "id": "12700ae892089d08ec5e30e447ce92407c75e8ec",
          "message": "(MOT-4295) feat(ci): publish harness integration and e2e coverage to the metrics site (#664)\n\n* feat(ci): publish harness integration and e2e coverage to the metrics site\n\nInstrument the harness stack with LLVM source-based coverage in the daily\nE2E cycle and publish browsable HTML reports plus a line-coverage trend\nto the gh-pages metrics site under dev/harness-e2e/coverage/.\n\n- coverage input on _harness-integration.yml and _harness-e2e.yml:\n  instrumented builds (-Cinstrument-coverage with runtime counter\n  relocation), continuous-mode LLVM_PROFILE_FILE so profiles survive the\n  SIGTERM/SIGKILL stack teardown, dedicated rust-cache keys, and\n  coverage report artifacts\n- new e2e coverage job merges per-matrix profraw artifacts against the\n  packaged stack binaries; the integration job reports in-job\n- harness-e2e-daily.yml enables coverage on both suites so the reports\n  ride the workflow_run the benchmark publisher already consumes\n- harness-e2e-benchmark.yml installs the reports with replace-latest\n  semantics, emits a coverage summary manifest, and records a\n  line-coverage time series via github-action-benchmark\n- shared .github/scripts/coverage_report.sh (llvm-profdata merge +\n  llvm-cov show/report/export) reused by CI and the new local\n  'make -C harness integration-coverage' target\n- the integration process supervisor now passes LLVM_PROFILE_FILE\n  through env_clear so instrumented children keep writing profiles\n\n* refactor(benchmark-site): keep coverage summary on the dedicated landing page\n\n* refactor(benchmark-site): progressive-disclosure IA for the execution detail page\n\nThe detail page rendered everything eagerly: ~40 metric tiles and 15 run\naccordions before any interaction, an unbounded failure list, the full\nprompt expanded per run, and a multi-megabyte JSON.stringify of the whole\ndetail at page load. Reorganize it failure-first:\n\n- failure strip becomes a grouped triage: top-5 chips per failing run\n  (count badge + first message), rest behind 'Show all'; the reliability\n  KPI folds into the strip title (5 -> 4 KPIs)\n- deep links now open every collapsed ancestor <details> (hashchange +\n  delegated clicks), so triage chips land expanded on the right run\n- Overview and Configuration collapse into disclosures with one-line\n  digests; scenario cards become one-line accordions (open only when\n  failed or sole scenario) with the metric tiles moved into the body\n- expanded run bodies switch to five lazily-rendered tabs (Evaluation,\n  Usage, Prompt, Sessions, Raw); prompt clamps at 260px with expand;\n  trace/run/page JSON and the download blob serialize on demand\n- content-visibility: auto on scenario and run accordions\n\nNew additive helper HarnessExecutionData.groupRunFailures with unit\ntests; no existing module APIs changed (23/23 site tests pass).",
          "timestamp": "2026-08-01T01:50:37Z",
          "url": "https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec"
        },
        "date": 1785568933362,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 21.326,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03308188,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.00325668,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.03633856,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 22.693,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.07033596,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.07033596,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 133.39,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 607.067,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::subject_cost_usd",
            "value": 2.5270296,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::total_cost_usd",
            "value": 2.5270296,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 325.658,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.84634192,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.84634192,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 1110.134,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30688423602-1\",\"run_id\":\"30688423602\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"},\"expected_reports\":5,\"generated_at\":\"2026-08-01T07:21:24.697528+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-01\",\"url\":\"https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec\",\"version\":\"2026-08-01\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-01\",\"repository\":\"iii-hq/workers\",\"sha\":\"12700ae892089d08ec5e30e447ce92407c75e8ec\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30688423602\"}"
          }
        ]
      },
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
          "id": "4b056e50720252afcb05ff4f5722149b7c5db1c6",
          "message": "(MOT-3748) fix(harness): preserve registrations across engine reloads (#666)\n\n* (MOT-3748) fix(harness): preserve registrations across engine reloads\n\n* (MOT-3748) fix(harness): align queue dependency with recovery contract\n\n* (MOT-3748) fix(harness): fall back for legacy queue schemas\n\n* (MOT-3748) test(harness): update queue manifest contract\n\n* (MOT-3748) ci(harness): run quickstart after release\n\n* (MOT-3748) ci(harness): smoke mandatory dependencies",
          "timestamp": "2026-08-01T17:50:47Z",
          "url": "https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6"
        },
        "date": 1785654864662,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 15.535,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03486492,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.00319908,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.038064,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 15.871,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.07089171999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.07089171999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 34.37,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.04286952,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.0072046,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.05007412,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 498.293,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::subject_cost_usd",
            "value": 4.410997439999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::total_cost_usd",
            "value": 4.410997439999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":2,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 175.243,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.80382692,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.80382692,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::total_cost_usd",
            "value": 6.373854199999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 739.312,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30736657213-1\",\"run_id\":\"30736657213\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"},\"expected_reports\":5,\"generated_at\":\"2026-08-02T07:06:40.367600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-02\",\"url\":\"https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6\",\"version\":\"2026-08-02\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-02\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b056e50720252afcb05ff4f5722149b7c5db1c6\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30736657213\"}"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Guilherme de S. Vieira Beira",
            "username": "guibeira",
            "email": "guilherme.vieira.beira@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4b52a11e730597f7e8743e17f4a12d387943946e",
          "message": "fix(ci): build the tag being released, not the dispatch branch (#669)\n\n`actions/checkout` with no `ref` resolves the ref that started the run. On\na `push: tags:` trigger that is the tag, so the release path was correct by\naccident. On a `workflow_dispatch` it is the branch the dispatch was started\nfrom — `alpha-release.yml` dispatches with `--ref main` — while the tag\nreaches the jobs only as metadata: the version parsed from its name and\n`tag_name` on the GitHub Release.\n\nSo a dispatched release compiles one commit and labels the artifacts with\nanother commit's version, silently. `state/v0.21.4-alpha.2` shipped this way:\nthe tag points at a commit pinning `iii-sdk = \"=0.22.0-alpha.3\"`, but the\nbinary was built from main, which pins `=0.21.6`. The published artifact\ncarries the prerelease version number and none of the code it names — it\ndoes not send `namespace` on `engine::workers::register`, so no worker\nbuilt this way can join a namespaced project.\n\nEvery checkout in the release path now takes the ref being released. The\nreusable workflows get an optional `ref` input defaulting to `''`, which is\n`actions/checkout`'s own default, so nothing changes for a tag push. The\ndispatch keeps using `--ref main`: the pipeline definition should come from\nmain, only the source it compiles should follow the tag.",
          "timestamp": "2026-08-02T18:17:00Z",
          "url": "https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e"
        },
        "date": 1785741802398,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 36.162,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.03377136,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.00334724,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.0371186,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 35.081,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.0963932,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.0963932,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 50.735,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.043403319999999995,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.00729764,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.05070096,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::wall_time_seconds",
            "value": 351.918,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::reactive_automation::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":1,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 372.164,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.60135308,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.60135308,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 2,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          },
          {
            "name": "efficiency::glm-5-2::suite::wall_time_seconds",
            "value": 846.06,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"schedule\",\"id\":\"30792193792-1\",\"run_id\":\"30792193792\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"},\"expected_reports\":5,\"generated_at\":\"2026-08-03T07:18:51.360492+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":5,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"4b52a11e730597f7e8743e17f4a12d387943946e\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30792193792\"}"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "10a168a75dc2b7088d976621bd76ccaa4d146ff1",
          "message": "feat(harness): add discriminative judge-backed scenarios with anchored rubrics\n\nAdd design_tradeoff (contested scaling decision that punishes non-committal\nanswers) and security_triage (subtle real vulnerabilities plus safe decoys\nthat punish invented findings) to break the ceiling effect of the existing\nsubjective scenarios. Anchor every judge-backed criterion description with\nexplicit full/half/zero score bands to reduce judge variance.",
          "timestamp": "2026-08-02T21:00:44Z",
          "url": "https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1"
        },
        "date": 1785768286662,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "reliability::glm-5-2::direct_answer::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::direct_answer::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::wall_time_seconds",
            "value": 20.046,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::subject_cost_usd",
            "value": 0.033080479999999995,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::judge_cost_usd",
            "value": 0.0037522799999999998,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::direct_answer::total_cost_usd",
            "value": 0.03683276,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::persistent_state::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::wall_time_seconds",
            "value": 24.059,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::subject_cost_usd",
            "value": 0.07008316,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::persistent_state::total_cost_usd",
            "value": 0.07008316,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_review::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::wall_time_seconds",
            "value": 47.024,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::subject_cost_usd",
            "value": 0.041741719999999996,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::judge_cost_usd",
            "value": 0.00767108,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_review::total_cost_usd",
            "value": 0.0494128,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::reactive_automation::missing_reports",
            "value": 1,
            "unit": "count",
            "extra": "{\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"missing_report\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::shell_coder_sandbox::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::wall_time_seconds",
            "value": 215.79,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::subject_cost_usd",
            "value": 1.80872368,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::judge_cost_usd",
            "value": 0,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::shell_coder_sandbox::total_cost_usd",
            "value": 1.80872368,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::design_tradeoff::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::design_tradeoff::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::design_tradeoff::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::design_tradeoff::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::design_tradeoff::wall_time_seconds",
            "value": 101.982,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::design_tradeoff::subject_cost_usd",
            "value": 0.05314991999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::design_tradeoff::judge_cost_usd",
            "value": 0.013474279999999998,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::design_tradeoff::total_cost_usd",
            "value": 0.0666242,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_triage::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_triage::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_triage::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::security_triage::missing_reports",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_triage::wall_time_seconds",
            "value": 52.782,
            "unit": "seconds",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_triage::subject_cost_usd",
            "value": 0.04441252,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_triage::judge_cost_usd",
            "value": 0.010671199999999999,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "efficiency::glm-5-2::security_triage::total_cost_usd",
            "value": 0.05508372,
            "unit": "USD",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"passed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::hard_gate_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":6,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::technical_failures",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":6,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::retry_attempts",
            "value": 0,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":6,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          },
          {
            "name": "reliability::glm-5-2::suite::missing_reports",
            "value": 1,
            "unit": "count",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"30820138874-1\",\"run_id\":\"30820138874\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"},\"expected_reports\":7,\"generated_at\":\"2026-08-03T14:42:24.617600+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":6,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-03\",\"url\":\"https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1\",\"version\":\"2026-08-03\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-03\",\"repository\":\"iii-hq/workers\",\"sha\":\"10a168a75dc2b7088d976621bd76ccaa4d146ff1\"},\"status\":\"failed\",\"subject\":{\"id\":\"glm-5-2\",\"model\":\"glm-5.2\",\"provider\":\"zai\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/30820138874\"}"
          }
        ]
      }
    ]
  }
}