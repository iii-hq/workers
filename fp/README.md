# fp

Lodash-style value transforms and `fp::pipe` — worker-side pipelines that
move big values function→function over the iii bus without routing them
through the model.

## Install

    iii worker add fp

The worker is a `deploy: binary` Rust worker with no configuration.

## Why

An agent that fetches a document and re-types it into the next call's
arguments burns its context window and stalls the provider stream mid-call.
`fp::pipe` runs the whole move in one call — each step triggers a function
and its result lands in the next step's payload — so the value flows
worker→worker and the chat only ever sees per-step sizes and a preview.

While the worker is connected it also injects a usage section into the agent
system prompt via the harness `pre-generate` hook (`fp::inject-guidance`),
so the guidance is presence-gated: no fp worker, no prompt text. The
binding is one-shot at startup and relies on the engine's recoverable
triggers (iii #1962): bound before the harness is up, it parks as a pending
intent and activates when the harness registers the trigger type.

## Functions

| function | lodash | request |
|---|---|---|
| `fp::pipe` | — | `{ through: [{function, payload?, into?}], preview_chars? }` |
| `fp::get` | `_.get` | `{ value, path }` — JSON pointer; a miss names the available keys |
| `fp::pick` | `_.pick` | `{ value, paths }` — subset an object by top-level keys |
| `fp::omit` | `_.omit` | `{ value, paths }` — drop top-level keys |
| `fp::take` | `_.take` | `{ value, n }` — first n array elements / string chars |
| `fp::drop` | `_.drop` | `{ value, n }` — skip the first n |
| `fp::map` | `_.map` | `{ value, path }` — pluck a pointer from each element; misses → null |
| `fp::filter` | `_.filter` | `{ value, matches }` — keep elements matching a partial object |
| `fp::split` | `_.split` | `{ value, separator }` |
| `fp::join` | `_.join` | `{ value, separator? }` (default `,`) |
| `fp::uniq` | `_.uniq` | `{ value }` — dedupe, first occurrences win |
| `fp::size` | `_.size` | `{ value }` — array length / string chars / object key count |
| `fp::compact` | `_.compact` | `{ value }` — remove `null` elements (`0`/`false`/`""` are kept) |
| `fp::nth` | `_.nth` | `{ value, n }` — element at index; negative counts from the end |
| `fp::getOr` | `fp.getOr` | `{ value, path, default }` — pointer value, or `default` on a miss |
| `fp::flatten` | `_.flatten` | `{ value }` — unnest one level |
| `fp::sortBy` | `_.sortBy` | `{ value, path }` — stable ascending sort by a plucked pointer (`""` = the element itself) |
| `fp::reverse` | `_.reverse` | `{ value }` — reversed copy (immutable, fp-style) |
| `fp::sum` | `_.sum` / `_.sumBy` | `{ value, path? }` — total; `path` plucks the addend from each element. Empty array totals `0` |
| `fp::mean` | `_.mean` / `_.meanBy` | `{ value, path? }` — arithmetic mean; an empty array errors |
| `fp::min` | `_.min` / `_.minBy` | `{ value, path? }` — smallest NUMBER (not the element holding it); an empty array errors |
| `fp::max` | `_.max` / `_.maxBy` | `{ value, path? }` — largest NUMBER (not the element holding it); an empty array errors |
| `fp::groupBy` | `_.groupBy` | `{ value, path }` — `{ key: [elements] }` bucketed by a plucked key (`""` = the element itself) |
| `fp::countBy` | `_.countBy` | `{ value, path }` — `{ key: count }` with the same key rules |

The reductions are the worker's arithmetic: all-integer inputs fold to an
integer (so the result compares cleanly in a `fp::when` guard), a non-numeric
element errors instead of being skipped — a skipped row is a total that
silently covers less than the caller counted — and `min`/`max` return the
number rather than lodash's `…By` element so a guard can compare it directly.

`groupBy`/`countBy` add the per-key axis the reductions lack: `countBy` gives
counts in one step, and `groupBy`'s buckets each feed `fp::sum` for a per-key
total. Group keys must be a string, number, or boolean — lodash coerces a null
or object key to `"null"`/`"[object Object]"`, silently merging distinct groups
into one bucket, so those error here instead.

Transforms take their input at `value` and return `{ value }`. They deviate
from lodash where silence would thread garbage through a pipe: a type
mismatch is an error (not a silent `{}`); a `map` path matching NO element
errors naming what was available (pointers pluck stored fields — no computed
properties like `/length`; `fp::size` counts); `compact` removes only `null`
(lodash's full-falsey removal would eat legitimate `0`/`false`/`""`);
`nth` out of bounds and `size` on a non-collection error instead of returning
`undefined`/`0`; `sortBy` requires every element to carry the key with one
comparable type. Sporadic `map` misses still become `null`, like lodash —
`fp::compact` drops them.

## The pipe

```jsonc
fp::pipe { through: [
  { function: "scrapling::fetch",
    payload:  { url: "https://…", format: "markdown", main_content_only: true } },
  { function: "fp::get",  payload: { path: "/content" } },
  { function: "fp::take", payload: { n: 20000 } },
  { function: "state::set",   payload: { scope: "research", key: "article" } }
]}
```

- Step N's result lands in step N+1's payload at `into` (a JSON pointer,
  default `/value` — which is exactly where the transforms and `state::set`
  read their input, so most pipes need no `into` at all).
- A transform step threads the transformed value itself — the `{ value }`
  wrapper in the function table is only its direct-call response shape, so
  there is never a `{value}` layer to unwrap between steps.
- The FIRST step receives no threaded value: start with a producing function
  (a fetch, a `state::get`) or seed a leading transform via `payload.value`.
  An unseeded leading transform fails validation before anything runs.
- `fp::*` transform steps run inline in this worker; every other step is
  one bus trigger with a 120 s budget. The whole pipe must also fit the
  caller's own dispatch timeout on `fp::pipe`.
- 1–12 steps. The response is receipts only: `{ steps: [{function, chars}],
  value_preview }` (`preview_chars` sizes the preview, default 400, capped at
  8000 — the receipt must never become the bulk channel it replaces).
- A failing step stops the pipe; the error carries the completed-step trail.

### Boundaries

- Steps run with THIS worker's authority, not the calling agent's per-step
  dispatch policy — the `fp::pipe` call itself is the policy/approval
  surface (an approver sees the full step list). For that reason the pipe is
  not agent-callable without approval by default; the pure transforms are.
  See `iii-permissions.yaml`.
- `shell::*`/`coder::*` steps run under the harness-forwarded filesystem
  scope: the harness stamps the trusted `fs_scope` onto the `fp::pipe` call
  itself (`harness/src/filesystem_scope.rs`) and fp re-stamps it onto each
  scoped step as the last write before dispatch, overwriting anything
  authored or threaded at `/fs_scope`. Without a stamp (no session working
  directory, or a non-harness caller — cron, worker-to-worker) scoped steps
  are refused, fail-closed. A path-access rejection inside a pipe just fails
  the step; only a direct call offers the access-grant ladder.
- Statically refused as steps: `engine::register_trigger`/`engine::unregister_trigger`
  (need the harness trusted-session stamp), nested pipes, and every class the
  agent policy hard-denies — `session::*`/`approval::*`,
  `configuration::*`/`oauth::*` (credentials), `router::*`/`provider::*`
  (model spend), `harness::*`/`run::*` (turn control), `stream::*` and bus
  internals, `*::on-config-change` — because steps run with worker authority
  and must not ride past those denies. `state::*` is the deliberate
  exception: persisting the threaded value is the pipe's purpose, and the
  pipe call itself is the approval surface.
