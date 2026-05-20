# Playground & Backend Contract

This document is the source of truth for two things:

1. **The streaming contract** every `ChatBackend` honors. The chat surface
   only knows about this contract, never about a specific provider — so the
   internals can churn freely as long as the contract holds.
2. **The Playground page** that exercises the contract through a catalog of
   scenarios (slow streams, errors, multi-function runs, markdown stress, etc).

If you're swapping the mock for a real backend, this is the file to read
first. If a scenario fails after your refactor, the contract has drifted —
either fix the backend or update both the scenario and this doc together.

## Quickstart

The Playground (and the Examples spec sheet) ship behind a build-time flag.

```bash
# dev: flag is on by default (set in .env.development)
npm run dev
# open #/playground
```

In dev, the header has a `chat / playground / examples` toggle. Pick a
scenario from the left rail, send any message, and watch the right-hand
event log mirror every `StreamEvent` the backend yields.

## The streaming contract

Every backend implements:

```ts
export interface ChatBackend {
  readonly id: string
  stream(
    prompt: string,
    mode: Mode,
    model: ModelId,
    opts?: ChatStreamOptions,
  ): AsyncGenerator<StreamEvent>
}
```

The generator yields `StreamEvent`s in this taxonomy:

| event              | payload                                   | when                                       |
|--------------------|-------------------------------------------|--------------------------------------------|
| `thought-start`    | —                                         | a thought block is opening                  |
| `thought-token`    | `{ token: string }`                       | one chunk of the thought body               |
| `thought-end`      | `{ durationMs: number }`                  | the thought block has finished              |
| `fcall-start`      | `{ functionId, input, pendingApproval? }` | a function call begins (or awaits approval) |
| `fcall-end`        | `{ output, durationMs }`                  | the function call resolved                  |
| `assistant-token`  | `{ token: string }`                       | one chunk of the assistant body             |
| `assistant-end`    | —                                         | the assistant body has finished             |

### Ordering rules

1. A `thought-start` is always followed by zero or more `thought-token`
   events and then exactly one `thought-end`. Backends never interleave a
   thought with another phase.
2. An `fcall-start` is always paired with exactly one matching `fcall-end`.
   Multiple `fcall-*` pairs may appear back-to-back; the consumer resets its
   pointer between pairs (see `multi-function-agent`).
3. `assistant-token`s may be empty or whitespace-only; the consumer treats
   them as opaque appends.
4. `assistant-end` is the terminal event for that turn. After yielding it,
   the generator returns.
5. A turn may legally contain *no* thought block, *no* function calls, or
   *no* assistant body. The minimum legal turn is a single `assistant-end`
   on an empty body.

### Abort semantics

The caller passes an `AbortSignal` via `opts.signal`. Backends MUST:

- Check `signal.aborted` between async waits and stop iterating early.
- Treat the signal as advisory: emitting a partial sequence is fine, the
  consumer's `finally` cleans up streaming flags. No special "aborted"
  event is required.
- Optionally throw a `DOMException('...', 'AbortError')` to signal that the
  backend itself initiated the abort. The chat surface treats AbortError as
  benign; any other thrown error is logged.

### Error semantics

There are two distinct shapes for failures:

