---
status: draft
date: 2026-05-18
owner: anderson
topic: traces page port from motia/console to workers/console/web
---

# traces page — design

A new top-level `traces` view in `workers/console/web` that ports the full
observability surface from `motia/console/packages/console-frontend` and
re-skins every component onto the iii Schematic design system
(`workers/console/web/DESIGN.md`).

## 1. goals

- Surface engine traces (list, waterfall, flame, map, flow, span detail,
  group-by, session detail) inside the existing chat app.
- Wire the page so users can move between `chat` and `traces` from the
  current `ModeToggle` in `App.tsx` without leaving the page.
- Render every element on the iii Schematic — Chivo Mono, lowercase voice,
  1px hairline rules, single orange accent, no rounded corners, no shadows.
- Keep the data layer aligned with how this app already talks to the
  engine: one `iii-browser-sdk` WebSocket singleton, called via
  `getIiiClient()`. No parallel SDK provider, no new engine connection.

## 2. non-goals

- No multi-engine or cross-tenant trace surfacing.
- No write-side trace ingestion or OTel-collector configuration UI.
- No bespoke alerting / SLO dashboards on top of traces (out of scope —
  cell empty-state when the memory exporter is off, no upsell).
- No TanStack Router migration for the chat app — the existing hash router
  stays the canonical routing primitive.
- No port of motia's `cmdk` command palette inside `TraceFilters`. The
  schematic rebuild uses bordered inputs + segmented controls instead.

## 3. architecture

### 3.1 routing

