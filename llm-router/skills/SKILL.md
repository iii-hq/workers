---
name: llm-router
description: >-
  What the API reference cannot tell you about the router: how the model
  catalog is fed and why a registered provider can still serve zero models,
  the provider token binding and its recovery runbook, the identity prompt
  precedence, and the rebind semantics behind router::ready. For the
  function surface itself, read the contracts with engine::functions::info.
---

# llm-router

The router is the single gateway between agents and model providers:
callers name `{ model, provider? }`, the router resolves and relays, and
provider workers are never called directly. The function surface is in the API
reference (https://workers.iii.dev/workers/llm-router?tab=api, or live via
`engine::functions::info`); this document covers only the
semantics that span functions and the failure modes no schema shows.

## The catalog is provider-fed, and the split state that implies

Models enter the catalog when a provider worker reconciles its slice —
usually right after registering, again on its own refresh cycles. Config
never adds models. Two consequences:

- A provider can be REGISTERED yet contribute ZERO models: it appears in
  the provider list while its models are missing from the catalog. That
  split state is the diagnostic signature of a declare/refresh that never
  completed — not of a wrong model name.
- `router/no_provider_for_model` therefore has two distinct causes: the id
  genuinely does not exist, or the provider that serves it is in the split
  state above. Check the provider list before trusting the error at face
  value.

Capability checks fail OPEN for unknown models: absence of a catalog row is
not evidence a feature is unsupported.

## Provider binding: the token, the rejection, the recovery

A provider's FIRST registration mints a binding token; the router persists
its hash (state scope `llm-router`, key `registry`) and returns the raw
token, which then exists only in the provider's process memory. Every later
registration must present it, else:

    router/registration_rejected: provider <id> is bound to another worker;
    re-binding is an operator action

Operational facts learned from a live 0.21 to 0.22 migration:

- An upgrade or restart cycle that loses the in-memory token leaves the
  provider permanently rejected, and some released provider binaries
  swallow the rejection without a log line — the only symptom is the split
  state above.
- There is no release function yet; the operator action is manual: back up
  the registry state value, clear the affected records (or set the value to
  `{}`), restart the llm-router worker — it reloads the registry at boot
  and fires `router::ready` — and restart any provider whose declare task
  already died. Providers then first-register and mint fresh tokens.
- `configured` and `available` are independent flags: registration sets a
  provider available; a dispatch-time function-not-found flips it back
  down. `router/provider_unavailable` means the record exists but the
  router does not currently trust the worker behind it.

## The identity prompt precedence

`router::system_prompt::get` resolves the per-provider identity prompt:
operator override when set and non-empty, else the provider-declared
prompt, else null — and on null the harness falls back to its embedded
default. An empty or null override means "use the provider's default", so
unsetting the console knob is always safe. Prompt lookup never errors by
design: an identity miss must not take a turn down.

Every provider's config slice carries the override knob; a provider that
declares no custom config schema gets the default credentials slice with a
write-only api key (the console never re-displays it). Config hot-reloads:
changes apply on the next call, no restart.

## Rebind semantics: router::ready

The router fires `router::ready` when it comes up. Provider workers bind
their re-declare handlers to it, which is what makes a router restart
self-healing: on ready, every connected provider re-registers and
re-reconciles. A provider that boots BEFORE the router retries its declare
with backoff, and the ready binding covers the restart case. The other two
published events — catalog changed and provider changed — exist for
consumers that cache model or provider lists and must invalidate on change.

## Boundaries

- The router stores no content catalog of its own: models come from
  provider reconciles, identity prompts from declarations plus the one
  override knob, credentials from config slices or the vault path.
- Streaming budgets and idle guards live router-side; providers bound their
  own upstream reads. A healthy stream emits frames or pings — prolonged
  silence is treated as a dead connection and surfaced as a terminal error
  frame, not a hang.

`harness/reference` documents where a harness turn touches the router; this
is the router-side contract behind those calls.