- **Soft errors** (the call ran but didn't succeed) ride on `fcall-end`'s
  `output` field. The convention is `{ error: { kind, message, ... } }`.
  The `error-on-fcall` scenario asserts this. Backends should prefer this
  shape over thrown exceptions for anything the user can act on.
- **Hard errors** (the stream itself broke) are thrown out of the generator.
  The chat surface logs them and returns the surface to "ready" state.

## The seam

```mermaid
graph TD
  ChatView["ChatView (UI)"]
  Backend["ChatBackend interface"]
  Mock["mockBackend (lib/backend/mock.ts)"]
  Real["realBackend (lib/backend/real.ts) - stub today"]
  Scenarios["scenarioBackend (pages/Playground/scenarios)"]

  ChatView -->|consumes| Backend
  Backend -.implements.- Mock
  Backend -.implements.- Real
  Backend -.implements.- Scenarios
```

The seam is `chat-app/src/lib/backend/`:

- [`types.ts`](src/lib/backend/types.ts) — the contract types: `StreamEvent`,
  `ChatStreamOptions`, `ChatBackend`.
- [`mock.ts`](src/lib/backend/mock.ts) — three canned bodies, jittered token
  delays, abort-aware sleeps. Imported only when `VITE_PLAYGROUND` is on.
- [`real.ts`](src/lib/backend/real.ts) — stub that throws
  `'backend not configured'`. Replace its body with your provider; preserve
  the `ChatBackend` shape and you're done.
- [`index.ts`](src/lib/backend/index.ts) — `getDefaultBackend()` picks one or
  the other based on the build-time flag.

The chat page imports `getDefaultBackend()` once at module load and passes
it to `ChatView` as a prop. Nothing else in the app depends on the choice.

## Scenarios

Each scenario is a `ChatBackend` exported from
[`pages/Playground/scenarios/`](src/pages/Playground/scenarios/). The
registry in [`scenarios/index.ts`](src/pages/Playground/scenarios/index.ts)
groups them and exposes them to the picker.

| id                  | group         | what it asserts                                                              |
|---------------------|---------------|------------------------------------------------------------------------------|
| `happy-plan`        | happy paths   | thought + assistant body, no function calls.                                 |
| `happy-ask`         | happy paths   | assistant body only, no thought, no function calls.                          |
| `happy-agent`       | happy paths   | thought + one function call + assistant body.                                |
| `multi-function-agent`  | agent         | three sequential `fcall-*` pairs — exercises pointer reset in `ChatView`.    |
| `pending-approval`  | agent         | `pendingApproval: true` lifecycle: pending → running → done.                 |
| `abort-mid-thought` | failure modes | half a thought, then `throw new DOMException('...', 'AbortError')`.          |
| `error-on-fcall`    | failure modes | `fcall-end.output = { error: { kind: 'rate_limited' } }`.                    |
| `slow-tokens`       | timing        | ~200ms between assistant tokens — watch for cursor flicker.                  |
| `fast-tokens`       | timing        | ~5ms between assistant tokens — stresses the patch path.                    |
| `long-markdown`     | markdown      | ~4kB body: headings, lists, tables, fenced code in 3 langs.                  |
| `markdown-stress`   | markdown      | nested lists, footnotes, autolinks, hard breaks, busy GFM tables.            |

This list is the regression suite. Wiring a real backend without breaking
any of these scenarios means the chat surface continues to render correctly.

## Agent turn layouts (exploration)

When a turn fans out into multiple `fcall-*` pairs (today's `multi-function-agent`
scenario, tomorrow's real harness), the current renderer stacks each call as
its own bordered box in
[`MessageList`](src/components/chat/MessageList.tsx). Five functions is five
boxes; the user's prompt and the eventual assistant reply scroll way out of
view, and there's no visual hint that the calls belong to the same turn.

Every proposal below is a **pure render concern** over the existing
`Message[]`. None of them require changes to the `StreamEvent` contract,
the `Message` schema, or the scenario catalog — only to how `MessageList`
arranges what it already has. They differ on how aggressively they push
toward the "message always present, function activity on the side, both equally
important" direction.

### Proposal A — Grouped function accordion (shipping in this round)

Consecutive `function-call` messages collapse into one bordered accordion
in the message flow. Single-call runs stay rendered as a standalone
[`FunctionCallMessage`](src/components/chat/FunctionCallMessage.tsx).

```
┌─[chat column]──────────────────────┐
│ you ›                              │
│   how do i probe worker-7?         │
│                                    │
│ ▸ thought briefly                  │
│                                    │
│ ┌────────────────────────────────┐ │
│ │ ▸ ● running function 2 of 3:       │ │  ← header is live; one
│ │     ƒ engine::info             │ │    box, one accordion
│ └────────────────────────────────┘ │
│                                    │
│ › assistant reply…                 │
└────────────────────────────────────┘

expanded body:
  ▾ ● running function 2 of 3: ƒ engine::info
  ─────────────────────────────────────────
    ✓  ƒ engine::list   for 450ms
    ●  ƒ engine::info   running…
    ·  ƒ engine::echo   queued
```

- **Header label** (priority order): `permission to run ƒ <name>` ·
  `running function i of N: ƒ <name>` · `N functions failed` · `ran N functions for
  <sum>ms`.
- **StatusDot tone** reflects worst-current-state: `warn` (pending),
  `accent` + pulse (running), `alert` (any errored output), `ink` (done).
- **Default open** while any child is pending/running/errored; respects
  user toggle once everything settles.
- **Files touched**: new
  [`FunctionCallGroup.tsx`](src/components/chat/FunctionCallGroup.tsx),
  new `embedded` prop on `FunctionCallMessage`, grouping helper inside
  `MessageList`.
- **Pros**: minimal change, no layout work, immediate noise reduction,
  fully revertible (storage stays flat).
- **Cons**: partial win — the user's prompt still scrolls away if the
  group expands and the reply is long. Doesn't satisfy "side by side".

#### Live backend

The same renderer runs unchanged over `realBackend`'s output. Two
contract-honoring details connect the harness to the group's header:

- **Durations are server-side.** `function_execution_end` carries a
  required `duration_ms: number` field, captured by the orchestrator
  between the matching `function_execution_start` and end emits and
  persisted on `ExecutedEntry` so resumed runs replay the original
  timing instead of ~0ms. [`translate.ts`](src/lib/backend/translate.ts)
  reads `event.duration_ms` directly — no client-side timing map. The
  group's `ran N functions for <sum>ms` header sums these per-child
  durations. Approval wait time is excluded by design: pending calls
  discard their timer, and the resumed step starts a new one.
- **Errors land on the canonical shape.** When the harness sets
  `is_error: true`, [`translate.ts`](src/lib/backend/translate.ts) wraps
  the `FunctionResult` into `{ error: { kind: 'function_error',
  message, details, content } }` (matching the `approval_resolved` deny
  branch and the [Error semantics](#error-semantics) contract). The
  group's `isErrorOutput()` then picks it up and bumps the
  `N functions failed` counter.

### Proposal B — Right-hand function activity pane

Add a third column to the chat surface (after `Sidebar | Chat`).
Individual `function-call` messages stop rendering in the main column;
instead a small `ran N functions ↗` chip appears inside the assistant message
that linked them. The pane is a persistent vertical timeline of function
calls for the active conversation.

```
┌─sidebar──┬─chat column─────────┬─function pane─────────┐
│ convo 1  │ you ›               │ functions (3)         │
│ convo 2  │   how do i probe…?  │ ─────────────────  │
│ * convo3 │                     │ ✓ engine::list    │
│          │ ▸ thought briefly   │   450ms           │
│          │                     │ ● engine::info    │
│          │ › assistant reply…  │   running…        │
│          │   ran 3 functions ↗     │ · engine::echo    │
│          │                     │   pending         │
└──────────┴─────────────────────┴───────────────────┘
              ↑                              ↑
        prose stays clean             live timeline never
        and full width                leaves the viewport
```

- **Files touched**: new `FunctionActivityPane.tsx`, layout split in
  `Chat.tsx` (or a new `ChatLayout`), filter in `MessageList` to skip
  standalone fcalls, new inline chip in `Message.tsx`'s assistant
  renderer, hash anchor or shared `useFunctionPane` hook so the chip can
  scroll the pane to the right call.
- **Pros**: most literal answer to "both equally important, on the side".
  Function activity is always visible without competing with prose.
- **Cons**: largest implementation cost. Needs collapse behavior for
  narrow viewports. Splits the surface in half on mobile.
- **Best-fit follow-up to this round.**

### Proposal C — Two-column turn block

Each agent turn (thought → fcalls → assistant) becomes one bordered block
split into two columns: function activity on the left, reply on the right.
The block is self-contained, so the reply is always adjacent to the
activity that produced it. Pre-turn user messages and post-turn replies
stay as flat rows above/below.

```
┌────────────────────────────────────────────┐
│ you ›                                      │
│   how do i probe worker-7?                 │
└────────────────────────────────────────────┘

┌─agent turn────────────────────────────────┐
│ ▸ thought briefly · ran 3 functions · 1.3s    │
├─functions (left)───────┬─reply (right)────────┤
│ ✓ engine::list 450 │ ## probe complete    │
│ ✓ engine::info 500 │ ran three functions…     │
│ ✓ engine::echo 350 │ all returned cleanly │
└────────────────────┴──────────────────────┘
```

- **Files touched**: new `AgentTurnBlock.tsx`, turn-segmentation helper
  inside `MessageList` (walks `Message[]` and packages each turn that
  contains at least one fcall), layout tweaks to keep the block readable
  at the current `max-w-[760px]`.
- **Pros**: clear turn boundaries; reply and functions sit at equal weight
  inside one frame; no full-layout change to the surface.
- **Cons**: needs turn segmentation logic (a turn is "everything from the
  end of the last user/assistant boundary up to and including the next
  `assistant-end`"). Stretches the chat column width assumption.

### Proposal D — Gutter trace markers

Function calls compress to status dots in the left gutter of the message
column (like a code-review gutter). The assistant reply gets the full
column width, uninterrupted. Hovering or clicking a dot reveals
input/output in a popover.

```
┌─[chat column]──────────────────────────────┐
│       you ›                                │
│         how do i probe worker-7?           │
│                                            │
│       ▸ thought briefly                    │
│                                            │
│  ●─── › assistant reply…                   │  ← marker hovered:
│  │       opens popover:                    │
│  │       ┌────────────────────┐            │
│  │       │ ƒ engine::list     │            │
│  │       │   input: {}        │            │
│  │       │   ✓ 450ms          │            │
│  │       └────────────────────┘            │
│  ●───                                      │
│  ●───                                      │
└────────────────────────────────────────────┘
```

- **Files touched**: new `GutterTrace.tsx` (absolute-positioned column
  anchored to the assistant message), popover primitive (none in
  `components/ui` today — would need a floating-ui wrapper), filter in
  `MessageList` to skip standalone fcalls.
- **Pros**: prose gets primacy without losing access to function detail.
  Visually quiet during long agent turns.
- **Cons**: discovery is poor — markers are easy to miss. Popover
  primitive is net-new. Doesn't render the *current* activity in a
  glance the way the pane (B) does.

### Proposal E — Sticky turn header + inline function strip

While a turn is in flight, the user's prompt becomes sticky at the top
of the message viewport and a compact horizontal chip strip of function calls
sits directly under it. Once the turn ends, the sticky behavior releases
and the block joins the normal flow.

```
┌─sticky─[user prompt stays anchored]────────┐
│ you ›                                      │
│   how do i probe worker-7?                 │
├────────────────────────────────────────────┤
│ [✓ list 450] [● info…] [· echo]            │  ← horizontal chip strip
└────────────────────────────────────────────┘
┌─[scrolls below the sticky]─────────────────┐
│ ▸ thought briefly                          │
│                                            │
│ › assistant reply…                         │
│                                            │
└────────────────────────────────────────────┘
```

- **Files touched**: `MessageList.tsx` grows a sticky region driven by
  `isStreaming`, new `FunctionStripChip` component, optional thoughts move
  below the sticky.
- **Pros**: cheapest path to "message always present" — the intent never
  leaves the viewport during the turn.
- **Cons**: sticky regions stack weirdly with the existing scrollable
  composer + header. Chip strip is a new affordance the rest of the
  surface doesn't echo.

### Comparison

| proposal                         | preserves contract | message in view             | impl cost |
| -------------------------------- | ------------------ | --------------------------- | --------- |
| A. grouped accordion             | yes                | partial (less push)         | S         |
| B. right-hand function pane          | yes                | yes (separate column)       | L         |
| C. two-column turn block         | yes                | yes (within turn block)     | M         |
| D. gutter trace markers          | yes                | yes (full-width reply)      | M         |
| E. sticky turn header + strip    | yes                | yes (anchored at top)       | M         |

**This round ships A.** It's the cheapest cut that removes the immediate
noise from `multi-function-agent`-style fan-outs, lands without any layout
change, and leaves the bigger redesigns documented and unblocked.
**Proposal B is the strongest match for the "side by side, both equally
important" direction** and is the most likely follow-up.

## Flag plumbing

A single env var, `VITE_PLAYGROUND`, controls visibility:

| file                | value     | effect                                              |
|---------------------|-----------|-----------------------------------------------------|
| `.env.development`  | `1`       | dev defaults: Playground + Examples + mock shipped. |
| `.env.production`   | empty     | prod defaults: pages and mock tree-shaken.          |

The flag is consumed in three places:

1. [`src/App.tsx`](src/App.tsx) — `lazy()`-wraps the Playground and Examples
   pages and only registers the routes when the flag is truthy.
2. [`src/hooks/use-hash-route.ts`](src/hooks/use-hash-route.ts) — `#/playground`
   and `#/examples` resolve to `chat` when the flag is off, so old deep links
   degrade gracefully.
3. [`src/lib/backend/index.ts`](src/lib/backend/index.ts) — `getDefaultBackend()`
   returns the mock when the flag is on, otherwise the real backend stub.

Vite/Rolldown inlines `import.meta.env.VITE_PLAYGROUND` as a literal at
build time. The dead branch (and every transitive import) is then dropped
by tree-shaking.

### Verifying a prod build is clean

```bash
npm run build
# expect: a single index-*.js, no Playground-*.js or Examples-*.js chunks

# none of these strings should appear in dist/assets/*.js:
grep -E '"happy-(plan|ask|agent)"|"abort-mid-thought"|"long-markdown"' dist/assets/*.js && echo "LEAK" || echo "clean"
```

A flag-on build (`VITE_PLAYGROUND=1 npm run build`) emits separate
`Playground-*.js` and `Examples-*.js` chunks — that's the expected dev/staging
layout, not the production layout.

## Adding a new scenario

Three steps. Average size is 30–60 lines.

1. Create `src/pages/Playground/scenarios/<id>.ts`. Use the helpers from
   [`scenarios/helpers.ts`](src/pages/Playground/scenarios/helpers.ts):

   ```ts
   import { makeBackend, streamAssistant, streamThought } from './helpers'

   export const myScenario = makeBackend(
     'my-scenario',
     async function* (_prompt, _mode, _model, opts) {
       yield* streamThought('reasoning…', { signal: opts?.signal })
       yield* streamAssistant('answer…', { signal: opts?.signal })
     },
   )
   ```

2. Register it in
   [`scenarios/index.ts`](src/pages/Playground/scenarios/index.ts):

   ```ts
   import { myScenario } from './my-scenario'

   export const SCENARIOS: PlaygroundScenario[] = [
     // ...
     {
       id: 'my-scenario',
       label: 'my scenario',
       description: 'one sentence about what this asserts.',
       group: 'happy paths',
       preferredMode: 'agent',
       backend: myScenario,
     },
   ]
   ```

3. Add a row to the table in [the Scenarios section](#scenarios) of this
   doc. The table is the regression contract — keep it in sync.

## Out of scope

- **Implementing the real backend.** [`real.ts`](src/lib/backend/real.ts) is
  a stub that throws. Replace its body when you wire your provider; respect
  the contract and nothing else changes.
- **Persisting playground conversations.** They're ephemeral by design; the
  `localStorage` path in [`lib/storage.ts`](src/lib/storage.ts) is reserved
  for the real chat surface.
- **CI assertions on the prod bundle.** Documented above as a manual step;
  not enforced automatically.
