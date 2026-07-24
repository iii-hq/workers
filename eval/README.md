# eval

`eval` compares two prompt or system-prompt variants under the same model and
execution settings. It runs control and treatment through the iii harness,
evaluates each result with an iii function, and reports correctness together
with tokens, cost, latency, function-call, trace, and span metrics.

## Start an evaluation

```json
{
  "dimension": "prompt",
  "model": {
    "model": "codex/gpt-5.6-luna",
    "provider": "openai-codex",
    "system_prompt_strategy": "override"
  },
  "control": {
    "label": "baseline",
    "prompt": "Reply with exactly OK.",
    "system_prompt": "Follow the user request exactly."
  },
  "treatment": {
    "label": "candidate",
    "prompt": "Return exactly the text OK.",
    "system_prompt": "Follow the user request exactly."
  },
  "evaluator": {
    "function_id": "eval::assert::exact",
    "arguments": {
      "expected": "OK"
    }
  }
}
```

Call `eval::start` with the request. It returns an `evaluation_id`
immediately. Use `eval::status` for occasional progress checks and
`eval::result` for the terminal report, or bind to the `eval::completed`
trigger type.

Three runs per variant are scheduled by default. Their order alternates by
pair to reduce order bias. A candidate is eligible only when every treatment
run passes and its pass count does not regress against control. Efficiency
metrics are descriptive and never select a winner automatically.

## Public functions

- `eval::start` — validate, persist, and enqueue an evaluation.
- `eval::list` — list recent evaluations as lightweight summaries.
- `eval::status` — inspect progress without loading the full report.
- `eval::result` — read the normalized request and terminal comparison report.
- `eval::cancel` — cancel the active harness session and remaining runs.
- `eval::delete` — delete a terminal evaluation and its session indexes.
- `eval::assert::exact` — built-in deep JSON/string equality evaluator.

Evaluator functions receive the output, `harness::metrics`, run identity, and
caller-supplied arguments. They return `{ passed, score?, reason?, details? }`.
Evaluators should be deterministic and idempotent because durable delivery is
at-least-once.

## Console UI

When the console worker is running, `eval` injects an **eval** page at
`#/ext/eval-benchmarks`. The page creates prompt or system-prompt comparisons,
tracks durable progress, restores recent reports after reload, and compares
correctness, token, cost, latency, function-call, trace, and span metrics.

The model picker reads the live `router::models::list` catalog and falls back
to manual model/provider entry when the catalog is unavailable. Exact-value
evaluation is built in; any evaluator function can also be selected by id with
JSON arguments. Harness policies and output/metadata options remain collapsed
until needed.

## Boundaries

The worker intentionally does not implement an agent loop, model router,
metrics collector, trace collector, test DSL, or LLM judge. Those concerns
remain in the harness, engine, and user-provided evaluator functions.
