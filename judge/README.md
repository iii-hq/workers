# judge

`judge` turns JSON state and Noul, Choice or Score questions into typed,
validated answers for ticket triage, content checks and relevance judgments.
It is a provider-neutral hub: every call is forwarded to a `judge-<provider>`
worker that owns the model, credentials and transport. The default provider is
[`judge-typesafe`](../judge-typesafe/) (TypeSafe JEV); other strategies plug in
by registering the same three functions under their own `judge-<provider>` id.

## Install

```bash
iii trigger compose::add worker=judge
```

Compose starts the hub with its `judge-typesafe` dependency, which in turn needs
`configuration`. Use engine **`iii/v0.24.0-rc.2`**, the verified release used by
this repository's CI. Wait for `judge::evaluate` to register before calling it.

## Quickstart

Set the TypeSafe key in the Console under **Settings → Workers → judge-typesafe**
or supply `TYPESAFE_API_KEY` in that worker's environment (see the
[provider README](../judge-typesafe/README.md#configuration)). Then ask whether
a support ticket needs urgent attention:

```bash
iii trigger judge::evaluate --timeout-ms 65000 --json '{
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

Check `status` before reading answers. Errors return `status: "error"` and a
`code` with no partial results; `provider_unavailable` means the selected
`judge-<provider>` worker is not registered on the engine. A bus invocation
failure is handled separately. The hub adds 5 s of bus slack on top of your
`timeout_ms`; the provider enforces the deadline itself.

See the [mixed Noul/Choice/Score example](reference.md#evaluate),
[result and error handling](reference.md#handle-results-and-failures),
[model listing](reference.md#list-models) and [cancellation](reference.md#cancellation).

## Providers

The default provider is a Console setting: **Settings → Workers → judge**,
field **Default provider**. The form lists every worker currently registered as
`judge-<provider>` (with its namespace), flags a stored provider that is not
running, and clears back to the built-in `typesafe`. It hot-reloads for new calls. `JUDGE_PROVIDER` in the hub's environment (or
`--provider`) only seeds that entry on first boot; it is `typesafe` unless set.
A request may pick another provider with a top-level `provider` field:

```bash
iii trigger judge::models::list --json '{"provider": "typesafe"}'
```

`judge::evaluate`, `judge::models::list` and `judge::cancel` forward to
`judge-<provider>::evaluate`, `judge-<provider>::models::list` and
`judge-<provider>::cancel`. A new provider implements those three functions
with the [`judge-contract`](../crates/judge-contract/) request and response
types (`provider_function_id` builds the ids) and runs as its own worker; the
hub needs no change. Cancellation stays scoped to the original caller: the hub
forwards `request_id` as `<caller>/<request_id>`, so provider workers must
accept ids longer than the public 128-byte bound.

The hub holds no credentials; its configuration entry (`judge`, or
`III_CONFIG_NAME`) carries only the default provider. For the full API,
read [reference.md](reference.md); for the provider's build, configuration and
tests, read [judge-typesafe](../judge-typesafe/README.md).
