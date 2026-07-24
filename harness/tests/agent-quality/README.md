# Harness prompt evaluation

This package compares two harness subjects against the same real-model
scenarios. It supports exactly two experiments:

- Same system prompt with two different models.
- Same model with two different system prompts.

Every scenario follows one execution path:

1. Build the scenario prompt.
2. Call `harness::send` once.
3. Wait for `harness::turn-completed`.
4. Read `harness::metrics` and the root transcript.
5. Evaluate whether the requested result works.

The common runner owns steps 2–4. A scenario defines only its prompt and its
result evaluation. Evaluations may inspect the transcript or call existing iii
functions to test durable output.

## Compare subjects

```bash
cargo run -p harness-agent-quality --bin harness-prompt-eval -- \
  --control subjects/luna.json \
  --treatment subjects/sol.json \
  --runs 3 \
  --output target/prompt-eval \
  --max-total-tokens 100000
```

The comparison type is inferred from the subject files. Every setting other
than the model or system-prompt contents must remain equal. Ambiguous,
identical, or multi-variable comparisons fail before a model call.

Use repeatable `--scenario` arguments for a focused comparison:

```bash
cargo run -p harness-agent-quality --bin harness-prompt-eval -- \
  --control path/to/control.json \
  --treatment path/to/treatment.json \
  --runs 3 \
  --output target/prompt-eval \
  --scenario plain_response \
  --scenario security_review
```

The output contains `comparison.json` and one self-contained `results.json`
for every control and treatment run. The comparison reports pass counts and
median execution, token, function-call, error, cost, trace, span, and trace
duration metrics per scenario when the engine's in-memory observability
exporter is available.
The treatment passes the correctness gate only when every treatment run passes
and no scenario regresses relative to the control.

## Limits

Every control and treatment run uses one shared limit policy. Limits are not
part of a subject, so a comparison still changes only the model or system
prompt.

Execution limits constrain the run while it is active:

- `--scenario-timeout-seconds` defaults to `300`; expiry calls `harness::stop`.
- `--invocation-timeout-seconds` defaults to `120` for each iii invocation.
- `--max-turns` defaults to `20`.
- `--max-output-tokens-per-call` defaults to `8192`.
- `--max-total-tokens` defaults to `100000` and means input plus output tokens
  over the complete root-and-descendant session tree. The harness reserves
  each call before dispatch and stops the turn before this budget can be
  exceeded.
- `--max-cost-usd` is optional. The harness reserves cost before dispatch and
  fails closed when any selected model lacks catalog pricing.

Evaluation limits use `harness::metrics` after the requested result has been
evaluated:

- `--max-function-call-errors` defaults to `0`.
- `--max-error-spans` is optional because it requires the engine's in-memory
  observability exporter.

Token and cost limits are also checked against `harness::metrics` after the
run, while the remaining evaluation limits fail the completed scenario.
Configuring an optional gate without its required metric fails closed as an
evidence error. Every `results.json` and `comparison.json` records the
effective limit policy.

## Scenarios

The current scenarios are:

- `plain_response`: checks an exact text response without tools.
- `single_function`: checks one durable state mutation.
- `security_review`: checks structured reasoning from the final response.
- `triggered_work`: checks the durable result of reactive work.

`single_function` and `triggered_work` exercise multiple internal model turns,
but each scenario still starts with exactly one runner call to `harness::send`.

To add a scenario, add a module that returns a `ScenarioSpec` containing:

- the prompt;
- private data needed by the evaluation;
- an async evaluation function.

Do not reproduce send, completion, metrics, transcript, timeout, or report
handling in scenario modules.

## Subjects

Subject files resolve `system_prompt_path` relative to the subject file:

```json
{
  "schema_version": "1",
  "subject_id": "baseline",
  "model": "resolved-model-id",
  "provider": "provider-route",
  "system_prompt_path": "system-prompt.md",
  "system_prompt_strategy": "override",
  "thinking_level": "low"
}
```

For a model comparison, both subjects use the same provider and exact prompt
contents but different `model` values. For a system-prompt comparison, they use
the same model and provider but different prompt contents.

## Run one subject

The lower-level runner is useful for developing or diagnosing a scenario:

```bash
cargo run -p harness-agent-quality -- \
  --subject subjects/luna.json \
  --output target/agent-quality \
  --scenario single_function
```
