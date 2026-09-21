---
name: judge
description: >-
  Evaluate JSON state with Noul, Choice or Score questions for ticket triage,
  classification and graded judgments through the selected judge provider
  (TypeSafe JEV by default), list provider models, or cancel an active request
  owned by the caller.
---

# judge

`judge::evaluate` evaluates supplied state with mixed Noul, Choice and Score
questions. `judge::models::list` lists provider models using the same credentials
and transport. `judge::cancel` signals cancellation of an identified evaluation or
listing owned by the same caller. Each call is forwarded to `judge-<provider>`:
the **Default provider** under Console Settings → Workers → judge (seeded from
`JUDGE_PROVIDER`, default `typesafe`) or a top-level `provider` field selects it. Credentials and the default model live with the
provider: for TypeSafe, configure `api_key` in the `judge-typesafe` entry under
Console Settings → Workers or supply `TYPESAFE_API_KEY` to that worker process.
An unregistered provider returns `code: "provider_unavailable"`.

## Invocation

1. Read the registered contract with
   `iii trigger engine::functions::info --json '{"function_id":"judge::evaluate"}'`.
   Use `judge::models::list` or `judge::cancel` as `function_id` for those contracts.
   For unfamiliar question/reply shapes,
   read [the mixed request and typed response examples](../reference.md#evaluate).
2. For evaluation, supply 1–512 evaluations with unique nonblank IDs, JSON state
   and a nonempty map of distinct question IDs. Choose primitives using the table
   below. Supply a positive `timeout_ms` within the operator's `max_timeout_ms`
   (default 300000). For queued work, also set `expires_at_unix_ms`, an absolute
   Unix deadline in milliseconds. Omit `model` to use the worker default.
   When selecting headers, retries or an attempt timeout, read
   [Request options and retries](../reference.md#request-options-and-retries).
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

To list models, invoke `iii trigger judge::models::list --json '{}'`. This uses
`GET /v1/models`; the default timeout is 30000 ms, with an optional smaller or
larger `timeout_ms` within the operator ceiling, and optional `expires_at_unix_ms`.
Both evaluation and listing accept `options` and an optional `request_id`.
Success supplies `models` containing `name`, `description` and `release_date`,
plus `stats`. A catalog may contain aliases without every accepted versioned ID.
Model listing claims no inference token usage.

## Cancel an active call

1. Use one persistent caller connection to start evaluation or model listing with
   a top-level `request_id`: a nonblank string of at most 128 printable ASCII
   characters. The engine must supply trusted `_caller_worker_id` metadata for
   identified calls and cancellation. Choose an ID not already active for that
   caller across either function; duplicates are rejected.
2. While it is active, invoke `judge::cancel` from that same caller with
   `{"request_id":"your-request-id"}`. Separate CLI invocations can have different
   ephemeral caller IDs. Use the [persistent caller bus example](../reference.md#cancellation).
3. Inspect `status` and await the original result. `{"status":"ok","cancelled":true}`
   means signal accepted, not provider rollback. Absent, completed or other callers'
   IDs return `cancelled: false`; malformed IDs or missing caller metadata return
   `invalid_request`. Completion/drop frees the ID for reuse. Retain known usage
   from a cancelled call and discard its partial answers.

Direct Rust clients use a local namespace by default, shared by clones.
Multiple hub or provider replicas require routing affinity so start and cancel reach the same
process. Ordinary evaluation/listing calls without IDs remain valid.

## Result and operational boundaries

- The provider evaluates supplied evidence; callers own tool discovery and fallback
  policies.
- Evaluation success supplies effective `model`, full `results` and `stats`.
  Evaluation and listing report typed errors as `status: "error"`, `code`, optional
  `http_status`, bounded `provider_error`, `retry_after_ms` and `stats`. Provider
  diagnostics recursively redact the worker key and caller header values; see
  [error envelopes](../reference.md#handle-results-and-failures) for their byte bound
  and truncation flag. Handle RPC failures separately. A failure returns
  no partial answers; a complete low score can be a valid no-match.
- Each `results[id].usage` has independently nullable `input_tokens` and
  `output_tokens`. Missing/null usage is unknown: valid answers survive, known
  counters remain aggregated and `usage_complete` is false. Negative/fractional counters are
  invalid. Transport retries with unknown outcomes keep aggregate usage incomplete
  even after success. Observed zero with incomplete usage does not mean zero cost.
- All calls share four upstream slots per worker. Whole-call deadlines include
  queue waits, retries and backoff; `options.attempt_timeout_ms` separately bounds
  each network attempt. Generic calls default to two retries; partial options
  inherit [documented defaults](../reference.md#request-options-and-retries).
  Redirects are disabled. Generic request/response limits default to
  8388608 bytes each (`max_request_bytes`, `max_response_bytes`); the configurable
  timeout ceiling is `max_timeout_ms`. These positive integer settings are local
  safeguards; provider token limits still apply.
- Credentials stay in the provider's configuration or environment. Neither
  function accepts caller-supplied keys or provider URLs.
- Caller headers cannot override credentials, host, content type, framing or
  hop-by-hop headers. Rebuild strict old response readers with the shared contract
  before adopting the new worker's additive metadata and error codes.

The TypeSafe provider reads `TYPESAFE_API_KEY` from its own process; restart
`judge-typesafe` after changing that environment. A configured key takes
precedence and clearing it restores the fallback. Provider selection and the
`provider` override are in [Configuration](../reference.md#configuration).
