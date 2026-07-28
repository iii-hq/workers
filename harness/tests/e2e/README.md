# Harness E2E tests

This package validates agent behavior with code-defined user scenarios running
against a real iii stack and real models. The subject uses the provider's
production system prompt; the runner never injects a test-only subject prompt.
The separately configured judge receives an evaluation-only system prompt.

This suite covers behavior and quality. Deterministic stack mechanics live in
`tests/integration/`, while registry installation and first-run behavior live
in `tests/quickstart/`.

Each scenario:

1. Builds a natural user prompt; stateful scenarios embed a unique run scope.
2. Calls `harness::send` once.
3. Waits for terminal state through `harness::status`.
4. Collects the transcript and durable metrics.
5. Applies objective hard gates and weighted criteria.
6. Optionally asks a fixed judge model to score qualitative criteria.
7. Calls `harness::teardown` and removes scenario-owned state.

Hard gates and a score threshold both apply. A persuasive response cannot pass
when the requested durable result was not produced.

Before execution, the runner resolves the exact subject and judge records with
`router::models::get`. Each scenario declares its own turn, token, and timeout
budget. Model incompatibilities are test failures because every configured
deployment is expected to execute every scenario. The timeout covers the
complete root-and-descendant session tree. After the root turn finishes, the
runner keeps polling `harness::metrics` until every descendant reaches a
terminal state.

## Scenarios

Scenario definitions, prompts, rubrics, and evaluators live under
`src/scenarios/`. The runner exposes every registered function to every
scenario through one global `allow: ["*"]` policy:

- `direct_answer`: produces a one-turn answer without tools; a judge scores its
  correctness, clarity, and instruction adherence.
- `persistent_state`: discovers and performs one exact durable state write,
  evaluated entirely in code.
- `security_review`: reviews three vulnerable snippets; a judge scores coverage,
  accuracy, remediation, and clarity.
- `reactive_automation`: orchestrates three parallel database writers,
  trigger-spawned aggregate reactors, and a single finalizer. The CI stack uses
  SQLite, so the scenario proves bounded `database::row-change` discovery and
  the namespaced `state`-trigger fallback. The evaluator queries the resulting
  tables, verifies trigger provenance from session metadata, and checks that all
  run-owned triggers were removed.

List the code-defined ids used by CI:

```bash
cargo run -p harness-e2e -- list
```

## Run against a local stack

From the `harness/` directory, run all scenarios against an already-running
stack:

```bash
cargo run -p harness-e2e -- run \
  --model claude-sonnet-4-6 \
  --provider anthropic \
  --judge-model claude-sonnet-4-6 \
  --judge-provider anthropic \
  --output target/e2e
```

Select one scenario and repeat it three times:

```bash
cargo run -p harness-e2e -- run \
  --model claude-sonnet-4-6 \
  --provider anthropic \
  --judge-model claude-sonnet-4-6 \
  --judge-provider anthropic \
  --output target/e2e \
  --scenario security_review \
  --runs 3
```

`III_URL`, `HARNESS_E2E_MODEL`, `HARNESS_E2E_PROVIDER`,
`HARNESS_E2E_JUDGE_MODEL`, and `HARNESS_E2E_JUDGE_PROVIDER` are accepted as
environment variables. Judge configuration is required only when a selected
scenario has qualitative criteria (`direct_answer` or `security_review`).
`--runs` accepts values from 1 through 20.

`--quality-advisory` or `HARNESS_E2E_QUALITY_ADVISORY=true` makes score-only
failures non-blocking. Hard-gate and technical failures remain blocking.

The judge protocol deliberately uses plain text JSON, which works across
providers without native structured-output support. The response is parsed and
validated strictly against the rubric. Invalid output gets up to two repair
attempts; a third invalid response is a `judge_error`, not a zero quality score.

## Scores and reports

Criterion weights total 100. A run passes when every hard gate passes and its
score reaches the scenario threshold. For repeated runs, the aggregate requires
at least two thirds of the runs to pass and the median score to reach the
threshold. Hard-gate and technical failures stop further repetitions for that
subject/scenario pair and always fail the aggregate. Score-only failures retain
the two-of-three tolerance used nightly.

