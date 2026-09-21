# judge-typesafe

TypeSafe JEV provider for the [`judge`](../judge/) hub. It turns JSON state and
Noul, Choice or Score questions into typed, validated answers, sharing TypeSafe
credentials and HTTP capacity across callers, with model listing and
caller-owned cancellation. Callers normally go through `judge::evaluate`; the
`judge-typesafe::*` functions below are the same contract addressed directly.

## Install

```bash
iii trigger compose::add worker=judge-typesafe
```

Compose starts the provider with its `configuration` dependency; installing
`judge` pulls it in automatically. Use engine
**`iii/v0.24.0-rc.2`**, the verified release with `configuration::ensure` used by
this repository's CI. Wait for JEV's functions to register before making a call.

## Quickstart

In the Console, open **Settings → Workers → judge-typesafe** and set **API key**, or supply
`TYPESAFE_API_KEY` in the JEV service's environment before starting it. See
[Configuration](#configuration) for precedence and reload behavior.

Ask whether a support ticket needs urgent attention:

```bash
iii trigger judge-typesafe::evaluate --timeout-ms 65000 --json '{
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

See the hub's [mixed Noul/Choice/Score example](../judge/reference.md#evaluate),
[result and error handling](../judge/reference.md#handle-results-and-failures),
[model listing](../judge/reference.md#list-models) and
[cancellation](../judge/reference.md#cancellation). Through the hub, `request_id`
arrives here as `<caller>/<request_id>`, which is why this worker accepts ids of
up to 512 bytes.

## Configuration

The Console entry defaults to `judge-typesafe`; set `III_CONFIG_NAME=judge-typesafe-prod` in the worker
environment to use `judge-typesafe-prod`. The form masks the API key and exposes these defaults:

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
The form retains unknown values when editing other fields and shows errors
returned by the configuration service; a `${TYPESAFE_API_KEY}` value expands in
the configuration service's environment instead of this process.

## Consumer compatibility

Callers use the shared `judge-contract` request and response types. See
[shared-contract compatibility](../judge/reference.md#limits-and-compatibility)
when updating existing consumers. JEV supplies evaluations; each caller owns its
thresholds, ranking decisions and fallback policy.

For the full API, read the hub's [reference.md](../judge/reference.md). For
source builds, UI work and local test commands, read [CONTRIBUTING.md](CONTRIBUTING.md).