Extend the existing `View` union in `src/hooks/use-hash-route.ts` from
`'chat' | 'playground' | 'examples'` to add `'traces'`. Wire it as a
fourth option in the `VIEW_OPTIONS` array in `App.tsx`. Traces ships
unconditionally — no build flag — since the "engine has no exporter"
case is already handled gracefully by an empty-state `Cell` (§5 risk
#1). Hash routes preserve view state across reloads.

No TanStack Router. The motia `createFileRoute('/traces')` boilerplate
collapses into a single conditional render in `App.tsx`:

```tsx
{view === 'traces' ? <Traces /> : view === 'examples' ? <Examples /> : …}
```

### 3.2 data layer

The chat app already owns a thin singleton over `iii-browser-sdk@0.12.0`
in `src/lib/iii-client.ts`. The trace surface reuses it verbatim.

**Why no `useEngineSdk()` provider.** Inspecting the SDK (`index.d.mts`,
`registerWorker`, `ISdk.trigger`), there is no traces API in the SDK
itself — motia's `api/observability/traces.ts` simply calls
`sdk.trigger({ function_id: 'engine::traces::list', payload })` etc., the
same generic RPC used today by `client.call('ui::subscribe', ...)` and
`client.call('run::start', ...)`. Engine RPC paths are passed as strings.
So the traces port talks to the engine via the same `getIiiClient()` the
chat backend uses.

**Adapter file:** `src/pages/Traces/api/traces.ts` ports
`motia/console/.../api/observability/traces.ts` with one structural
change — drop the `sdk` parameter, resolve the client internally:

```ts
import { getIiiClient } from '@/lib/iii-client'

export async function fetchTraceTree(
  traceId: string,
): Promise<TraceTreeResponse> {
  const client = await getIiiClient()
  return client.call<TraceTreeResponse>(
    'engine::traces::tree',
    { trace_id: traceId },
  )
}
```

Same shape for `listTraces`, `clearTraces`, `fetchTraceGroups`. Function
ID constants (`engine::traces::list|tree|clear|group_by`) stay verbatim
from motia's `OBSERVABILITY_TRACE_FUNCTIONS` record.

**Connection state.** The SDK exposes
`addConnectionStateListener(handler)` and the existing wrapper
re-exports it. The trace page subscribes to it once at mount and renders
a `StatusPanel variant="alert"` (DESIGN.md §10) when the engine is
disconnected or the trace exporter is not registered (the latter is
detected by a "function not registered" rejection on `engine::traces::list`).

**Query layer.** `@tanstack/react-query` v5 is added. `useTraceData` and
`useTraceGroups` mirror motia's hooks, swapping out the `sdk` argument
for `getIiiClient()`-based calls. Query keys carry the same shape so
cache invalidation semantics are identical.

### 3.3 file layout

```
web/src/pages/Traces/
  index.tsx                          # TracesPage (port of routes/traces.tsx)

  components/
    TraceListRow.tsx                 # extracted from index for testing
    TraceHeader.tsx
    TraceFilters.tsx                 # rewritten on schematic primitives
    AttributesFilter.tsx             # collapsible <details> per §12
    ViewSwitcher.tsx                 # 4-up ModeToggle
    WaterfallChart.tsx
    FlameGraph.tsx
    TraceMap.tsx                     # xyflow custom nodes/edges
    FlowView.tsx                     # xyflow + dagre, custom renderer
    WorkflowChain.tsx
    SpanPanel.tsx                    # Radix Tabs + 6 tab bodies
    SpanInfoTab.tsx
    SpanErrorsTab.tsx
    SpanLogsTab.tsx
    SpanOtelLogsTab.tsx
    SpanTagsTab.tsx
    SpanBaggageTab.tsx
    SpanLinksTab.tsx
    SessionDetailPanel.tsx
    TraceGroupsView.tsx
    ServiceBreakdown.tsx

  hooks/
    useTraceData.ts
    useTraceFilters.ts
    useTraceGroups.ts
    useResizablePanels.ts

  lib/
    spanTree.ts        + spanTree.test.ts        # pure logic — ported as-is
    traceTransform.ts  + traceTransform.test.ts
    traceFilters.ts    + traceFilters.test.ts
    traceListItem.ts   + traceListItem.test.ts
    traceUtils.ts      + traceUtils.test.ts
    groupTraces.ts     + groupTraces.test.ts
    spanLabel.ts       + spanLabel.test.ts
    formatPossibleJson.ts + formatPossibleJson.test.ts
    otel-utils.ts
    timeRangeUtils.ts
    traceColors.ts                   # re-pointed to schematic palette

  api/
    traces.ts                        # observability RPC wrapper

web/src/components/ui/               # net-new schematic primitives
  Tabs.tsx                           # Radix tabs wrapper
  Tooltip.tsx                        # Radix tooltip wrapper
  Pagination.tsx                     # bordered, label-caps
  Skeleton.tsx                       # rectangular bg-panel pulse
  EmptyState.tsx                     # wraps Cell from §10
  ErrorBoundary.tsx                  # fallback as StatusPanel
  Badge.tsx                          # label-caps text inline (no fill)
  StatusPanel.tsx                    # DESIGN.md §10 canonical
  Cell.tsx                           # DESIGN.md §10 canonical
  Button.tsx                         # DESIGN.md §10 canonical (cva + Slot)

web/src/lib/
  cn.ts                              # clsx + tailwind-merge per §0
```

The `lib/` utilities (~10 files plus tests) are pure data transforms with
zero React or DOM coupling — they port verbatim and keep their existing
vitest specs.

### 3.4 dependency plan

**Add:**

- `@tanstack/react-query` ^5
- `@radix-ui/react-tabs`
- `@radix-ui/react-tooltip`
- `@radix-ui/react-slot` (required by DESIGN.md §10 Button)
- `@xyflow/react`
- `dagre`, `@types/dagre`
- `lucide-react`
- `zod` (filter validation)
- `class-variance-authority` (Button variants per §10)
- `clsx`, `tailwind-merge` (cn helper per §0)

Total: 12 new packages. Bundle impact estimate: ~250 KB gzipped
(`@xyflow/react` + `dagre` are the largest contributors at ~80 KB
combined; the rest are small).

**Skip** (motia uses, schematic doesn't need):

- `@tanstack/react-router`, `@tanstack/zod-adapter` — hash router stays.
- `cmdk` — replaced by bordered inputs + `ModeToggle` segments + the
  schematic `SearchField`.
- `sonner` — no toasts on the trace page.
- `@radix-ui/react-dialog`, `react-select`, `react-separator`,
  `react-label` — span detail is inline, not modal; filters use bordered
  selects, not Radix select.

## 4. design-system mapping

### 4.1 token swap

Every token used by the motia traces page maps onto a schematic token.
Audit applied during port:

| motia token                           | schematic token                                                          |
| ------------------------------------- | ------------------------------------------------------------------------ |
| `bg-background`                       | `bg-bg`                                                                  |
| `bg-foreground`                       | `bg-ink` (rare — only inverse fills like primary button)                 |
| `bg-sidebar`, `bg-dark-gray`          | `bg-panel`                                                               |
| `bg-dark-gray/30`, `bg-dark-gray/50`  | `bg-panel` (the schematic does not use opacity steps on neutrals)        |
| `bg-primary/10` (selected row)        | `bg-panel` + `border-l-2 border-l-accent` (§4 allowed accent on rule)    |
| `text-foreground`                     | `text-ink`                                                               |
| `text-muted`, `text-mute`             | `text-ink-faint`                                                         |
| `text-muted-foreground`               | `text-ink-ghost`                                                         |
| `text-primary`                        | `text-accent`                                                            |
| `text-accent` (motia accent)          | `text-accent` (orange in schematic — already canonical)                  |
| `text-success`, `bg-success/x`        | `text-accent` (success uses accent per §9)                               |
| `text-warning`, `bg-warning/x`        | `text-warn` (#a87a00 — no full fill)                                     |
| `text-error`, `bg-error/x`            | `text-alert` (#c43e1c) — body fill stays `bg`, row stripe via `border-l-2 border-l-alert` + faint `bg-alert/5` (§9) |
| `text-yellow animate-pulse` (pending) | `StatusDot tone="warn" pulse` (§8 sanctioned pulse)                      |
| `border-border`                       | `border-rule`                                                            |
| `border-border-subtle`                | `border-rule-2`                                                          |
| `rounded`, `rounded-md`, `rounded-lg` | `rounded-none` (§6 — there is no "soft" variant)                         |
| `shadow-*`                            | dropped — borders define structure (§4); `.deal-shadow` is reserved      |

The schematic palette tokens come from DESIGN.md §0. The trace page
inherits them by importing the same `@theme` block — no per-page token
definitions.

### 4.2 component swap

| motia component                                | schematic equivalent                                                                            |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Page header (`<h1>Traces</h1>` + actions)      | `PageHeader` (§10) with `eyebrow="$"`, lowercase `title="traces"`, actions slot                 |
| `Badge variant="warning"` ("paused")           | `label-caps-sm` text inline beside title — no full-color fills (§9)                             |
| `Button` from motia's `card.tsx`               | `Button` from DESIGN.md §10 (`primary`, `ghost`, `pill`, `icon`, `terminal`)                    |
| `StatusIcon` (CheckCircle2 / XCircle / Activity) | `StatusDot` (§10) with `tone={accent\|alert\|warn}`, `pulse` for pending                       |
| Trace list row, selected (`bg-primary/10`)     | `bg-panel` + `border-l-2 border-l-accent` (mirrors WorkerCard focused, §10)                     |
| Trace list row, new (`animate-trace-flash`)    | 600 ms `bg-accent/8 → bg-bg` ease-out, defined as `@utility trace-flash` next to `pulse-dot`    |
| `Skeleton` (rounded grey block, animate-pulse) | Rectangular `bg-panel` block with 1.4s opacity-pulse keyframe; no rounded                       |
| `Pagination` (shadcn-style, rounded)           | Bordered control: `ghost` Button arrows + label-caps-sm "page n of m", `tabular-nums` counts    |
| `EmptyState` (icon + title + desc)             | `Cell` (§10) with optional lucide icon at 18px in `text-ink-faint`                              |
| `ErrorBoundary` fallback                       | `StatusPanel variant="alert"` (§10) with retry `Button`                                         |
| `Tabs` (Radix) inside `SpanPanel`              | Radix Tabs underneath, styled: label-caps-sm tab labels, active tab gets `border-b border-ink`  |
| `Tooltip` (motia tooltip)                      | Radix Tooltip styled: `border border-rule`, `bg-bg`, label-caps-sm metadata + monospace body    |
| Resize divider                                 | 3px column: `bg-rule hover:bg-accent active:bg-accent`, no transition glow                      |

### 4.3 charts

Charts are the visual heavy lift. Each gets re-skinned on the schematic
palette and stripped of rounded geometry / gradients / shadows.

**WaterfallChart.** Track: `bg-rule-2`. Bars: `bg-ink`. Error bars:
`bg-alert`. Time axis ticks: `text-ink-ghost label-caps-sm`. Selected
span: `outline outline-2 outline-accent` (the single accent moment per
visible region — §3). Hover ruler: 1px vertical `border-l border-rule`.
Sticky time axis row: `border-b border-rule`. No rounded bar corners. No
gradient fills.

**FlameGraph.** Same palette, hierarchical rectangles tiled in three
ink shades by depth (`bg-ink/100 → bg-ink/85 → bg-ink/70`) — never
chromatic gradients. Error rects: `bg-alert/85`. Text inside rects:
`text-bg` micro / code-sm. Selected rect: `outline outline-2
outline-accent`. No rounded corners. No drop shadows on hover.

**TraceMap (xyflow, force-directed).** Custom node renderer wraps each
span as a §10-style bordered card: `bg-bg`, 1px `border-rule`,
`bg-panel` head strip carrying a `StatusDot` + label-caps-sm operation
name, body shows `tabular-nums` duration + service in `text-ink-faint`.
Edges: 1px `stroke-ink`. Error edges: `stroke-alert`. Background:
solid `bg-bg`, no dot grid. Zoom/pan controls: `Button variant="icon"`
from §10.

**FlowView (xyflow + dagre).** Same node renderer as TraceMap.
Top-to-bottom dagre layout. Function/trigger lanes drawn as `border-r
border-rule` columns. Active span: focused `border-l-2 border-l-accent`
rail on the node. No background coloring of lanes.

**WorkflowChain.** Chain segments rendered as small `Cell`-shaped boxes
joined by 1px `border-t border-rule` horizontal lines. Each segment:
label-caps-sm name + `tabular-nums` duration. No icons inside segments
(label is the identity).

### 4.4 TraceFilters rewrite

Motia's `TraceFilters.tsx` is 29 KB and depends on `cmdk` for command
palettes plus `react-select` for the dropdowns. The schematic rebuild:

- Search input: `SearchField` (§10) at 14 px (not 24 px — this is a
  filter bar, not a hero search), inline blinking accent caret.
- `groupBy` segments: `ModeToggle<GroupBy>` with options `none` /
  `function` / `trigger` / `session`.
- `timeRange` / `statusFilter` selects: custom bordered triggers that
  open a 1px-ruled pop list. No `react-select`. No `cmdk`.
- `AttributesFilter` (advanced): a `<details>` block (§12 pattern). The
  open marker is a label-caps-sm chip; rows inside are key + operator +
  value tuples with `border border-rule` chrome.
- "Clear all" / "Save filter" actions: `Button variant="ghost"`
  size="sm".

Validation warnings (currently a yellow banner) become a `StatusPanel
variant="warn"` between the filter row and the list.

### 4.5 SpanPanel rewrite

Radix Tabs stay; the chrome changes. Tab list: horizontal strip with
label-caps-sm labels in `text-ink-faint`, active tab `text-ink` with a
1px `border-b border-ink` underline. Tab body: flat divide-y rule-2
type-table per §12 (one row per attribute: name / type / value, with
description on a second line). No nested cards inside tab bodies.

Tab order (preserved from motia):

1. `info` — span identity, duration, status, service.
2. `errors` — exception events, stack frames, error attributes.
3. `events` — span events (renamed from "Logs" in motia — keep as
   "logs" for parity, lowercase).
4. `otel logs` — correlated OTel log records.
5. `tags` — tag attributes (`http.*`, `iii.*`, etc.) as a flat table.
6. `baggage` — OTel baggage key/value pairs.
7. `links` — span-to-span links with click-through to navigate.

The "no key remount" comment in `routes/traces.tsx:632-640` carries over
verbatim — the tab strip retains its state when navigating between
spans.

### 4.6 live-update UX (preserved as-is)

The motia page has three interlocking behaviors that the port keeps
identical:

- **Pause button.** Manual pause/resume of the live query. Schematic:
  `Button variant="ghost"` with a `Pause`/`Play` lucide icon plus a
  label-caps-sm "paused" word next to the title.
- **Hover-pause.** Pointer entering the list pauses new-trace ingestion;
  leaving flushes pending traces. No visual indicator — the absence of
  the "paused" label is the signal. (See `routes/traces.tsx:346-354`.)
- **Trace flash.** New traces fade in with a 600 ms accent-faint flash
  (`@utility trace-flash`). The animation end clears the trace ID from
  the `newTraceIds` set.

## 5. risks and verification gates

1. **Engine has the trace exporter enabled.** The engine the chat app
   talks to must register `engine::traces::list|tree|clear|group_by`.
   Detection: a "function not registered" rejection on first list call.
   Failure path: render `Cell` empty state titled "no observability" with
   an ink-faint body explaining the engine has no exporter configured.
   Same shape as motia's `hasOtelConfigured = false` branch.

2. **TanStack Query v5 + React 19 strict mode.** Both are compatible per
   the v5 changelog, but verify in Phase 1 by booting the app under
   `<React.StrictMode>` (Vite default) and running through the trace
   list with the devtools attached. Flag any double-effect warnings.

3. **Tailwind v4 token block.** Confirm `index.css` already imports the
   `@theme` block from DESIGN.md §0. If not, add it as part of Phase 1
   so the trace components can reference `bg-bg`, `text-ink-faint`,
   `border-rule`, `text-accent`, `text-warn`, `text-alert`, etc.

4. **Connection-state flicker.** The SDK reconnects with backoff; the
   trace page must not thrash queries during the brief
   `connecting`/`reconnecting` window. `useTraceData` should gate
   refetches on `state === 'connected'` and clear in-flight queries on
   transitions back to `disconnected`.

5. **xyflow CSS isolation.** `@xyflow/react` ships its own stylesheet
   (`@xyflow/react/dist/style.css`) that uses CSS variables and applies
   global resets. Verify it does not collide with DESIGN.md's `@theme`
   block. If it does, scope the import to the two view components that
   need it and override its variables to schematic tokens.

6. **Lowercase audit.** Catch any Title-Case strings inherited from
   motia: "Refresh", "Clear all filters", "No traces recorded",
   "Resume", "Failed to load trace", etc. Phase 8 includes a
   line-by-line copy pass.

## 6. phasing

Each phase ends with a working app — no half-built features sitting
around. Phases are independently mergeable.

| phase | scope                                                                                                          | verifiable outcome                                                            |
| ----- | -------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| 1     | Plumbing: `cn` helper, DESIGN.md tokens audited in `index.css`, deps installed, hash route adds `'traces'`, `Cell`/`StatusPanel`/`Button` primitives, empty `TracesPage` rendering "engine not connected" placeholder | App boots; navigating to traces shows a friendly empty state                  |
| 2     | `api/traces.ts` adapter, `useTraceData` wired through `getIiiClient()`, raw trace list rendered (no styling polish) | List of traces appears on the page (typed any-styled scaffolding)             |
| 3     | List view: rows, `StatusDot`, duration, services, pagination, empty state, skeleton; schematic-styled            | List view is feature-complete and visually settled                            |
| 4     | Trace detail header + `WaterfallChart` + `useResizablePanels`                                                   | Click trace → see waterfall                                                   |
| 5     | `SpanPanel` + all 7 tabs (info, errors, logs, otel-logs, tags, baggage, links)                                  | Click span → see all detail tabs                                              |
| 6     | `FlameGraph` + `ViewSwitcher` + `ServiceBreakdown`                                                              | Flame view + service breakdown work                                           |
| 7     | `TraceMap` + `FlowView` (xyflow + dagre)                                                                        | Both graph views render and navigate                                          |
| 8     | Group-by + `SessionDetailPanel`, live updates (pause/resume + hover-pause + flash), `TraceFilters` polish, lowercase audit | Feature-complete, schematic-clean                                       |

## 7. acceptance criteria

- Navigating to `#traces` shows the page; navigating away preserves
  state via the hash route.
- All UI copy is lowercase except for the `label-caps-*` styles.
- No element has rounded corners except `StatusDot` (and any glyph
  circles).
- No element has a drop shadow except the existing `.deal-shadow`
  utility (which the trace page does not use).
- The orange accent appears at most once per visible region (one of:
  status dot, selected-row rail, live pulse, or focused tab underline).
- All numbers (durations, counts, timestamps) render with
  `tabular-nums`.
- The page reflows gracefully under 880 px: list, detail, and span
  panel collapse in that order. Uses container queries, not viewport
  breakpoints.
- Light is the default theme; switching to dark via the existing
  `ModeToggle` re-renders the trace page correctly (palette inverts,
  accent swaps to `#3ea8ff`).
- Live updates pause on pointer hover and flush on leave — same as
  motia's behavior.
- `pnpm typecheck` and `pnpm lint` pass cleanly.
- All ported `lib/*.test.ts` files pass under vitest.

## 8. open decisions

1. **Engine connection feedback.** Show a persistent connection-state
   chip in the page header? Motia hides connection state from the
   traces UI entirely. Recommendation: don't add one for traces; the
   global app should surface engine connectivity once at the app shell
   level, not per page.

2. **Defer xyflow views?** Phase 7 (TraceMap + FlowView) adds the
   biggest dependencies. If shipping speed matters more than parity,
   stop after Phase 6 and revisit. Recommendation: include them — the
   user explicitly chose Option A (full port).
