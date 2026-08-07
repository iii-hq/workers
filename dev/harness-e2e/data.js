window.BENCHMARK_DATA = {
  "lastUpdate": 1786066626769,
  "repoUrl": "https://github.com/iii-hq/workers",
  "entries": {
    "Harness E2E Quality": [
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
          "id": "450995a8672a43416c09c73c193ab2b035dc8dd7",
          "message": "fix(providers): sync deepseek and zai locks with llm-router 1.4.2\n\nThe standalone provider lockfiles still pinned llm-router 1.4.1 after the\npath dependency was bumped, so every --locked build of provider-deepseek\nand provider-zai fails (Harness E2E Daily has been red since the bump).\nRegenerated with cargo metadata; no other entries changed.",
          "timestamp": "2026-08-06T18:26:15Z",
          "url": "https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7"
        },
        "date": 1786066625691,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "quality::deepseek-v4-flash::direct_answer::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::direct_answer::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"direct_answer\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::persistent_state::median_score",
            "value": 90,
            "range": "90–90",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::persistent_state::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"persistent_state\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::security_review::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::security_review::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_review\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::reactive_automation::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::reactive_automation::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"reactive_automation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::shell_coder_sandbox::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::shell_coder_sandbox::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"shell_coder_sandbox\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::design_tradeoff::median_score",
            "value": 100,
            "range": "94–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::design_tradeoff::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"design_tradeoff\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::security_triage::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::security_triage::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"security_triage\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":50.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::research_pipeline::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"research_pipeline\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::research_pipeline::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"research_pipeline\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::mechanical_reaction::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"mechanical_reaction\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::mechanical_reaction::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"mechanical_reaction\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::timer_wake::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"timer_wake\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::timer_wake::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"timer_wake\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::receiving_operation::median_score",
            "value": 75,
            "range": "75–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"receiving_operation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"failed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::receiving_operation::pass_rate",
            "value": 33.33333333333333,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"receiving_operation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"failed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_loop::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_loop\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_loop::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_loop\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::subagent_validation::median_score",
            "value": 100,
            "range": "70–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"subagent_validation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::subagent_validation::pass_rate",
            "value": 66.66666666666666,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"subagent_validation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::multi_subagent_validation::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"multi_subagent_validation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::multi_subagent_validation::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"multi_subagent_validation\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::subagent_validation_failure::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"subagent_validation_failure\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::subagent_validation_failure::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"subagent_validation_failure\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::custom_validator::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"custom_validator\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::custom_validator::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"custom_validator\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_self_repair::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_self_repair\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_self_repair::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_self_repair\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":80.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_scope_enforcement::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_scope_enforcement\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_scope_enforcement::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_scope_enforcement\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_chain::median_score",
            "value": 100,
            "range": "100–100",
            "unit": "points",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_chain\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::validation_chain::pass_rate",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":true,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"runs\":3,\"scenario\":\"validation_chain\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"passed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"threshold\":90.0,\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::suite::scenario_pass_rate",
            "value": 94.73684210526315,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"expected_reports\":19,\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":19,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"failed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          },
          {
            "name": "quality::deepseek-v4-flash::suite::report_coverage",
            "value": 100,
            "unit": "percent",
            "extra": "{\"engine_revision\":\"0.22.0\",\"execution\":{\"actor\":\"ytallo\",\"attempt\":1,\"event\":\"workflow_dispatch\",\"id\":\"31133924989-1\",\"run_id\":\"31133924989\",\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"},\"expected_reports\":19,\"generated_at\":\"2026-08-07T01:36:09.722560+00:00\",\"judge\":{\"context_window\":1000000,\"max_output_tokens\":128000,\"model\":\"glm-5.2\",\"provider\":\"zai\",\"supports_tools\":true,\"supports_vision\":false},\"lane\":\"daily\",\"passed\":false,\"received_reports\":19,\"release\":{\"registry_tag\":\"daily\",\"tag\":\"daily/2026-08-07\",\"url\":\"https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7\",\"version\":\"2026-08-07\",\"worker\":\"main\"},\"requested_runs\":3,\"scenario\":\"suite\",\"schema_version\":2,\"source\":{\"ref\":\"daily/2026-08-07\",\"repository\":\"iii-hq/workers\",\"sha\":\"450995a8672a43416c09c73c193ab2b035dc8dd7\"},\"status\":\"failed\",\"subject\":{\"id\":\"deepseek-v4-flash\",\"model\":\"deepseek-v4-flash\",\"provider\":\"deepseek\"},\"workflow_url\":\"https://github.com/iii-hq/workers/actions/runs/31133924989\"}"
          }
        ]
      }
    ]
  }
}