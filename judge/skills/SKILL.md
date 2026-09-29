---
name: judge
description: >-
  Evaluate JSON state with Noul, Choice or Score questions for ticket triage,
  classification and graded judgments through the selected judge provider
  (TypeSafe JEV by default), or cancel an active request owned by the caller.
---

# judge

`judge::evaluate` evaluates supplied state with mixed Noul, Choice and Score
questions. `judge::cancel` signals cancellation of an identified evaluation owned
by the same caller. Each call is forwarded to `judge-<provider>`:
the **Default provider** under Console Settings → Workers → judge (seeded from
`JUDGE_PROVIDER`, default `typesafe`) or a top-level `provider` field selects it. Credentials and the default model live with the
provider: for TypeSafe, configure `api_key` in the `judge-typesafe` entry under
Console Settings → Workers or supply `TYPESAFE_API_KEY` to that worker process.
An unregistered provider returns `code: "provider_unavailable"`.

## Invocation

1. Read the registered contract with
   `iii trigger engine::functions::info --json '{"function_id":"judge::evaluate"}'`.
   Use `judge::cancel` as `function_id` for that contract.
   For unfamiliar question/reply shapes,
   read [the mixed request and typed response examples](../reference.md#evaluate).
2. For evaluation, supply 1–512 evaluations with unique nonblank IDs, JSON state
   and a nonempty map of distinct question IDs. Choose primitives using the table
   below. Supply a positive `timeout_ms` within the operator's `max_timeout_ms`
   (default 300000). For queued work, also set `expires_at_unix_ms`, an absolute
   Unix deadline in milliseconds. Omit `model` to use the worker default.
   `options.attempt_timeout_ms` bounds each network attempt; retries are the
   provider's fixed policy, see [Request options and retries](../reference.md#request-options-and-retries).
3. Invoke using the positional function name and `--json`, then inspect `status`
   before reading results. Completion requires all requested typed answers or a
   reported failure; keep `stats.usage_complete` with the observed usage.

| Primitive | Question criteria | Typed answer |
| --- | --- | --- |
| `noul` | Optional `true` and/or `false` descriptions; omitted, null or empty criteria are accepted. | `noul` in `[0, 1]`. |
| `choice` | 1–255 named options. | `choice`, `probabilities`, `confidence`. |
| `score` | 2–10 ordered, non-null levels. | `score` in `[0, levels - 1]`, `probabilities`, `confidence`, `legend`. |

For all primitives, instructions may be omitted or contain strings, objects,
arrays or null. Noul and Choice descriptions accept those same forms. Score
levels accept strings, objects or arrays; **null Score levels are invalid**
(confirmed by a provider 422 and the SDK schema, despite the advanced guide).
Structured values remain structured in Score legends. Noul can describe just one
endpoint. Bare numbers and booleans are not top-level descriptions. A minimal mixed call:

```bash
iii trigger judge::evaluate --json '{
  "timeout_ms": 3000,
  "evaluations": [{
    "id": "ticket-42",
    "state": {"ticket": "Production checkout is down for all customers."},
    "questions": {
      "urgent": {
        "type": "noul",
        "instructions": "Does this ticket describe an urgent production outage?"
      },
      "department": {"type": "choice", "criteria": {"billing": null, "technical": null}},
      "severity": {"type": "score", "criteria": ["Routine", {"impact": "Production blocked"}]}
    }
  }]
}'
```

On `status: "ok"`, read `results["ticket-42"].answers`, selecting the field matching
each answer's `type`. Choice probabilities use option IDs; Score probabilities
and legends use zero-based index strings. Treat confidence as returned by the
provider; it is not a caller-defined threshold or a formula to recompute.

## Cancel an active call

Start the evaluation with a top-level `request_id` (≤128 printable
ASCII characters, unique among the caller's active calls), then invoke
`judge::cancel` with `{"request_id":"..."}` **from the same caller connection**:
ownership comes from the engine's `_caller_worker_id`, and separate CLI
invocations may get different ids. `cancelled: true` means the signal was
accepted, not that the provider rolled anything back; still await the original
call, which answers `code: "cancelled"`. Absent, finished or other callers' ids
return `cancelled: false`. Details and the bus example:
[cancellation](../reference.md#cancellation).

## Result and operational boundaries

- Success carries the effective `model`, all `results` and `stats`; errors carry
  `status: "error"`, `code`, optional `http_status`, a bounded and key-redacted
  `provider_error`, `retry_after_ms` and `stats`, with no partial answers. Bus
  invocation failures are separate. See
  [results and failures](../reference.md#handle-results-and-failures).
- `usage` counters are independently nullable; `stats.usage_complete` is false
  whenever any counter is unknown, so zero never means free.
- Deadlines, retries, permits and byte limits are in
  [limits and compatibility](../reference.md#limits-and-compatibility).
  Credentials stay in the provider's configuration or environment; requests
  never carry keys, provider URLs or extra headers.
