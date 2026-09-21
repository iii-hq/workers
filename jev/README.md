# jev

JEV helps workers triage tickets, check content and judge relevance by turning
JSON state and Noul, Choice or Score questions into typed, validated answers.
It shares TypeSafe credentials and HTTP capacity across callers, with model
listing and caller-owned cancellation. Any worker can call its standalone bus API.

## Install

```bash
iii trigger compose::add worker=jev
```

Compose starts JEV with its `configuration` dependency. Use engine
**`iii/v0.24.0-rc.2`**, the verified release with `configuration::ensure` used by
this repository's CI. Wait for JEV's functions to register before making a call.

## Quickstart

In the Console, open **Settings → Workers → JEV** and set **API key**, or supply
`TYPESAFE_API_KEY` in the JEV service's environment before starting it. See
[Configuration](#configuration) for precedence and reload behavior.

Ask whether a support ticket needs urgent attention:

```bash
iii trigger jev::evaluate --timeout-ms 65000 --json '{
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

A successful response looks like this; scores, usage and timing are illustrative:

```json
{
  "status": "ok", "model": "jev-1.13.0",
  "results": {"ticket-42": {
    "answers": {"urgent": {"type": "noul", "noul": 0.98}},
    "usage": {"input_tokens": 120, "output_tokens": 8}
  }},
  "stats": {"attempts": 1, "requests": 1, "questions": 1,
    "input_tokens": 120, "output_tokens": 8, "elapsed_ms": 250,
    "usage_complete": true}
}
```

Check `status` before reading answers. A Noul value is between 0 and 1; choose
your own decision threshold. Errors return `status: "error"` and a `code` with no
partial results; a bus invocation failure is handled separately. The payload
budget covers the whole call, including queueing and retries; the longer CLI
timeout leaves room for transport overhead.

See the [mixed Noul/Choice/Score example](reference.md#evaluate),
[result and error handling](reference.md#handle-results-and-failures),
[model listing](reference.md#list-models) and [cancellation](reference.md#cancellation).

## Configuration

The Console entry defaults to `jev`; set `III_CONFIG_NAME=jev-prod` in the worker
environment to use `jev-prod`. The form masks the API key and exposes these defaults:

```yaml
api_key: null                  # Fall back to the worker's TYPESAFE_API_KEY.
model: jev-1.13.0               # Default unless a call supplies its own model.
max_request_bytes: 8388608      # Maximum JSON bytes per upstream evaluation.
max_response_bytes: 8388608     # Maximum bytes per upstream response.
max_timeout_ms: 300000          # Whole-call timeout ceiling in milliseconds.
```

A nonblank configured key overrides the worker's environment key. Clearing it
restores the fallback. All configuration fields hot-reload for new calls;
in-flight calls keep their snapshot, and rejected reloads keep the last valid
value. Changing the process environment requires a restart.

The `configuration` service persists values at `./config/<configuration-id>.yaml`
with its default filesystem adapter. Keep literal API keys out of committed
project configuration. An optional `--config` file only seeds a missing entry.
See the [configuration reference](reference.md#configuration) for environment
placeholder expansion, numeric limits and form behavior.

## Consumer compatibility

Callers use the shared `jev-contract` request and response types. See
[shared-contract compatibility](reference.md#limits-and-compatibility) when
updating existing consumers. JEV supplies evaluations; each caller owns its
thresholds, ranking decisions and fallback policy.

For the full API, read [reference.md](reference.md). For source builds, UI work
and local test commands, read [CONTRIBUTING.md](CONTRIBUTING.md).
