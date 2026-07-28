# Harness E2E quality tests

This package runs code-defined user scenarios against a real iii stack and a
real model. It uses the provider's production system prompt: the runner never
injects a test-only system prompt.

Each scenario:

1. Builds a natural user prompt with a unique run scope.
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

- `direct_answer`: explains authentication versus authorization without tools.
- `persistent_state`: discovers and performs one exact durable state write.
- `security_review`: reviews three vulnerable snippets with a qualitative judge.
- `reactive_automation`: creates and fires a one-shot state reaction.

List the code-defined ids used by CI:

```bash
cargo run -p harness-e2e -- list
```

## Run against a local stack

Run all scenarios against an already-running stack:

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
scenario has qualitative criteria.

The judge protocol deliberately uses plain text JSON, which works across
providers without native structured-output support. The response is parsed and
validated strictly against the rubric. Invalid output gets up to two repair
attempts; a third invalid response is a `judge_error`, not a zero quality score.

## Scores and reports

Criterion weights total 100. A run passes when every hard gate passes and its
score reaches the scenario threshold. Aggregates require at least two thirds of
the runs to pass and the median score to reach the threshold. Technical failures
always fail the aggregate, while ordinary quality failures retain the
two-of-three tolerance used nightly.

Scenarios with a judge reference delegate every criterion score to the judge.
Scenarios without one award every criterion objectively in code. Mechanical
effects remain hard gates in both cases.

Every run has one explicit status:

- `passed`, `quality_failed`, or `hard_gate_failed` for evaluated quality;
- `subject_error`, `judge_error`, `resource_limit`, or `infrastructure_error`
  for failures that must not be interpreted as a score.

Scores and criterion awards are `null` when evaluation did not complete.

The runner writes `results.json` with:

- exact catalog-resolved subject and judge model identity and capabilities;
- effective scenario execution policy;
- judge protocol and pinned engine revision when CI supplies it;
- prompt, transcript, and `harness::metrics` for every run;
- hard-gate results and per-criterion points;
- judge attempts and failures grouped by phase;
- median score, pass rate, and aggregate status.

## CI

The reusable workflow builds the pinned engine and only the provider workers
needed by its subject matrix and fixed judge. It reads the scenario matrix from
`harness-e2e list` and starts one isolated stack per subject/scenario pair. At
most two live-model jobs run concurrently by default.

Trusted pull requests run every affected scenario once. The scheduled and
manual nightly workflow runs every scenario three times. The nightly subject
matrix can be supplied manually or through the `HARNESS_E2E_SUBJECTS`
repository variable:

```json
[
  {"id":"anthropic-sonnet","model":"claude-sonnet-4-6","provider":"anthropic"},
  {"id":"openai-gpt","model":"gpt-5.4","provider":"openai"}
]
```

The selected providers still need their corresponding CI credentials. Fork
pull requests and Dependabot do not receive provider credentials.

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
responses, transcript call normalization, aggregation, and report schema.