Scenarios with a judge reference delegate every criterion score to the judge.
Scenarios without one award every criterion objectively in code. Mechanical
effects remain hard gates in both cases.

Every run has one explicit status:

- `passed`, `quality_failed`, or `hard_gate_failed` for evaluated quality;
- `subject_error`, `judge_error`, `resource_limit`, or `infrastructure_error`
  for failures that must not be interpreted as a score.

Scores and criterion awards are `null` when evaluation did not complete. A
technical failure is never converted into a zero score.

The runner writes `results.json` with:

- exact catalog-resolved subject and judge model identity and capabilities;
- effective scenario execution policy;
- judge protocol and pinned engine revision when CI supplies it;
- prompt, transcript, and `harness::metrics` for every run;
- hard-gate results and per-criterion points;
- judge attempts, token usage, and failures grouped by phase;
- subject, judge, and total model cost per run and scenario;
- median score, pass rate, and aggregate status.

Subject usage comes from `harness::metrics`; judge usage comes from
`router::complete`. When evaluation succeeds, judge usage includes the initial
response and every repair attempt. Costs are `null` when a provider or model has
no catalog pricing or complete usage data. Unknown costs are not treated as
zero.

## CI

| Workflow | Trigger | Live-model runs | Gate |
| --- | --- | ---: | --- |
| Pull-request CI | Relevant pull-request changes | 0 | Deterministic integration only |
| Harness E2E Main | Relevant push to `main` | 1 per subject/scenario | Score advisory; hard gates and technical failures blocking |
| Harness E2E Nightly | New `main` revision on schedule, or manual dispatch on `main` | 3 per subject/scenario | Median score, two-of-three pass policy, hard gates and technical failures blocking |

The reusable workflow itself is guarded to run only from `main`. It builds the
pinned engine, the SQLite-backed database worker, and only the provider workers
required by the subject matrix and fixed judge. Scenario ids come directly from
`harness-e2e list`. Each subject/scenario pair receives a fresh stack, and
repetitions run sequentially inside that job with unique table, session, and
state namespaces. At most two matrix jobs make live-model calls concurrently.

The scheduled nightly skips the live suite when the current `main` SHA already
has a successful full nightly result. A manual dispatch bypasses this
same-revision check but must target `main`.

The subject matrix comes from the `HARNESS_E2E_SUBJECTS` repository variable.
Nightly dispatch inputs may override the matrix and judge:

```json
[
  {"id":"anthropic-sonnet","model":"claude-sonnet-4-6","provider":"anthropic"},
  {"id":"openai-gpt","model":"gpt-5.4","provider":"openai"}
]
```

`HARNESS_E2E_JUDGE_MODEL` and `HARNESS_E2E_JUDGE_PROVIDER` configure the fixed
judge used for every subject.

The hosted workflows currently forward `ANTHROPIC_API_KEY` and
`OPENAI_API_KEY`. Subscription-backed `claude-code` and `openai-codex`
providers require their credential files to be provisioned securely on the
runner; the current workflow does not inject those files.

Each matrix job publishes its result for 14 days; failed jobs also publish
diagnostics for 14 days. The workflow summary shows subject, judge, and total
cost per scenario and consolidated by subject. CI currently evaluates fixed
code-defined thresholds; it does not compare scores automatically with a
previous run.

The launcher performs a clean first boot with isolated configuration, session,
queue, state, and log directories. It starts the provider workers named by the
subject and judge configuration. It executes repository binaries directly; it
does not test registry installation or `iii worker add`. Those paths are covered
by the nightly/manual [Harness quickstart validator](../quickstart/README.md),
which runs without provider credentials or model calls.

## Adding a scenario

Add a module returning `ScenarioSpec` and register its `ScenarioId`. Keep these
rules:

- prompts describe user intent and never prescribe function ids;
- declare a scenario-sized execution policy;
- objective effects are hard gates, not judge opinions;
- criterion ids are unique, weights total 100, and awarded points are bounded;
- use a hidden judge reference for qualitative scoring, otherwise award every
  criterion in the code evaluator;
- every durable resource has unconditional cleanup.

Unit tests validate the registry, rubric weights, objective awards, judge
responses and usage, transcript call normalization, blocking-gate behavior,
cost aggregation, advisory mode, and report schema.
