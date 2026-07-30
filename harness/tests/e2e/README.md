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
  SQLite, so the scenario proves bounded `database::row-changed` discovery and
  the namespaced `state`-trigger fallback. The evaluator queries the resulting
  tables, verifies trigger provenance from session metadata, and checks that all
  run-owned triggers were removed.
- `shell_coder_sandbox`: performs 12 required operations across worker setup,
  `coder`, `shell`, and `sandbox`. It adds both registry workers; inspects,
  creates, updates, moves, and reads an exact Python file; executes it on the
  host; then creates, executes in, lists, and stops an isolated microVM. The
  evaluator verifies every effect, exact stdout from both environments,
  operation ordering, shutdown, and scenario-owned cleanup.

List the code-defined ids used by CI:

```bash
cargo run -p harness-e2e -- list
```

List the models registered in the running stack:

```bash
cargo run -p harness-e2e -- models
cargo run -p harness-e2e -- models --provider openai-codex
```

## Run against a local stack

From the `harness/` directory, run all scenarios against an already-running
stack:

```bash
cargo run -p harness-e2e -- run \
  --model glm-5.2 \
  --provider zai
```

The command only connects to the stack configured by `III_URL`; it never
starts, builds, or stops the engine or workers. The stack must already expose
the Harness dependencies, worker lifecycle and directory functions, plus the
selected provider and model. Scenario capabilities such as `database` are
discovered and installed by the agent when absent.

`--model` and `--provider` are the only required parameters. Results default to
`target/e2e`, and qualitative scenarios automatically use the subject model as
their judge. Use `--output`, `--judge-model`, or `--judge-provider` only to
override those defaults.

Select one scenario and repeat it three times:

```bash
cargo run -p harness-e2e -- run \
  --model claude-sonnet-4-6 \
  --provider anthropic \
  --scenario security_review \
  --runs 3
```

`III_URL`, `HARNESS_E2E_MODEL`, `HARNESS_E2E_PROVIDER`,
`HARNESS_E2E_JUDGE_MODEL`, `HARNESS_E2E_JUDGE_PROVIDER`, and
`HARNESS_E2E_OUTPUT` are accepted as environment variables. `--runs` accepts
values from 1 through 20.

`--quality-advisory` or `HARNESS_E2E_QUALITY_ADVISORY=true` makes score-only
failures non-blocking. Hard-gate and technical failures remain blocking.

The runner emits a progress heartbeat every 15 seconds with the active turn,
step, pending function count, child-session count, and descendant-tree size.
Set `--progress-interval-seconds 0` to disable it.

Transient provider and transport failures receive one retry by default.
Quality failures, hard-gate failures, cleanup failures, and resource limits are
never retried. `--technical-retries` accepts values from 0 through 3. Retried
attempts, their failure reasons, elapsed time, and cost remain visible in the
report.

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

Every execution also prints a compact terminal summary with scenario status,
median score, threshold, elapsed time, cost, failed gates, partial criteria, and
technical failure reasons. Inspect a saved artifact without a running stack:

```bash
cargo run -p harness-e2e -- report target/e2e
cargo run -p harness-e2e -- report target/e2e/results.json --verbose
```

The runner writes `results.json` with:

- exact catalog-resolved subject and judge model identity and capabilities;
- effective scenario execution policy;
- judge protocol and installed engine version when CI supplies it;
- prompt, transcript, and `harness::metrics` for every run;
- hard-gate results and per-criterion points;
- judge attempts, token usage, and failures grouped by phase;
- transient retry attempts and their original technical failures;
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

The reusable workflow itself is guarded to run only from `main`. It installs
the latest published stable `iii` release with its `iii-init` and `iii-worker`
companions, then builds the SQLite-backed database worker and only the provider
workers required by the subject matrix and fixed judge. Scenario ids come
directly from `harness-e2e list`. Each subject/scenario pair receives a fresh
stack, and repetitions run sequentially inside that job with unique table,
session, and state namespaces. At most two matrix jobs make live-model calls
concurrently.

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

To run GLM in CI, configure the repository secret `ZAI_API_KEY` and set the
subject matrix to a Z.AI provider entry. For example, the Coding Plan catalog
uses `glm-5.2`:

```json
[{"id":"glm-5-2","model":"glm-5.2","provider":"zai"}]
```

The same provider can be used as the judge with
`HARNESS_E2E_JUDGE_MODEL=glm-5.2` and
`HARNESS_E2E_JUDGE_PROVIDER=zai`. The workflow passes `ZAI_API_KEY` only to
the isolated E2E job; never put the key in `HARNESS_E2E_SUBJECTS` or a tracked
file. Pay-as-you-go Z.AI keys require configuring the provider's general API
endpoint; Coding Plan keys use the provider's default coding endpoint.

The hosted workflows currently forward `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
and `ZAI_API_KEY`. Subscription-backed `claude-code` and `openai-codex`
providers require their credential files to be provisioned securely on the
runner; the current workflow does not inject those files.

Each matrix job publishes its result for 14 days; failed jobs also publish
diagnostics for 14 days. The workflow summary shows subject, judge, and total
cost per scenario and consolidated by subject. CI currently evaluates fixed
code-defined thresholds; it does not compare scores automatically with a
previous run.

The launcher performs a clean first boot with isolated configuration, session,
queue, state, and log directories. It starts the provider workers named by the
subject and judge configuration. It executes repository binaries directly. The
`shell_coder_sandbox` scenario tests engine-side registry installation through
`worker::add`; the `iii worker add` CLI path remains covered by the nightly/manual
[Harness quickstart validator](../quickstart/README.md), which runs without
provider credentials or model calls.

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
