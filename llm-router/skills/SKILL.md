---
name: llm-router
description: >-
  The model gateway: how a chat request routes to a provider worker and
  streams back, the model catalog and its reconcile lifecycle, provider
  registration and the token binding that guards it, the per-provider
  identity prompt precedence, credentials, and the trigger types the router
  fires. Read this before calling router::chat/complete, before wiring a
  provider worker, and before debugging "no provider for model" or
  "registration rejected".
---

# llm-router

The router is the single gateway between agents and model providers. A
caller names `{ model, provider? }`; the router resolves the provider,
relays the generation from the provider worker's stream surface, and owns
every cross-cutting concern on the way: the model catalog, per-provider
identity prompts, output budgets, abort, and provider credentials. Provider
workers (`provider::<id>::stream`) are never called directly by consumers.

## Calling models

- `router::chat` — stream a chat completion: routes `{ model, provider? }`
  to a provider, relays assistant frames to the caller's `writer_ref`
  channel, and returns the terminal response. This is what the harness uses
  per generation.
- `router::complete` — non-streaming convenience over `router::chat`: runs
  the turn on an internal channel and returns the final assistant message
  plus usage. The `messages` it takes are full message objects — content is
  an ARRAY OF BLOCKS (`[{ "type": "text", "text": ... }]`), never a plain
  string. A string-shaped message dies before the upstream call with a
  misleading `stream ended without a terminal frame` error.
- `router::abort` — cancel an in-flight chat/complete by `request_id`;
  reports whether a live request was actually cancelled.
- `router::embed` — batch text embeddings through a provider's
  `provider::<id>::embed` surface. Names a provider or discovers the first
  embed-capable one from the live registry; one vector per input, order
  preserved.

Model selection: a bare model id resolves through the catalog; `provider`
disambiguates when two providers serve the same id. `router/no_provider_for_model`
means the catalog has no row for that id — see the catalog lifecycle below
before assuming the model name is wrong.

## The model catalog

- `router::models::list` — catalog models, optionally filtered by provider
  and/or a capability flag.
- `router::models::get` — one catalog record by `{ provider, id }`; null
  when not registered.
- `router::models::supports` — capability check (fails OPEN for unknown
  models: absence of a row is not evidence of absence of a feature).
- `router::models::budget` — the effective output budget for a model,
  resolved with the same precedence `router::chat` uses.

The catalog is fed by providers, not by config: each provider worker calls
`router::models::reconcile` (internal) to replace its slice, usually right
after registering and again on its own refresh cycles. A provider that is
registered but has not reconciled contributes zero models — the provider
appears in `router::provider::list` while its models are missing from
`router::models::list`. That split state is the signature of a provider
whose declare/refresh never completed.

## Providers: registration, binding, availability

Provider workers register with `router::provider::register` (internal),
presenting a declaration (id, optional identity prompt, optional config
schema). The FIRST registration mints a binding token; the router persists
its hash (state scope `llm-router`, key `registry`) and returns the raw
token to the provider — it exists nowhere else. Every later registration
must present that token; without it the router answers
`router/registration_rejected: provider <id> is bound to another worker;
re-binding is an operator action`.

Operational consequences, learned the hard way:

- The raw token lives in provider process memory. An upgrade or restart
  cycle that loses it leaves the provider permanently rejected — and some
  released provider binaries swallow the rejection silently, so the only
  symptom is missing catalog models.
- The recovery recipe (no release function exists yet): back up
  `state::get { scope: "llm-router", key: "registry" }`, set the value to
  `{}` (or remove the affected records), restart the llm-router worker (it
  reloads the registry at boot and fires `router::ready`), and restart any
  provider whose declare task already died. Providers then first-register
  and mint fresh tokens.
- `router::provider::list` shows `configured` (has usable credentials) and
  `available` (currently serving). Registration sets `available: true`; a
  dispatch-time function-not-found flips it back down. `router/provider_unavailable`
  on a call means the record exists but the router does not currently trust
  the worker behind it.

`router::provider::resolve` (internal) hands a provider its effective
settings (api_url, max_tokens) from the config slice;
`router::provider::update_credential` (internal) is the vault/refresh
write path.

## Configuration and the identity prompt

The router's configuration entry carries one slice per provider. A provider
that declares no custom `config_schema` gets the default slice
`{ api_key, api_url, max_tokens }` — api_key is write-only in the schema so
the console never re-displays it. Every slice additionally gets a nullable
`system_prompt` knob: the console renders it as a set/unset toggle plus a
textarea prefilled with the provider-declared prompt.

`router::system_prompt::get { provider? }` resolves the effective identity
prompt (`default_provider` when omitted): operator override when set and
non-empty, else the provider-declared prompt, else null — and the harness
falls back to its embedded default on null. Empty or null override means
"use the provider's default", so unsetting the knob is always safe. Prompt
lookup never errors; it must not take a turn down.

Config changes hot-reload (`router::on_config_changed`, internal): the
router snapshots the whole entry, so a change takes effect on the next call
without a restart.

## Trigger types the router publishes

- `router::ready` — fired when the router comes up. Provider workers bind
  their re-declare handlers to it so a router restart re-collects every
  declaration. If a provider boots BEFORE the router, its first declare
  retries with backoff, and the `ready` binding covers the restart case.
- `router::models::changed` — the catalog changed (a reconcile landed).
  Consumers that cache model lists (console pickers) invalidate on it.
- `router::provider::changed` — a provider record changed (registration,
  availability flip, credential update).

## Boundaries

- The router never stores prompts beyond the per-provider override knob and
  never stores a content catalog of its own: models come from provider
  reconciles, identity prompts from provider declarations, credentials from
  the config slices or the vault path.
- `router::route` is internal resolution plumbing; call `chat`/`complete`.
- Streaming budgets and idle guards live router-side; providers bound their
  own upstream reads. A healthy stream emits frames or pings — prolonged
  silence is treated as a dead connection, surfaced to the caller as a
  terminal error frame.

## Reading this with the harness reference

`harness/reference` documents when the harness calls `router::chat`,
`router::abort`, `router::models::*`, and `router::system_prompt::get`
during a turn. This document is the router-side contract behind those
calls.
