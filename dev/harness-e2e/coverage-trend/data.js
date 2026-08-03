window.BENCHMARK_DATA = {
  "lastUpdate": 1785741807141,
  "repoUrl": "https://github.com/iii-hq/workers",
  "entries": {
    "Harness Stack Coverage": [
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
        "date": 1785568936176,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.39,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 43.55,
            "unit": "%"
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
        "date": 1785654868162,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 43.27,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 44.96,
            "unit": "%"
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
        "date": 1785741806188,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.8,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 44.88,
            "unit": "%"
          }
        ]
      }
    ]
  }
}