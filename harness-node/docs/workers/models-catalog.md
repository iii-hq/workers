# models-catalog

Model capabilities knowledge base on the iii bus (`models::*`). State-first;
seeds from a baked-in baseline when state is empty.

## Purpose

A small catalogue answering "given a (provider, model_id), what does it
support?" Each `Model` record carries display name, context window, max
output tokens, plus boolean capability flags (`supports_thinking`,
`supports_xhigh`, `supports_tools`, `supports_vision`, `supports_cache`)
and optional `thinking_budgets` / `transports` / `pricing` fields. The
catalogue is state-first: reads go to iii state under scope `models`,
prefix `models:`; if state is empty the worker falls back to (and seeds
state from) the JSON file embedded at
[src/models-catalog/models.json](harness-node/src/models-catalog/models.json).

The seed happens lazily at `register()` time and never blocks boot.
Operators can layer their own entries on top with `models::register`.

## Registered functions

- `models::list` — List models, optionally filtered by provider or capability. Reads from iii state; falls back to the embedded seed when state is empty.
- `models::get` — Look up a single model by (provider, model_id). State-first.
- `models::supports` — Check whether a model supports a capability. State-first.
- `models::register` — Write a model to iii state under `models:<provider>:<id>`.

## Triggers

None. The seed runs once on register and writes one `state::set` per
embedded model.

## State keys

| Scope | Key shape | Value |
|---|---|---|
| `models` | `models:<provider>:<model_id>` | `Model` record. |

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

The worker reads no top-level config keys. The state-request timeout is
hard-coded to 5 s in
[src/models-catalog/state.ts](harness-node/src/models-catalog/state.ts)
(`DEFAULT_STATE_CONFIG.state_request_timeout_ms`).

## Dependencies

From
[src/models-catalog/iii.worker.yaml](harness-node/src/models-catalog/iii.worker.yaml):
`iii-state ^0.11.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/models-catalog/main.ts](harness-node/src/models-catalog/main.ts) | Binary entry point (`iii-models-catalog`). |
| [src/models-catalog/register.ts](harness-node/src/models-catalog/register.ts) | Kicks off `seedStateIfEmpty` and registers the four handlers. |
| [src/models-catalog/types.ts](harness-node/src/models-catalog/types.ts) | `Model`, `Pricing`, `ThinkingBudgets`, `Capability`, `parseCapability`, `supportsModel`. |
| [src/models-catalog/catalog.ts](harness-node/src/models-catalog/catalog.ts) | `loadEmbeddedCatalog` reads `models.json` and caches the parsed list. |
| [src/models-catalog/state.ts](harness-node/src/models-catalog/state.ts) | `seedStateIfEmpty` + state-first `listFromStateOrSeed` / `getFromStateOrSeed`. |
| [src/models-catalog/models.json](harness-node/src/models-catalog/models.json) | Baked-in baseline catalogue. |
| [src/models-catalog/handlers/list.ts](harness-node/src/models-catalog/handlers/list.ts) | `models::list` handler. |
| [src/models-catalog/handlers/get.ts](harness-node/src/models-catalog/handlers/get.ts) | `models::get` handler. |
| [src/models-catalog/handlers/supports.ts](harness-node/src/models-catalog/handlers/supports.ts) | `models::supports` handler. |
| [src/models-catalog/handlers/register.ts](harness-node/src/models-catalog/handlers/register.ts) | `models::register` handler. |
| [src/models-catalog/iii.worker.yaml](harness-node/src/models-catalog/iii.worker.yaml) | Worker manifest. |
