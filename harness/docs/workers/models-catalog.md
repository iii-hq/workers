# models-catalog

Model capabilities knowledge base on the iii bus (`models::*`). Populated
exclusively by provider discovery — there is no baked-in seed.

## Purpose

A small catalogue answering "given a (provider, model_id), what does it
support?" Each `Model` record carries display name, context window, max
output tokens, plus boolean capability flags (`supports_thinking`,
`supports_xhigh`, `supports_tools`, `supports_vision`, `supports_cache`)
and optional `thinking_budgets` / `transports` / `pricing` fields.

Entries live in iii state under scope `models`, **one key per provider**
whose value is a `Model[]` array. Providers write the full list via
`models::reconcile` (typically through each provider's
`provider::<name>::refresh_models` or `reconcileModels` in discovery).
Reads return only what providers have registered — there is no embedded
fallback.

## Registered functions

- `models::list` — List models, optionally filtered by provider or capability. Returns only models registered by providers.
- `models::get` — Look up a single model by (provider, model_id); null when no provider has registered it.
- `models::supports` — Check whether a provider-registered model supports a capability (false when unknown).
- `models::reconcile` — Replace a provider's catalog with a `Model[]` in one state write (the only write path).

## Triggers

None.

## State keys

| Scope | Key shape | Value |
|---|---|---|
| `models` | `<provider>` (e.g. `anthropic`) | `Model[]` |

Only provider-id keys and `Model[]` values are supported. Pre-batch per-model
keys (`models:<provider>:<id>`) are not read. If upgrading from older storage,
clear scope `models` once and run `provider::<id>::refresh_models` for each
configured provider.

## Capability strings

`models::supports` and the `capability` filter on `models::list` accept
these strings:

| String | Maps to |
|---|---|
| `thinking` | `supports_thinking` |
| `thinking:low`, `thinking:medium`, `thinking:high` | `supports_thinking` (level enum only matters for `xhigh`) |
| `thinking:xhigh` | `supports_xhigh` |
| `tools` | `supports_tools` |
| `vision` | `supports_vision` |
| `cache` | `supports_cache` |

Unknown strings return `supported: false` from `models::supports` and
match no models from `models::list`.

## Configuration

The worker reads no top-level config keys.

## Dependencies

From
[src/models-catalog/iii.worker.yaml](harness/src/models-catalog/iii.worker.yaml):
`iii-state ^0.11.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/models-catalog/main.ts](harness/src/models-catalog/main.ts) | Binary entry point (`iii-models-catalog`). |
| [src/models-catalog/register.ts](harness/src/models-catalog/register.ts) | Registers list/get/supports/reconcile handlers. |
| [src/models-catalog/types.ts](harness/src/models-catalog/types.ts) | `Model`, `Pricing`, `ThinkingBudgets`, `Capability`, `ListFilter`, `parseCapability`, `supportsModel`. |
| [src/models-catalog/state.ts](harness/src/models-catalog/state.ts) | State-only `listFromState` / `getFromState` / `providerStateKey`. |
| [src/models-catalog/handlers/list.ts](harness/src/models-catalog/handlers/list.ts) | `models::list` handler. |
| [src/models-catalog/handlers/get.ts](harness/src/models-catalog/handlers/get.ts) | `models::get` handler. |
| [src/models-catalog/handlers/supports.ts](harness/src/models-catalog/handlers/supports.ts) | `models::supports` handler. |
| [src/models-catalog/handlers/reconcile.ts](harness/src/models-catalog/handlers/reconcile.ts) | `models::reconcile` handler. |
| [src/models-catalog/iii.worker.yaml](harness/src/models-catalog/iii.worker.yaml) | Worker manifest. |
