# judge-openai

OpenAI Decisions API (`POST /v1/decisions`) provider for the
[`judge`](https://github.com/iii-hq/workers/tree/main/judge) hub. It turns JSON
state and Noul, Choice or Score questions into typed, validated answers from
`gpt-6-luna`, sharing the OpenAI credential and HTTP capacity across callers,
with model listing and caller-owned cancellation. Callers go through
`judge::evaluate` (select it with `provider: "openai"` or the session's judge
picker); the `judge-openai::*` functions below are the same contract addressed
directly. They are registered as internal, so default discovery
(`engine::functions::list`, `directory::search_functions`) shows only the hub.

The Decisions API is in public beta. The reply format may still change; this
worker ignores unknown reply fields and rejects anything it cannot validate as
`invalid_response`.

## Install

```bash
iii trigger compose::add worker=judge-openai
```

Compose starts the provider with its `configuration` dependency; add the
[`judge`](https://github.com/iii-hq/workers/tree/main/judge) hub as well to call
it through `judge::evaluate`. Use engine **`iii/v0.24.0-rc.2`**, the verified
release with `configuration::ensure` used by this repository's CI. Wait for the
provider's functions to register before making a call.

## Quickstart

In the Console, open **Settings → Workers → Judge OpenAI** and set **API key**
(the form checks the saved key against `judge-openai::models::list` and fills the
**Default model** select from it), preferably as a `secret://OPENAI_API_KEY`
reference to the
[`secrets`](https://github.com/iii-hq/workers/tree/main/secrets) worker. See
[Configuration](#configuration) for the environment fallback and precedence.

Ask whether a support ticket needs urgent attention:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
  "provider": "openai",
  "timeout_ms": 60000,
  "evaluations": [{
    "id": "ticket-42",
    "state": {"ticket": "Production checkout is down for all customers."},
    "questions": {
      "urgent": {
        "type": "noul",
        "instructions": "Does this describe an urgent production outage?",
        "criteria": {"true": "A critical production workflow is blocked."}
      }
    }
  }]
}'
```

A successful response looks like this; values, usage and timing are illustrative:

```json
{
  "status": "ok", "model": "gpt-6-luna",
  "results": {"ticket-42": {
    "answers": {"urgent": {"type": "noul", "noul": 0.98}},
    "usage": {"input_tokens": 120, "output_tokens": 0}
  }},
  "stats": {"attempts": 1, "requests": 1, "questions": 1,
    "input_tokens": 120, "output_tokens": 0, "elapsed_ms": 250,
    "usage_complete": true}
}
```

Check `status` before reading answers. Errors return `status: "error"` and a
`code` with no partial results. See the hub's
[mixed Noul/Choice/Score example](https://github.com/iii-hq/workers/blob/main/judge/reference.md#evaluate),
[result and error handling](https://github.com/iii-hq/workers/blob/main/judge/reference.md#handle-results-and-failures),
[model listing](https://github.com/iii-hq/workers/blob/main/judge/reference.md#list-models) and
[cancellation](https://github.com/iii-hq/workers/blob/main/judge/reference.md#cancellation).

## How questions map to Decisions

Each evaluation is one `POST https://api.openai.com/v1/decisions` with all of its
questions; evaluations run in parallel on at most four connections per worker.
Each question is sent with `name` set to its question id, and answers are matched
back by name.

| judge | Decisions request | Answer |
| --- | --- | --- |
| `state` | `input`: a string as is; an object or array as compact JSON text | — |
| `noul` | `predicate`; `criteria.true` / `criteria.false` are appended to the instructions as `True means: …` / `False means: …` | `probability` → `noul` |
| `choice` | `choice` with `choices: [{value: key, description}]` in key order | `choice`, `probabilities` and `confidence` as returned |
| `score` | `score` with `levels: [{label}]` in order; a blank level is labelled with its index | `score`, `probabilities` keyed `"0"`…`"n-1"`, `confidence` as returned, `legend` = the request's levels |

Text is trimmed, structured instructions and descriptions are sent as compact
JSON, and blank or null instructions use a default (`Is this true of the input?`,
`Which choice best fits the input?`, `Which level best describes the input?`).
The API's probabilities, confidence and choice are never renormalized or
corrected; every answer must still pass the contract's checks (an entry for
every option, a sum within 0.02 of 1, the score in range), and a missing entry
is `invalid_response`, never filled in.

- **One-option choices.** The API takes 2 to 255 choices. A Choice with a single
  option is answered here, without HTTP, as that option with probability 1 and
  confidence 1. An evaluation made only of such questions makes no call and
  reports zero usage; a request still needs a key.
- **Refusals.** When the model declines any question, the whole call fails with
  `invalid_response`: no partial answers, no invented probability.
  `provider_error.message` reads `OpenAI refused N question(s) in evaluation
  <id>`, and `provider_error.detail` holds `refused` (at most 32 question ids,
  `truncated: true` beyond that) and that reply's `usage`. Consumers such as
  directory search and `browser::run` pause the provider for 30 seconds after
  such a failure.
- **Usage.** `stats` counts tokens and `requests` only from validated replies,
  including those accepted before another evaluation failed; refused or invalid
  replies are not counted there.
- **Models.** Only `gpt-6-luna` is supported. A request naming another model is
  `invalid_request` before any HTTP. `models::list` reads `GET /v1/models` and
  returns `gpt-6-luna` only when the key can see it, with a 922,000-token
  `context_window` (its maximum input); an empty list means the key cannot use
  the Decisions model.
- **Errors.** 408, 429 and 5xx are retried twice, honoring `Retry-After`; a hint
  longer than the remaining budget returns `http` with `retry_after_ms` at once,
  and a 429 for exhausted quota or spend limits is final. A 401 or 403 keeps
  only the OpenAI error `type` and `code`, since its message can quote part of
  the key.

The endpoint is fixed; there is no endpoint setting.

## Configuration

The Console entry defaults to `judge-openai`; set `III_CONFIG_NAME` in the worker
environment to use another entry. The form masks the API key and exposes these
defaults:

```yaml
api_key: null                  # secret://OPENAI_API_KEY, or fall back to the worker's OPENAI_API_KEY.
model: gpt-6-luna               # The only supported model.
max_request_bytes: 8388608      # Maximum JSON bytes per Decisions request.
max_response_bytes: 8388608     # Maximum bytes per response.
max_timeout_ms: 300000          # Whole-call timeout ceiling in milliseconds.
```

The key a call uses:

1. A configured `api_key`. Store the key in the `secrets` worker with
   `judge-openai` among its `consumers` and set `api_key:
   secret://OPENAI_API_KEY`. The reference is resolved through
   `secrets::resolve` at call time, kept only in memory, and re-read when
   `secrets::changed` names it, after 5 minutes, or 10 seconds after a failure.
   A reference that does not resolve returns `missing_key` with
   `provider_error.message` naming the fix, and never falls back to the
   environment. Any other `scheme://` value is rejected the same way. A literal
   key also works but should stay out of committed configuration.
2. Without `api_key`, `OPENAI_API_KEY` from the worker's environment at boot.
   This is an operator opt-in: llm-router uses the same variable name, so a
   worker started with a shared environment file answers with that key. Leave
   the variable out of this worker's environment to require the configured key.

All configuration fields hot-reload for new calls; in-flight calls keep their
snapshot, and rejected reloads keep the last valid value. Changing the process
environment requires a restart.

## Privacy

The worker never logs keys, inputs or answers, and sets
`III_DISABLE_TRACE_PAYLOADS=1` at startup so its own OpenTelemetry spans carry
no request or response payloads. Spans recorded by the `judge` hub and by the
calling workers still include the payload; set the variable on those workers
too if that matters.

## Consumer compatibility

Callers use the shared `judge-contract` request and response types. See
[shared-contract compatibility](https://github.com/iii-hq/workers/blob/main/judge/reference.md#limits-and-compatibility)
when updating existing consumers. Each caller owns its thresholds, ranking
decisions and fallback policy.

For the full API, read the hub's
[reference.md](https://github.com/iii-hq/workers/blob/main/judge/reference.md).
For source builds, UI work and local test commands, read
[CONTRIBUTING.md](CONTRIBUTING.md).
