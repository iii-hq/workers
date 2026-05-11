# policy::approval_gate

Internal hook responder registered as a `durable:subscriber` listener to your configured hook topic (`agent::before_function_call` by default).

`(envelope) → { block?, reason? }` — parses `approval_required`, writes pending state, emits `approval_requested` + `approval_resolved`, and awaits `approval::resolve`. Callers orchestrate this via turns; routing workers publish the envelopes.

## When to use

- Validate the sandbox is reachable with a dev-only `iii.trigger`; production traffic comes from orchestrated hook publications.

## Notes

Pair with `provider-router` (or any publisher of the same hook topic); ensure the `skills` worker is running so these docs register for MCP clients.
