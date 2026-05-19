# Waterfall virtualization library swap — design

**Status:** draft, awaiting user review.
**Author:** anderson + claude
**Date:** 2026-05-19
**Touches:** `console/web/src/pages/Traces/components/WaterfallChart.tsx`,
`console/web/src/pages/Traces/lib/virtualWindow.{ts,test.ts}` (delete),
`console/web/package.json`.

## Context

PR #157 shipped the traces page with hand-rolled fixed-height
virtualization (`lib/virtualWindow.ts`). PR #158 followed up with two
perf fixes: removed `hoveredSpanId` from the reducer (so mouse-sweep
across rows stopped triggering full chart re-renders) and rAF-throttled
`handleScroll`.

User testing on a 2389-span trace surfaces three remaining glitches:

1. **Background color and hover state flicker on rows during scroll.**
   `transition-colors` on the row chrome and `transition-all
   duration-150` on the timing bar animate when rows enter or leave the
   virtualization slice. At ~120Hz trackpad scroll this becomes
   constant low-amplitude flicker.
2. **Rows shift / jump position when expand or toggle filters.**
   Toggling `hide engine routing` / `collapse routing pairs`
   recomputes `visibleSpans`. The slice container's `translateY(offsetY)`
   jumps instantly to the new offset because the whole slice translates
   as one unit. The same span moves from `offsetY=500` to `offsetY=200`
   with no intermediate state.
3. **Whole-chart pause for hundreds of ms periodically.** Rows are not
   memoized. Each scroll-driven re-render reconciles the entire visible
   slice (~40 rows × ~15 React elements each = ~600 elements) even
   though most rows are the same as the previous frame, just shifted by
   one. Inline closures (`onClick={() => onSpanClick(span)}`) and
   inline style objects make every prop identity unstable, so React
   cannot skip any row.

Goal: smooth scroll, no flicker on row mount, no visible jump on
filter toggle, no main-thread pause beyond a single frame budget on
data load. End user: a trace investigator can fluidly scroll and
filter a 2000+-span trace without the page feeling broken.

## Approach (chosen: option C — adopt `@tanstack/react-virtual`)

`@tanstack/react-virtual` v3.x is a small (~5KB gz) headless React
hook. It uses ResizeObserver and a passive native scroll listener
internally, exposes `getVirtualItems()` and `getTotalSize()`, and
positions each rendered row independently via absolute positioning.

Two of the three glitches dissolve with the swap alone:

- **Jump on toggle.** The library positions every row at its own
  `top: vrow.start` via inline `transform`. When `visibleSpans`
  changes shape, individual rows slide to their new positions in one
  composited paint instead of the whole slice translating as a block.
- **Pause on scroll.** Scroll state never enters React. The library's
  internal scroll handler reads `scrollTop` against a ResizeObserver-
  tracked container size and produces a fresh `VirtualItem` array
  only when the visible range actually changes — not on every wheel
  tick.

The third glitch (flicker) is not virtualization-shaped — it's purely
the CSS transitions. We strip those in the same change.

A side concern: rows currently re-render unnecessarily because their
props identity changes every parent render. We address that by
extracting `WaterfallRow` as a `React.memo` component, `useCallback`-
ing the parent handlers, and moving `gridTemplateColumns` to a CSS
custom property on the scroll container so column-width resize
updates one style instead of 40.

### Why not option A (memoize + strip animations only)

Cheap, single-file, no new dep. But the "jump on toggle" glitch is
inherent to the single-translate-block approach. Fixing it
hand-rolled would mean rewriting our virtualization to per-row
absolute positioning anyway, which is exactly what the library
already provides battle-tested. Not worth maintaining our own.

### Why not option B (A + iterative spanTree.ts)

Helps deep-chain traces but irrelevant to today's 2389-span trace —
those recursions are microseconds. Defer until a deeply nested
trace surfaces.

## Files

**Modify**
- `console/web/src/pages/Traces/components/WaterfallChart.tsx` —
  replace virtualization plumbing; extract row component.
- `console/web/package.json` — add
  `"@tanstack/react-virtual": "^3.10.0"` (pin to current stable
  range; update the actual version to whatever is current when
  the implementation lands).

**Delete**
- `console/web/src/pages/Traces/lib/virtualWindow.ts`
- `console/web/src/pages/Traces/lib/virtualWindow.test.ts`

**Unchanged**
- `lib/spanTree.ts` — `buildSpanTree`, `flattenTree`,
  `markCriticalPath` stay. Virtualization is orthogonal to tree
  shape.
