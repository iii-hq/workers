---
type: index
name: fp
description: >-
  Move and reshape big values between iii functions without routing them
  through the model — fp::pipe worker-side pipelines plus pure
  lodash-style transforms (get, pick, omit, take, drop, map, filter, split,
  join, uniq).
---

# fp

The fp worker moves bulk data worker→worker. `fp::pipe` runs a short
pipeline in one call: each step triggers a function and its result lands in
the next step's payload at `into` (default `/value`); the caller receives
per-step sizes and a preview — never the value itself. The ten `fp::*`
transforms mirror their lodash namesakes, take input at `value`, and run
inline as pipe steps, threading the transformed value itself onward — their
`{ value }` response wrapper appears only on standalone direct calls (fine
for small values).

## When to Use

- Fetch a document and persist it without reading it: `scrapling::fetch` →
  `fp::get {path: "/content"}` → `fp::take {n: 20000}` → `state::set`,
  all inside one `fp::pipe`.
- Move any big function result into another function's arguments — never
  re-type a large value into a call by hand.
- Reshape a list worker-side: `fp::filter {matches}` → `fp::map
  {path}` → `fp::uniq` as pipe steps.
- Top-N worker-side: `fp::sortBy {path}` → `fp::reverse` → `fp::take {n}`
  as pipe steps.
- Probe a bulk result's shape cheaply: a `fp::get` step with a wrong
  `path` fails naming the keys that were available.
- Slice or subset a small value directly: `fp::take`, `fp::pick`,
  `fp::omit`, `fp::split`, `fp::join`.

## Boundaries

- Pipe steps run with the fp worker's authority, not the calling agent's
  per-step dispatch policy, so `fp::pipe` is not agent-callable without
  approval by default; the pure transforms are (see iii-permissions.yaml).
- Refused as steps: `shell::*`/`coder::*`, trigger control
  (`engine::register_trigger`/`engine::unregister_trigger`), nested pipes,
  and the agent-policy hard-denied classes (`session::*`/`approval::*`,
  credentials via `configuration::*`/`oauth::*`, model spend via
  `router::*`/`provider::*`, turn control via `harness::*`/`run::*`, bus
  internals) — call those directly; `state::*` stays allowed on purpose.
- The first pipe step receives no threaded value — start with a producer
  (a fetch, `state::get`) or seed a leading transform via `payload.value`.
- 1–12 steps, 120 s per bus step; the whole pipe must fit the caller's
  dispatch timeout. Transforms error on type mismatches instead of silently
  threading `{}`; `map` errors when a path matches no element (pointers
  pluck stored fields, not computed properties like `/length`).

## Functions

- `fp::pipe` — `{ through: [{function, payload?, into?}], preview_chars? }`
  → `{ steps: [{function, chars}], value_preview }`.
- `fp::get` / `fp::pick` / `fp::omit` — pointer extract / key
  subset / key drop.
- `fp::take` / `fp::drop` — first-n / skip-n on strings and arrays.
- `fp::map` / `fp::filter` / `fp::uniq` — pluck / partial-object
  match / dedupe on arrays.
- `fp::split` / `fp::join` — string ↔ array.
- `fp::size` / `fp::nth` — count a collection / element at index
  (negative counts from the end).
- `fp::getOr` — pointer extract with a `default` on a miss.
- `fp::compact` / `fp::flatten` — drop `null` elements / unnest one level.
- `fp::sortBy` / `fp::reverse` — stable ascending sort by pointer
  (`""` = the element itself) / reverse.
