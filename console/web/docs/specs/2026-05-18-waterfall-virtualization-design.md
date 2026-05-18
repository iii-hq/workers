# WaterfallChart Virtualization — Design

## Goal

Render trace waterfalls with thousands of spans without freezing the page. Today `WaterfallChart.tsx:403` maps every visible span to a DOM row; a 4000-span trace blocks the React commit phase long enough that the page is unresponsive and the parent card stays on "loading…" because no repaint can land. Group-by-message routinely produces traces in that range.

## Approach

Hand-rolled fixed-height windowing. Compute a `[startIndex, endIndex)` slice from existing scroll state and render only that slice inside a height spacer. No new dependency, no scroll-ownership change. The chart already has every input the math needs (`ROW_HEIGHT=32`, `containerHeight`, `scrollPosition`), so the diff is small and the minimap math at `WaterfallChart.tsx:296` stays correct unchanged.

## Components

### `lib/virtualWindow.ts` (new)

One pure function, no React imports:

```ts
export interface VirtualWindow {
  startIndex: number
  endIndex: number
  offsetY: number
}

export function computeVirtualWindow(args: {
  scrollTop: number
  containerHeight: number
  rowHeight: number
  itemCount: number
  /** Rows rendered above/below the viewport to absorb fast scrolls. Default 8. */
  overscan?: number
}): VirtualWindow
```

Behavior:
- `itemCount === 0` → `{ 0, 0, 0 }`.
- `containerHeight === 0` (first paint before resize observer fires) → render at least `2 * overscan` rows starting at 0 so first paint shows content.
- `scrollTop` clamped to `[0, itemCount * rowHeight]`.
- `startIndex = max(0, floor(scrollTop / rowHeight) - overscan)`.
- `endIndex = min(itemCount, ceil((scrollTop + containerHeight) / rowHeight) + overscan)`.
- `offsetY = startIndex * rowHeight`.

### `WaterfallChart.tsx` (modify)

Inside the existing scroll container, replace `{visibleSpans.map(...)}` with:

```tsx
const { startIndex, endIndex, offsetY } = computeVirtualWindow({
  scrollTop: scrollPosition,
  containerHeight,
  rowHeight: ROW_HEIGHT,
  itemCount: visibleSpans.length,
})
const slice = visibleSpans.slice(startIndex, endIndex)

<div style={{ height: contentHeight, position: 'relative' }}>
  <div style={{ transform: `translateY(${offsetY}px)` }}>
    {slice.map((span) => /* existing row JSX, unchanged */)}
  </div>
</div>
```

Per-row JSX is untouched. Expand/collapse, hover, click, kind indicator, status dot, label, bar, critical-path accent, merged-routing chip — all stay where they are.

### `SessionDetailPanel.tsx` (modify)

Remove the huge-trace interstitial added in the previous fix. With virtualization in place the page no longer freezes on commit, so the "render anyway" gate is dead weight.

- Drop `HUGE_SPAN_THRESHOLD`, `renderHuge` state, and the interstitial JSX.
- Restore `defaultOpen={idx === 0}` unconditionally — the original "first card auto-opens so the user sees content immediately" behavior.
- Keep `retry: 1` on the `useQuery` (independent improvement — failed fetches surface in seconds instead of stacking).
- Keep the `expectedSpans` hint and the `groupAttribute` header label work — both are useful regardless of virtualization.

## Data flow

```
scrollPosition (reducer, updated on container onScroll)
       ┃
       ▼
computeVirtualWindow ──► { startIndex, endIndex, offsetY }
       ┃
       ▼
visibleSpans.slice(startIndex, endIndex) ──► row render
       ┃
       ▼
<div height=contentHeight>           (scrollbar anchor — unchanged)
  <div translateY=offsetY>           (slice container)
    rows...
  </div>
</div>
```

Minimap at `WaterfallChart.tsx:556` continues to read `contentHeight` and `viewportRatio`. No change needed.

Expand/collapse changes `visibleSpans.length` → `contentHeight` re-derived → spacer resizes → scrollbar adjusts naturally. No additional plumbing.

## Edge cases

- **Empty list** (`itemCount === 0`): window is `{0,0,0}`, slice is empty, height-spacer is `0`. The toolbar still renders.
- **First paint** (`containerHeight === 0`): window returns the first `2 * overscan` rows so the user doesn't see a flash of nothing while the resize observer settles.
- **Scrolled past end** (browser overshoot, programmatic scroll, list shrinks while scrolled): `startIndex` is clamped to `[0, itemCount)`. `endIndex` is `min(itemCount, ...)`.
- **Selection / hover on rows outside the window**: external `selectedSpanId` and reducer `hoveredSpanId` already work by id, so selecting a span via the right-side detail panel still highlights its row when the user scrolls back to it. No "scroll into view on select" affordance in scope.

## Testing

- `tests/lib/virtualWindow.test.ts` — pure unit tests for the six cases above.
- No render-perf assertion in vitest (unreliable). Manual verification: open a trace with `>1000` spans, confirm the page stays responsive and scrolls smoothly, expand/collapse propagates, click on a row still selects it.

## Out of scope

- Variable-height rows (rows are uniformly `ROW_HEIGHT=32`).
- Horizontal virtualization.
- Replacing the scroll model (e.g. infinite-load semantics).
- `TraceListRow` and other lists — server-paginated, not a perf problem.
- "Scroll into view on selection" — separate UX call.