- `lib/traceTransform.ts` — already iterative from PR #157.
- `SessionDetailPanel.tsx` — the parent is unaware of the
  virtualization swap. Its loading-state UI and click-to-load
  affordance (PR #157) keep working as-is.

## Concrete shape of the new render

```tsx
import { useVirtualizer } from '@tanstack/react-virtual'

// inside WaterfallChart
const scrollParentRef = useRef<HTMLDivElement | null>(null)

const stableOnSpanClick = useCallback(
  (span: FlatSpanRow) => onSpanClick(span),
  [onSpanClick],
)
const toggleExpand = useCallback(
  (spanId: string) => dispatch({ type: 'TOGGLE_SPAN', spanId }),
  [],
)

const virtualizer = useVirtualizer({
  count: visibleSpans.length,
  getScrollElement: () => scrollParentRef.current,
  estimateSize: () => ROW_HEIGHT,
  overscan: 16,
})

const virtualRows = virtualizer.getVirtualItems()
const totalSize = virtualizer.getTotalSize()

return (
  <div className="flex flex-col h-full">
    {/* toolbar + sticky time axis unchanged */}
    <div
      ref={scrollParentRef}
      style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
      className="flex-1 overflow-y-auto"
    >
      <div style={{ height: totalSize, position: 'relative' }}>
        {virtualRows.map((vrow) => {
          const span = visibleSpans[vrow.index]
          return (
            <div
              key={vrow.key}
              data-index={vrow.index}
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                width: '100%',
                height: ROW_HEIGHT,
                transform: `translateY(${vrow.start}px)`,
              }}
            >
              <WaterfallRow
                span={span}
                isSelected={selectedSpanId === span.span_id}
                isExpanded={expandedIds.has(span.span_id)}
                isCritical={showCriticalPath && span.isCriticalPath}
                onSpanClick={stableOnSpanClick}
                toggleExpand={toggleExpand}
              />
            </div>
          )
        })}
      </div>
    </div>
  </div>
)
```

`WaterfallRow` lives in the same file (or a sibling file
`WaterfallRow.tsx` if it grows enough). It is `React.memo`-wrapped
with default shallow compare. Inside, the row uses Tailwind arbitrary
property syntax to read the CSS variable:

```tsx
<div className="grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-1 items-center cursor-pointer w-full text-left ...">
```

No `transition-colors` on the row chrome. No `transition-all
duration-150` on the bar. `transition-transform` on the chevron is
kept — it animates on user click (expand/collapse), not on scroll, so
no flicker class.

## State removed from the reducer

The `DisplayState.scrollPosition` field and the `SET_SCROLL` action
become dead code once the library owns scroll. Same for the
`containerHeight` `useState` and the `useEffect` that wired up the
ResizeObserver. Removing them shrinks the reducer back to its essential
concerns (`expandedIds`, `showCriticalPath`).

## Error handling

- On first render `scrollParentRef.current` is null. `useVirtualizer`
  handles that gracefully — `getVirtualItems()` returns an empty array
  until the ref attaches. Same as our current
  "containerHeight === 0 first-paint" branch.
- If `visibleSpans` shrinks below the current scroll position (e.g.
  collapse-all), the library clamps scrollTop. No special handling
  needed.
- No new exception paths.

## Testing

**Existing tests stay green:**
- `traceTransform.test.ts` (159 cases, including the 12k-deep
  regression from PR #157).
- `spanTree.test.ts`, `groupTraces.test.ts`, `spanLabel.test.ts`,
  `traceFilters.test.ts`, `traceListItem.test.ts`,
  `formatPossibleJson.test.ts`.

**Deleted:**
- `virtualWindow.test.ts` (8 cases) — the helper is gone, the library
  is upstream-tested.

**New (optional but recommended):**
- `WaterfallChart.test.tsx` with a 200-span fixture. Asserts:
  (a) only a small subset of rows appears in the DOM at any time,
  proving virtualization is active;
  (b) the first span's name is rendered;
  (c) clicking the chevron toggles `isExpanded`.

  Skip if mounting a full component with `@tanstack/react-virtual` in
  jsdom requires polyfills we don't have. The library is well-tested
  upstream; manual live verification is sufficient.

**Live verification (required):**
- `pnpm dev` from `console/web`.
- Open a 2000+-span trace via the traces page.
- Scroll with trackpad and wheel — should be smooth, no jank.
- Hover rows during scroll — no color flicker.
- Toggle `hide engine routing`, `collapse routing pairs`, click
  expand/collapse on a span — rows should reposition smoothly, no
  visible jump mid-screen.
- Click a span row — selection highlights correctly, span detail
  panel populates.
- Use keyboard (Tab + Enter / Space on a row) — selection works.
- Drag the span-column-resize separator — column width changes
  smoothly, all rows reflect the new width without per-row jank.
- Run `pnpm typecheck` and `pnpm test` — expect green.

## Out of scope

- `spanTree.ts` recursive `buildSpanTree` / `flattenTree` /
  `markCriticalPath` rewrite. Defer until a deeply-nested trace
  surfaces (we already wrote the template in `traceTransform.ts`
  via PR #157).
- Pre-row layout caching beyond what the library does (variable row
  heights, content-visibility tricks).
- Replacing other parts of the traces page (FlameGraph, FlowView,
  ServiceBreakdown, TraceMap) with virtualization. Only the
  waterfall chart needs it.
- Scroll-to-span on selection (e.g., jump to row when SpanPanel
  highlights a different span). Library supports it via
  `virtualizer.scrollToIndex(idx)`; we just don't wire it up here.

## Open questions

None. Library API is stable, our data flow is well-understood,
verification path is concrete.
