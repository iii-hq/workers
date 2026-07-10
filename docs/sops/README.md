# Standard operating procedures

Step-by-step guides for adding and shipping workers. On conflict with workflow
YAML, **the workflow wins** — update these docs.

| SOP | When to use |
|---|---|
| [`new-worker.md`](new-worker.md) | First read when adding any worker — naming, repo wiring, CI, release checklist |
| [`binary-worker.md`](binary-worker.md) | Scaffolding a Rust `deploy: binary` daemon (layout, functions, triggers, tests) |
| [`configuration.md`](configuration.md) | Integrating a worker with the `configuration` worker (schema-validated, hot-reloadable, shared config). Workers on the configuration worker do not ship a `config.yaml`; the registry and console own live config. |
| [`trace-hidden-functions.md`](trace-hidden-functions.md) | Marking a function's spans hidden-by-default in trace UIs (`trace_hidden: true` registration metadata) — for per-turn bookkeeping and dispatch machinery that drowns timelines |
| [`release.md`](release.md) | Cutting a version, re-running a failed release, troubleshooting publish |

## Typical flow

1. Read [`new-worker.md`](new-worker.md) §1–§6 and pick a deploy mode.
2. Follow the language-specific scaffold:
   - Rust binary → [`binary-worker.md`](binary-worker.md)
   - Node bundle → [`harness/`](../../harness/) (monorepo pattern)
3. Open a PR; CI runs per [`architecture/testing-and-ci.md`](../architecture/testing-and-ci.md).
4. Ship via [`release.md`](release.md).
