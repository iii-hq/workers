# judge API reference

`judge` evaluates arbitrary JSON descriptions with Noul, Choice and Score questions
through a pluggable `judge-<provider>` worker; `judge-typesafe` (TypeSafe JEV) is
the default provider.
For installation and a first call, start with the [README](README.md); provider
builds, credentials and local tests live with each provider, for example
[judge-typesafe](../judge-typesafe/README.md).

## Contents

- [Configuration](#configuration)
- [Evaluate](#evaluate)
- [Request options and retries](#request-options-and-retries)
- [Handle results and failures](#handle-results-and-failures)
- [List models](#list-models)
- [Cancellation](#cancellation)
- [Limits and compatibility](#limits-and-compatibility)
- [Provider compatibility notes](#provider-compatibility-notes)

The public functions are `judge::evaluate`, `judge::models::list` and `judge::cancel`.
Each forwards to `judge-<provider>::evaluate`, `judge-<provider>::models::list`
and `judge-<provider>::cancel`; provider workers register exactly those ids
(`judge_contract::provider_function_id`) as internal functions, so
`engine::functions::list` lists them only with `include_internal: true`.
The [shared Rust contract](https://github.com/iii-hq/workers/blob/main/crates/judge-contract/src/lib.rs)
defines their request and response types. The [worker skill](skills/SKILL.md)
describes when an agent should invoke evaluation.

## Configuration

The hub requires `configuration` at startup (engine **`iii/v0.24.0-rc.2`** or
later, with `configuration::ensure`). Its entry, `judge` by default or the
worker's `III_CONFIG_NAME`, holds one field:

```yaml
provider: typesafe   # judge-<provider> worker used when a request omits provider
```

Edit it under **Settings → Workers → judge** in the Console; valid changes apply
to new calls (in-flight calls keep their snapshot, rejected reloads keep the last
valid value). `JUDGE_PROVIDER` in the hub's process environment (or `--provider`)
seeds the entry only when nothing is stored yet; it is `typesafe` unless set, so
`judge::*` reaches `judge-typesafe::*`. Any request may override the stored value
with a top-level `provider` string (lowercase letters, digits and hyphens, at
most 64 bytes). A provider that is not registered on the engine returns
`{"status":"error","code":"provider_unavailable"}`.

Credentials, default model and execution limits belong to the provider worker.
For TypeSafe, open **Settings → Workers → judge-typesafe** in the Console or read
the [judge-typesafe configuration](../judge-typesafe/README.md#configuration).
The hub never accepts an API key or provider URL in a request.

## Evaluate

This example uses the worker's default model. One evaluation can mix all three primitives:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
  "timeout_ms": 60000,
  "evaluations": [{
    "id": "ticket-42",
    "state": {"ticket": "Production checkout is down for all customers."},
    "questions": {
      "urgent": {
        "type": "noul",
        "instructions": {"question": "Does this describe an urgent production outage?"},
        "criteria": {"true": ["A critical production workflow is blocked."]}
      },
      "department": {
        "type": "choice",
        "criteria": {
          "billing": null,
          "technical": {"scope": "Bugs, outages and production failures"}
        }
      },
      "severity": {
        "type": "score",
        "instructions": ["Assess customer impact", "Higher levels mean greater impact"],
        "criteria": ["Routine", {"impact": "Degraded service"}, "Critical workflow blocked"]
      }
    }
  }]
}'
```

The question shapes are:

| `type` | `criteria` | Answer |
| --- | --- | --- |
| `noul` | Optional object containing only `true` and/or `false`. Omitted, `null` and `{}` are accepted. Either endpoint may be described independently. | `noul` between 0 and 1. |
| `choice` | Required map of 1–255 named options to descriptions. | Selected `choice`, probabilities for every option and `confidence`. |
| `score` | Required array of 2–10 non-null descriptions in order from lowest to highest. | Continuous `score` between 0 and the last level index, probabilities, `confidence` and a `legend`. |

For every primitive, `instructions` may be omitted or set to text, an object,
an array or `null`. Noul and Choice criteria descriptions accept those same four
forms. **Score levels accept text, objects or arrays; a null level is invalid.**
Descriptions need not be nonblank strings. Nested objects and arrays may contain
ordinary JSON scalar values, including null. A bare number or boolean is not a
valid top-level instruction or description. For example, these question objects
are also valid:

```json
{
  "urgent": {"type": "noul"},
  "routine": {"type": "noul", "instructions": null, "criteria": {"false": null}},
  "department": {"type": "choice", "criteria": {"billing": null, "technical": null}},
  "severity": {"type": "score", "criteria": ["Routine", ["Critical workflow blocked"]]}
}
```

Use unique, nonblank evaluation IDs and distinct question IDs. `evaluations` is
an array of 1–512 evaluations; each has a string, object or array as JSON `state`
and a nonempty question map. The outer batch is a worker facility, not an upstream
API field. Unknown request fields are rejected.

`timeout_ms` is required and must be a positive integer no greater than the
configured `max_timeout_ms` (300000 by default). An optional
`expires_at_unix_ms` is an absolute Unix deadline in milliseconds; use it when
queueing or composing calls so stale work cannot start upstream requests. The
earlier of that expiry and the relative budget wins. IDs and deadlines are
bus-only; each upstream `POST /v1/systemone` body contains only `model`, `state`
and `questions`.

The CLI's `--timeout-ms` controls how long the caller waits for the RPC result;
it is separate from the payload's worker budget. The 60000 ms example uses
`--timeout-ms 65000` to allow the worker's full budget plus transport overhead.
Set the corresponding RPC timeout when invoking from another worker too.

## Request options and retries

Evaluation and model listing accept an optional `options` object with one
field, `attempt_timeout_ms`: a positive millisecond bound on each network
attempt, including reading its body. The whole-call `timeout_ms` /
`expires_at_unix_ms` budget still covers validation, permit waits, every attempt
and backoff; an attempt timeout never extends it.

Retries are the provider's policy, not the caller's. `judge-typesafe` follows
the TypeSafe SDK defaults: two retries after the first attempt on HTTP 408, 429
and 5xx, connection failures and attempt timeouts; exponential backoff from
500 ms, capped at 5 s, minus up to 25% jitter; a server `retry-after-ms` or
`Retry-After` (delta-seconds or HTTP date) up to 60 s is honored instead.
Backoff releases the shared HTTP permit, the whole-call deadline bounds every
wait, and exhaustion returns the final provider error. Requests never carry
credentials, provider URLs or extra HTTP headers.

## Handle results and failures

Successful bus delivery can carry either typed status. An illustrative response
to the mixed request above is below; these scores and usage are example data:

```json
{
  "status": "ok",
  "model": "jev-1.13.0",
  "results": {
    "ticket-42": {
      "answers": {
        "urgent": {"type": "noul", "noul": 0.98},
        "department": {
          "type": "choice",
          "choice": "technical",
          "probabilities": {"billing": 0.03, "technical": 0.97},
          "confidence": 0.94
        },
        "severity": {
          "type": "score",
          "score": 1.9,
          "probabilities": {"0": 0.02, "1": 0.06, "2": 0.92},
          "confidence": 0.86,
          "legend": {"0": "Routine", "1": {"impact": "Degraded service"}, "2": "Critical workflow blocked"}
        }
      },
      "usage": {"input_tokens": 312, "output_tokens": 48}
    }
  },
  "stats": {
    "attempts": 1,
    "requests": 1,
    "questions": 3,
    "input_tokens": 312,
    "output_tokens": 48,
    "elapsed_ms": 250,
    "usage_complete": true
  }
}
```

Inspect `status` before reading results:

- `status: "ok"`: `model` is the effective model and `results` contains all
  requested answers. Each answer type must match its question. Choice IDs must
  belong to the supplied options; probability maps must cover exactly those
  options, or Score's zero-based level indices (`"0"`, `"1"`, …). Score legends
  use the same indices and must match the supplied descriptions, including structure.
- `status: "error"`: `code`, optional `http_status`, `provider_error`,
  `retry_after_ms` and `stats` describe failure.
  Codes are `invalid_request`, `missing_key`, `payload_too_large`, `deadline`,
  `attempt_timeout`, `cancelled`, `http`, `transport`, `invalid_response` and
  `provider_unavailable` (the selected `judge-<provider>` worker is not
  registered). `deadline` means the whole-call budget expired; `attempt_timeout`
  means a network attempt timed out. There are no partial `results`.
- Any other bus invocation failure (a timed-out or disconnected hub or provider)
  is separate from this typed envelope. Handle it at the RPC boundary as a
  failed evaluation.

For example, a provider rate-limit failure with retries disabled has this shape
(illustrative diagnostics and stats):

```json
{
  "status": "error",
  "code": "http",
  "http_status": 429,
  "provider_error": {
    "detail": {"message": "Rate limited for [REDACTED]"},
    "truncated": false
  },
  "retry_after_ms": 1000,
  "stats": {
    "attempts": 1,
    "requests": 0,
    "questions": 0,
    "input_tokens": 0,
    "output_tokens": 0,
    "elapsed_ms": 100,
    "usage_complete": false
  }
}
```

`provider_error` preserves optional JSON `detail` or a text `message`, plus
`truncated`. Error bodies are bounded to the smaller of `max_response_bytes` and
64 KiB. The worker recursively redacts its configured API key and caller header
values from returned diagnostics while retaining useful validation paths. Plain
text, malformed JSON and oversized provider error bodies still report `code:
"http"` and the HTTP status. Malformed escaped diagnostics that cannot be safely
sanitized are omitted; `truncated` identifies clipped or omitted diagnostics.
`retry_after_ms` is the parsed provider hint, when available, rather than a promise
that another attempt will occur.
If reading an error body reaches the attempt deadline after headers arrive,
the known HTTP status and retry hint still govern retries; incomplete diagnostics
are marked truncated. Expiring the whole-call deadline still stops the call.

Noul values, probabilities and confidence must be finite and within `[0, 1]`;
Score values must be finite and within their level range. Probability sums may
differ from 1 by up to 0.02 to accommodate provider rounding. Duplicate answer,
probability or legend keys, missing answers and mismatched types/IDs fail the
whole batch. The worker returns provider scores and confidence without
recalculating them. Choose a caller-specific decision threshold; JEV imposes
no eligibility threshold.

A complete, low-scoring evaluation can mean **no match**. A missing answer,
deadline or service error cannot. Discard all partial answers when any evaluation
fails. JEV performs no local fallback; each caller decides how to handle failed
evaluations and complete results that do not meet its threshold.

Each `results[id]` includes optional `usage` with independently nullable
`input_tokens` and `output_tokens`. For example,
`"usage": {"input_tokens": 12, "output_tokens": null}` records known input and
unknown output. Absent usage is also unknown. Older results without `usage`
remain readable with the shared contract.

`stats` contains `attempts`, `requests`, `questions`, `input_tokens`,
`output_tokens`, `elapsed_ms` and `usage_complete`. Missing or null provider usage
counters are **unknown**: valid answers remain usable, known counters are still
aggregated, and `usage_complete` becomes false. Negative or fractional token
counts are invalid responses. `attempts` counts actual HTTP attempts, including
retries; `requests` counts accepted, validated responses. A transport retry with
an unknown outcome leaves aggregate usage incomplete even if a later attempt
succeeds. Failure retains known usage from completed,
validated responses while discarding all answers. These observed counters are
not total billing; zero observed tokens with incomplete usage do not mean zero
cost. Record the effective model and failures alongside scores when comparing runs.

## List models

```bash
iii trigger judge::models::list --json '{}'
```

This calls `GET /v1/models` with the worker's credential snapshot. Its request
accepts optional `timeout_ms` (default **30000**), `expires_at_unix_ms`,
`options` and `request_id`. The timeout must fit the operator's `max_timeout_ms`; if
that ceiling is below 30000, supply an explicit smaller timeout. Example:

```bash
iii trigger judge::models::list --json '{"timeout_ms":5000}'
```

Success has `status: "ok"`, a `models` array and `stats`. Each model card has
string fields `name`, `description` and `release_date`; cards and aliases are
returned without filtering to locally known versions. An example reply
(illustrative values):

```json
{
  "status": "ok",
  "models": [
    {"name": "jev-example", "description": "Example model", "release_date": "2026-09-19"}
  ],
  "stats": {
    "attempts": 1,
    "requests": 1,
    "questions": 0,
    "input_tokens": 0,
    "output_tokens": 0,
    "elapsed_ms": 100,
    "usage_complete": true
  }
}
```

Failure uses the same `status: "error"`, `code`, optional `http_status`,
`provider_error`, `retry_after_ms` and `stats` envelope as evaluation. Model listing
reports no inference token usage.
It shares evaluation's four HTTP slots, queue/deadline handling, bounded response
reading and sanitized errors. The returned catalog may list aliases without every
accepted versioned ID; a missing versioned model card does not establish that the
provider rejects that model.

## Cancellation

To make an evaluation or model listing cancellable, supply a top-level
`request_id`: a nonblank string of at most 128 printable ASCII characters.
This identifies the whole call, separately from individual evaluation IDs.
`judge::cancel` accepts `{"request_id":"ticket-run-42"}` and returns
`{"status":"ok","cancelled":true}` when the cancellation signal is accepted.
It returns `cancelled: false` for an absent or completed ID. Malformed IDs or
missing required caller metadata return `{"status":"error","code":"invalid_request"}`.

Ownership comes from trusted engine `_caller_worker_id` metadata. Identified
evaluation/listing calls and cancellation require this metadata; the engine
supplies it. Ordinary calls without a request ID remain valid. An active ID is
unique across evaluation and model listing for that caller; duplicates are
rejected. Another caller cannot cancel it and receives `cancelled: false`.
Completion or dropping the call removes its registration, allowing reuse.

Use the **same persistent caller connection** for starting and cancelling work.
Separate `iii trigger` CLI processes can receive different ephemeral caller IDs,
so a second process is not a reliable way to cancel the first. Conceptually, one
long-lived worker sends these bus messages while the first call is active:

```text
caller -> judge::evaluate
  {"request_id":"ticket-run-42","timeout_ms":60000,
   "evaluations":[{"id":"ticket","state":{"message":"Sign-in is blocked"},
                   "questions":{"urgent":{"type":"noul"}}}]}

same caller -> judge::cancel
  {"request_id":"ticket-run-42"}
judge::cancel -> caller
  {"status":"ok","cancelled":true}
judge::evaluate -> caller
  {"status":"error","code":"cancelled","stats":...}
```

Cancellation can race with completion; await the original call's result too.
An accepted signal interrupts permit waits, HTTP body reads and retry backoff.
The worker aborts and drains pending tasks before returning stats, retaining known
accepted usage and discarding partial answers. `cancelled: true` means **signal
accepted**, not provider rollback, remote provider cancellation or zero cost.

The provider scopes ownership by the engine caller it sees, which through the
hub is always the hub itself. The hub therefore forwards `request_id` as
`<caller length>:<original caller>/<request_id>` (at most 512 bytes composed,
else `invalid_request`), so two callers reusing an id never collide and
a direct provider call cannot forge another caller's prefix. Cancel through the
same `provider` the call was started with; another provider answers
`cancelled: false`. With multiple hub or provider replicas, evaluation/listing
and cancellation need routing affinity to the same processes: the registry is
local to the provider process.

## Limits and compatibility

One `judge-typesafe` worker shares at most **four concurrent HTTP requests** across all
callers, including evaluation and model listing. Waiting for a
slot, retrying and reading the response consume the same deadline. HTTP redirects
are disabled. Generic defaults are 8 MiB per encoded request, 8 MiB per
response and a 300000 ms timeout ceiling, configurable by the operator. There is
no generic state-plus-largest-question byte cap. These are local transport
safeguards, not token estimates; the provider's model token limits remain
authoritative even when a body fits locally. The worker validates all answers
before returning a successful batch. Preflight validates and encodes the whole
batch without retaining its bodies. Each dispatch body is then encoded again
after acquiring a shared HTTP slot; later evaluations can use slots released
by requests waiting in retry backoff.

Existing JSON request payloads remain valid because new fields default. Existing
binaries using strict old response readers must be rebuilt against the updated
shared `judge-contract` crate before adopting this worker; additive `usage`,
`provider_error` and `retry_after_ms` fields and new error codes are otherwise
rejected by those readers. Older workers also reject the new request fields;
coordinate the worker and consumer rollout.

## Provider compatibility notes

The compatibility audit recorded on 2026-09-19 checked the API surface against the
[HTTP reference](https://docs.typesafe.ai/api),
[structured content guide](https://docs.typesafe.ai/primitives/advanced),
[question schemas](https://docs.typesafe.ai/sdk/python/api/types/questions),
[response schemas](https://docs.typesafe.ai/sdk/python/api/types/responses) and
[model catalog](https://docs.typesafe.ai/models). That audit recorded a discrepancy
confirmed by a live provider probe: the structured content guide advertises null
Score levels, but `POST /v1/systemone` rejected `score.criteria[0] = null` with HTTP 422;
the SDK schema also excludes null levels. The worker therefore rejects null Score
levels while retaining nullable instructions and Noul/Choice descriptions.
Optional descriptions and usage otherwise follow the documented forms. JEV's
[request options and retries](#request-options-and-retries),
[deadlines](#evaluate), [usage accounting](#handle-results-and-failures) and
[caller-scoped cancellation](#cancellation) are specified in this public
reference; the upstream SDK references for retry defaults are linked above.
