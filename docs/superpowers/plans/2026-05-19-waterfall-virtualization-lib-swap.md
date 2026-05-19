# Waterfall Virtualization Library Swap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-rolled virtualization in `WaterfallChart.tsx` with `@tanstack/react-virtual` to eliminate three reported glitches (color flicker, row jump on filter toggle, periodic main-thread pause) and let the library handle the moving parts that today live in our reducer.

**Architecture:** The swap is a chain of refactors that build on each other. First extract a memo'd row component (so individual rows can skip reconciliation), then move column-width to a CSS variable (so resizing doesn't touch row styles), then strip the CSS transitions that cause mount-time flicker, then swap the virtualization core, then delete the dead helper. Each task is independently reviewable and keeps the page working end-to-end.

**Tech Stack:** React 19, TypeScript, Vite, Tailwind, vitest. New dep: `@tanstack/react-virtual` v3.x.

**Spec:** `docs/superpowers/specs/2026-05-19-waterfall-virtualization-lib-swap-design.md`

**Repository conventions:**
- Run all commands from the repo root: `/Users/andersonleal/projetos/motia/workers`
- The console/web package uses pnpm workspaces. `pnpm` commands run via the package's `cd console/web && pnpm <cmd>` or `pnpm --filter chat-app <cmd>` — the existing scripts in `console/web/package.json` define `typecheck` and `test`.
- Commit messages use Conventional Commits (see prior PRs #157, #158). Plain `git commit -m`. Never `--no-verify`.

**Branch:** Pick up the current branch `fix/traces-waterfall-freeze` (PR #158). The spec was committed there; the implementation lands on top.

---

## File structure

**Modify**
- `console/web/src/pages/Traces/components/WaterfallChart.tsx` — primary surgery, all seven tasks touch it.
- `console/web/package.json` — Task 1 adds the dep.

**Delete (Task 6)**
- `console/web/src/pages/Traces/lib/virtualWindow.ts`
- `console/web/src/pages/Traces/lib/virtualWindow.test.ts`

**Unchanged**
- `console/web/src/pages/Traces/lib/spanTree.ts`
- `console/web/src/pages/Traces/lib/traceTransform.ts`
- `console/web/src/pages/Traces/components/SessionDetailPanel.tsx`

---

## Task 1: Install `@tanstack/react-virtual`

**Files:**
- Modify: `console/web/package.json`
- Modify: `console/web/pnpm-lock.yaml` (auto-generated)

- [ ] **Step 1: Install the dep**

```bash
cd console/web && pnpm add @tanstack/react-virtual
```

Expected output: a line like `+ @tanstack/react-virtual <X.Y.Z>` and a lockfile update. The version pinned should be the latest stable in the 3.x line (3.10+ as of this writing). Take whatever pnpm resolves.

- [ ] **Step 2: Confirm the dep landed**

```bash
grep -n "react-virtual" console/web/package.json
```

Expected output:
```
"@tanstack/react-virtual": "^3.X.Y",
```

- [ ] **Step 3: Run typecheck and tests to confirm nothing broke**

```bash
cd console/web && pnpm typecheck && pnpm test
```

Expected: typecheck clean, all existing tests pass (159 cases).

- [ ] **Step 4: Commit**

```bash
git add console/web/package.json console/web/pnpm-lock.yaml
git commit -m "chore(traces): add @tanstack/react-virtual dep

Used by the upcoming WaterfallChart virtualization swap. The hook
replaces our hand-rolled lib/virtualWindow.ts (deleted in a later
task) and gives us per-row absolute positioning so filter toggles
no longer jump the whole slice."
```

---

## Task 2: Extract `WaterfallRow` as a memo'd component

This task is a pure refactor — it does NOT change behavior. Pulling the row JSX out of the parent + wrapping in `React.memo` + stabilizing the parent handlers via `useCallback` is the prerequisite for the virtualization swap. Without it, swapping virtualization would not actually fix the "periodic pause" glitch because every scroll-triggered re-render would still reconcile every row.

**Files:**
- Modify: `console/web/src/pages/Traces/components/WaterfallChart.tsx`

- [ ] **Step 1: Inside `WaterfallChart.tsx`, after the existing `import` block but before the `interface DisplayState` declaration, define the `WaterfallRow` component and its props type.**

Insert this block immediately after the last import (around L51):

```tsx
interface WaterfallRowProps {
  span: FlatSpanRow
  isSelected: boolean
  isExpanded: boolean
  isCritical: boolean
  spanColWidth: number
  onSpanClick: (span: VisualizationSpan) => void
  onToggleExpand: (spanId: string) => void
}

const WaterfallRow = memo(function WaterfallRow({
  span,
  isSelected,
  isExpanded,
  isCritical,
  spanColWidth,
  onSpanClick,
  onToggleExpand,
}: WaterfallRowProps) {
  const effectiveChildren = span.mergedRouting
    ? (span.children[0]?.children ?? [])
    : span.children
  const hasChildren = effectiveChildren.length > 0
  const isEngineDim = !isSelected && isEngineRoutingSpan(span)
  const kindIndicator = getSpanKindIndicator(span.kind)
  const displayLabel = formatSpanLabel(span)
  const isError = span.status === 'error'

  // Schematic mapping: bar fill is monochrome ink for OK,
  // alert for error, warn for unset/pending. Critical path
  // collapses onto the single accent moment per region per
  // DESIGN.md §3.
  const barClass = isCritical
    ? 'bg-accent'
    : isError
      ? 'bg-alert'
      : span.status === 'ok'
        ? 'bg-ink'
        : 'bg-ink-ghost'

  // Hover is pure CSS (`hover:bg-panel`). Selected/error chrome
  // takes priority over hover via CSS specificity (more-specific
  // bg classes).
  const rowChrome = isSelected
    ? 'bg-panel border-l-2 border-l-accent'
    : isError
      ? 'bg-alert/5 border-l-2 border-l-alert'
      : 'hover:bg-panel'

  return (
    <div
      role="button"
      tabIndex={0}
      className={cn(
        'grid gap-4 px-3 py-1 items-center transition-colors cursor-pointer w-full text-left',
        rowChrome,
      )}
      style={{ gridTemplateColumns: `${spanColWidth}px 1fr` }}
      onClick={() => onSpanClick(span)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSpanClick(span)
        }
      }}
    >
      <div
        className={cn(
          'flex items-center gap-1.5 min-w-0',
          isEngineDim && 'opacity-60',
        )}
      >
        <div
          className="flex-shrink-0 flex"
          style={{ width: span.displayDepth * 16 }}
        >
          {indentKeys(span.span_id, span.displayDepth).map((key) => (
            <div
              key={key}
              className="w-4 h-6 border-l border-rule-2"
            />
          ))}
        </div>

        {hasChildren ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onToggleExpand(span.span_id)
            }}
            className="w-4 h-4 flex items-center justify-center text-ink-faint hover:text-ink flex-shrink-0"
            aria-label={isExpanded ? 'collapse span' : 'expand span'}
          >
            <ChevronRight
              className={cn(
                'w-3 h-3 transition-transform',
                isExpanded && 'rotate-90',
              )}
            />
          </button>
        ) : (
          <div className="w-4 h-4 flex-shrink-0" />
        )}

        <StatusDot tone={statusDotTone(span.status)} />

        {span.service_name && (
          <span className="flex-shrink-0 px-1.5 py-0.5 text-[10px] font-mono tracking-[0.06em] border border-rule bg-bg text-ink-faint leading-none lowercase">
            {span.service_name}
          </span>
        )}
        {kindIndicator && (
          <span
            className="flex-shrink-0 text-[11px] text-ink-faint leading-none w-3 text-center"
            title={kindIndicator.label}
          >
            {kindIndicator.icon}
          </span>
        )}

        <span
          className={cn(
            'text-[13px] font-mono truncate lowercase',
            isSelected ? 'text-accent' : 'text-ink',
          )}
          title={span.name}
        >
          {displayLabel}
        </span>
        {span.mergedRouting && (
          <span
            className="flex-shrink-0 px-1 py-0.5 text-[9px] font-mono tracking-[0.06em] border border-rule bg-panel text-ink-faint leading-none tabular-nums"
            title="merged: this row hides the engine 'call' child of a handle_invocation pair"
          >
            +1
          </span>
        )}

        <span className="font-mono text-[11px] text-ink-faint flex-shrink-0 ml-auto tabular-nums">
          {formatDuration(span.duration_ms)}
        </span>
      </div>

      {/* bar track */}
      <div className="relative h-6 bg-rule-2">
        <div
          className={cn(
            'absolute h-4 top-1 min-w-[3px] transition-all duration-150',
            barClass,
            isSelected && 'outline outline-2 outline-accent',
          )}
          style={{
            left: `${span.start_percent}%`,
            width: `${Math.max(0.5, span.width_percent)}%`,
          }}
          title={`${span.name} — ${formatDuration(span.duration_ms)}`}
        />
      </div>
    </div>
  )
})
```

- [ ] **Step 2: Add `memo` to the React import**

Change the existing import line (around L41):
```tsx
import { useEffect, useMemo, useReducer, useRef, useState } from 'react'
```
to:
```tsx
import { memo, useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
```

- [ ] **Step 3: In the `WaterfallChart` body, replace the inline row map with the new `WaterfallRow`.**

Find the `visibleSlice.map((span) => { ... })` block (around L455-L608) and replace its body with:

```tsx
{visibleSlice.map((span) => {
  return (
    <WaterfallRow
      key={span.span_id}
      span={span}
      isSelected={selectedSpanId === span.span_id}
      isExpanded={expandedIds.has(span.span_id)}
      isCritical={showCriticalPath && span.isCriticalPath}
      spanColWidth={spanColWidth}
      onSpanClick={onSpanClick}
      onToggleExpand={toggleExpand}
    />
  )
})}
```

Delete the rest of the inline row JSX that previously lived between `visibleSlice.map((span) => {` and its closing `})`.

- [ ] **Step 4: Stabilize the `toggleExpand` handler via `useCallback`.**

Find the existing definition (around L284):
```tsx
const toggleExpand = (spanId: string) => {
  dispatch({ type: 'TOGGLE_SPAN', spanId })
}
```

Replace with:
```tsx
const toggleExpand = useCallback((spanId: string) => {
  dispatch({ type: 'TOGGLE_SPAN', spanId })
}, [])
```

The `onSpanClick` prop comes from the parent and is already stable enough — `React.memo`'s shallow compare will treat its reference as stable across parent renders if the parent uses `useCallback` (`SessionDetailPanel.tsx` does — see PR #157 wiring). Do NOT wrap `onSpanClick` in another `useCallback` inside `WaterfallChart`; that would shadow the parent's stability.

- [ ] **Step 5: Run typecheck**

```bash
cd console/web && pnpm typecheck
```

Expected: clean. If there are unused imports flagged (e.g., `getSpanKindIndicator`, `isEngineRoutingSpan`, `formatSpanLabel`, `indentKeys`, `StatusDot`, `ChevronRight`, `cn`, `formatDuration`, `statusDotTone`), they should now be used by `WaterfallRow` instead of the parent — but make sure they are still imported in the file. The imports were already there; the refactor only moves their usage.

- [ ] **Step 6: Run tests**

```bash
cd console/web && pnpm test
```

Expected: 159 tests pass. This is a pure refactor.

- [ ] **Step 7: Commit**

```bash
git add console/web/src/pages/Traces/components/WaterfallChart.tsx
git commit -m "refactor(waterfall): extract WaterfallRow as a memo'd component

Prerequisite for the upcoming @tanstack/react-virtual swap: rows must
be memoizable on primitive props so scroll-triggered re-renders don't
reconcile the whole visible slice. No behavior change.

Pulls the row JSX from the inline map into a top-of-file
WaterfallRow component wrapped in React.memo. Stabilizes
toggleExpand via useCallback. Parent's onSpanClick was already
useCallback'd via SessionDetailPanel.tsx."
```

---

## Task 3: Move `gridTemplateColumns` to a CSS variable

This stops the column-resize handler from updating 40 inline `style` objects per drag frame. The variable lives on the scroll container; the time-axis header and every row read it via Tailwind's arbitrary value syntax.

**Files:**
- Modify: `console/web/src/pages/Traces/components/WaterfallChart.tsx`

- [ ] **Step 1: In `WaterfallRow`, replace the inline `gridTemplateColumns` style with a CSS variable read via Tailwind.**

Find the row's outer `<div>` (the `role="button"` one inside `WaterfallRow`). Change:
```tsx
className={cn(
  'grid gap-4 px-3 py-1 items-center transition-colors cursor-pointer w-full text-left',
  rowChrome,
)}
style={{ gridTemplateColumns: `${spanColWidth}px 1fr` }}
```
to:
```tsx
className={cn(
  'grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-1 items-center transition-colors cursor-pointer w-full text-left',
  rowChrome,
)}
```
(No `style` prop, no `spanColWidth` reference.)

- [ ] **Step 2: Remove `spanColWidth` from `WaterfallRowProps` and from the call site.**

In `WaterfallRowProps`, delete the `spanColWidth: number` line.
In the parent's `WaterfallRow` usage (Task 2's Step 3), delete the `spanColWidth={spanColWidth}` prop.

- [ ] **Step 3: In `WaterfallChart`, set the CSS variable on the scroll container.**

Find the scroll container (around L443-L446):
```tsx
<div
  ref={containerRef}
  className="flex-1 overflow-y-auto"
  onScroll={handleScroll}
>
```

Change to:
```tsx
<div
  ref={containerRef}
  style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
  className="flex-1 overflow-y-auto"
  onScroll={handleScroll}
>
```

The `as React.CSSProperties` cast is required because React's CSSProperties type does not include CSS custom properties by default.

- [ ] **Step 4: Update the sticky time axis to use the same CSS variable.**

Find the time-axis header (around L411-L415):
```tsx
<div
  className="grid gap-4 px-3 py-2 text-[11px] font-semibold text-ink-ghost uppercase tracking-[0.06em] border-b border-rule bg-bg"
  style={{ gridTemplateColumns: `${spanColWidth}px 1fr` }}
>
```

Change to:
```tsx
<div
  style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
  className="grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-2 text-[11px] font-semibold text-ink-ghost uppercase tracking-[0.06em] border-b border-rule bg-bg"
>
```

(The time axis sits OUTSIDE the scroll container — it has its own scope of the variable.)

- [ ] **Step 5: Run typecheck**

```bash
cd console/web && pnpm typecheck
```

Expected: clean.

- [ ] **Step 6: Run tests**

```bash
cd console/web && pnpm test
```

Expected: 159 tests pass.

- [ ] **Step 7: Commit**

```bash
git add console/web/src/pages/Traces/components/WaterfallChart.tsx
git commit -m "refactor(waterfall): hoist gridTemplateColumns to a CSS variable

Column-width resize used to update an inline style on every visible
row (~40 elements). Now the value lives on the scroll container as
\`--span-col-width\` and the rows read it via Tailwind's
\`grid-cols-[var(--span-col-width)_1fr]\` arbitrary value syntax.
Drag the resize handle and only two elements (the time axis header
and the scroll container) get their style mutated — the row
components are unaffected by the resize."
```

---

## Task 4: Strip the row + bar CSS transitions

The `transition-colors` on the row chrome and `transition-all duration-150` on the bar fill animate when rows enter the virtualization slice. This produces the visible flicker the user reports. The chevron's `transition-transform` is kept because it animates on click (expand/collapse), which is a deliberate user interaction.

**Files:**
- Modify: `console/web/src/pages/Traces/components/WaterfallChart.tsx`

- [ ] **Step 1: Remove `transition-colors` from the row.**

In `WaterfallRow`, find the outer `<div>` className (set in Task 3 Step 1):
```tsx
'grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-1 items-center transition-colors cursor-pointer w-full text-left',
```

Change to:
```tsx
'grid grid-cols-[var(--span-col-width)_1fr] gap-4 px-3 py-1 items-center cursor-pointer w-full text-left',
```

(Just drop `transition-colors`.)

- [ ] **Step 2: Remove `transition-all duration-150` from the bar fill.**

Find the bar fill `<div>` inside `WaterfallRow` (around the `barClass` consumer):
```tsx
className={cn(
  'absolute h-4 top-1 min-w-[3px] transition-all duration-150',
  barClass,
  isSelected && 'outline outline-2 outline-accent',
)}
```

Change to:
```tsx
className={cn(
  'absolute h-4 top-1 min-w-[3px]',
  barClass,
  isSelected && 'outline outline-2 outline-accent',
)}
```

- [ ] **Step 3: Run typecheck**

```bash
cd console/web && pnpm typecheck
```

Expected: clean.

- [ ] **Step 4: Run tests**

```bash
cd console/web && pnpm test
```

Expected: 159 tests pass.

- [ ] **Step 5: Commit**

```bash
git add console/web/src/pages/Traces/components/WaterfallChart.tsx
git commit -m "fix(waterfall): strip row+bar CSS transitions

The previous \`transition-colors\` on the row chrome and
\`transition-all duration-150\` on the timing bar fired every time
a row entered or left the virtualization slice — at ~120Hz trackpad
scroll that's constant low-amplitude flicker. The chevron's
\`transition-transform\` is kept; it animates on click, not on
scroll."
```

---

## Task 5: Swap virtualization to `@tanstack/react-virtual`

The big task. Remove the hand-rolled virtualization plumbing and let the library own scroll state, container height tracking, and per-row positioning. The minimap (which previously read `containerHeight` and `scrollPosition`) is rewired: the show/hide condition uses a small `useState`+ResizeObserver pair (kept locally), and the thumb position updates via a direct ref-based scroll listener so it tracks scrollTop without forcing a chart re-render on every wheel tick.

**Files:**
- Modify: `console/web/src/pages/Traces/components/WaterfallChart.tsx`

- [ ] **Step 1: Add the `useVirtualizer` import.**

At the top of `WaterfallChart.tsx`, add:
```tsx
import { useVirtualizer } from '@tanstack/react-virtual'
```

(Insert it alphabetically near the other `@tanstack` import — `@tanstack/react-query` is already imported in the SessionDetailPanel; in this file it's a fresh import.)

- [ ] **Step 2: Remove `scrollPosition` from the reducer.**

In the `DisplayState` interface (around L60-65):
```tsx
interface DisplayState {
  expandedIds: Set<string>
  showCriticalPath: boolean
  scrollPosition: number
}
```

Change to:
```tsx
interface DisplayState {
  expandedIds: Set<string>
  showCriticalPath: boolean
}
```

In the `DisplayAction` union (around L67-71), delete the line:
```tsx
  | { type: 'SET_SCROLL'; position: number }
```

In `initialDisplayState`, delete the `scrollPosition: 0,` line.

In the reducer, delete the entire `case 'SET_SCROLL':` block (it's the last one).

In the destructure on the line that previously was:
```tsx
const { expandedIds, showCriticalPath, scrollPosition } = displayState
```
Change to:
```tsx
const { expandedIds, showCriticalPath } = displayState
```

- [ ] **Step 3: Remove the `handleScroll` block and the rAF refs.**

Find the rAF scroll throttle block (the comment starts with "rAF-throttled scroll. The native `scroll` event…", around L280-L305) and delete it in its entirety: the `scrollRafRef` declaration, the `pendingScrollRef` declaration, the `handleScroll` function, and the `useEffect` cleanup that cancels the pending rAF.

- [ ] **Step 4: Replace the `containerHeight` state with a `useVirtualizer` setup.**

Find these lines (around L156-L172):
```tsx
const containerRef = useRef<HTMLDivElement>(null)
const [containerHeight, setContainerHeight] = useState(0)

useEffect(() => {
  const el = containerRef.current
  if (!el) return
  setContainerHeight(el.clientHeight)
  const obs = new ResizeObserver((entries) => {
    for (const entry of entries) {
      setContainerHeight(entry.contentRect.height)
    }
  })
  obs.observe(el)
  return () => obs.disconnect()
}, [])
```

Replace the entire block (the `containerHeight` useState AND the resize observer useEffect) with:
```tsx
const containerRef = useRef<HTMLDivElement | null>(null)

// containerHeight is kept locally only to decide whether to show
// the minimap. The library owns the heavy lifting of scroll
// position + visible-range tracking.
const [containerHeight, setContainerHeight] = useState(0)
useEffect(() => {
  const el = containerRef.current
  if (!el) return
  setContainerHeight(el.clientHeight)
  const obs = new ResizeObserver((entries) => {
    for (const entry of entries) {
      setContainerHeight(entry.contentRect.height)
    }
  })
  obs.observe(el)
  return () => obs.disconnect()
}, [])
```

(Note: the block stays nearly identical. The change is in subsequent steps — we keep `containerHeight` for the minimap, but the virtualizer is the one tracking scroll for the row list.)

- [ ] **Step 5: Wire `useVirtualizer` after `visibleSpans` is computed.**

Just below the `visibleSpans = useMemo(...)` block (around L270), add:
```tsx
const virtualizer = useVirtualizer({
  count: visibleSpans.length,
  getScrollElement: () => containerRef.current,
  estimateSize: () => ROW_HEIGHT,
  overscan: 16,
})
const virtualRows = virtualizer.getVirtualItems()
const contentHeight = virtualizer.getTotalSize()
```

- [ ] **Step 6: Remove the old `contentHeight`/`viewportRatio`/`thumbHeight`/`thumbPosition` derived values.**

Find this block (was around L296-L307, may have shifted after earlier edits):
```tsx
const contentHeight = visibleSpans.length * ROW_HEIGHT
const viewportRatio =
  containerHeight > 0 && contentHeight > 0
    ? containerHeight / contentHeight
    : 1
const thumbHeight = Math.max(20, MINIMAP_HEIGHT * viewportRatio)
const thumbPosition =
  contentHeight > 0 ? (scrollPosition / contentHeight) * MINIMAP_HEIGHT : 0
```

Delete the entire block. `contentHeight` is now `virtualizer.getTotalSize()` (already computed in Step 5). `thumbHeight` and `thumbPosition` will be reconstructed in the next step via a ref-based scroll listener for the minimap (since `scrollPosition` no longer exists in state).

- [ ] **Step 7: Replace the static `thumbPosition` style with a ref-driven thumb.**

Find the minimap thumb element (around the `top: thumbPosition, height: thumbHeight` style, was around L640-L650). The block looks like:
```tsx
<div
  className="absolute left-0 right-0 bg-accent/20 border border-accent"
  style={{
    top: thumbPosition,
    height: thumbHeight,
  }}
/>
```

Replace with:
```tsx
<div
  ref={thumbRef}
  className="absolute left-0 right-0 bg-accent/20 border border-accent"
  style={{ top: 0, height: 20 }}
/>
```

The thumb starts at the top with a 20px height (the floor used before). A useEffect will keep it synced to scroll.

Add `thumbRef` to the component (near `containerRef`, around L156):
```tsx
const thumbRef = useRef<HTMLDivElement | null>(null)
```

Add a useEffect AFTER the virtualizer setup that wires the thumb to native scroll:
```tsx
// The minimap thumb tracks the scroll container directly so it
// stays glued to the user's scrollTop without forcing a React
// re-render of the whole chart on every wheel tick. The
// virtualizer already exposes contentHeight via getTotalSize(),
// so we read it on each rAF frame from the same source.
useEffect(() => {
  const el = containerRef.current
  const thumb = thumbRef.current
  if (!el || !thumb) return
  let raf: number | null = null
  const update = () => {
    raf = null
    const scrollHeight = el.scrollHeight
    if (scrollHeight <= 0) return
    const top =
      (el.scrollTop / scrollHeight) * MINIMAP_HEIGHT
    const height = Math.max(
      20,
      MINIMAP_HEIGHT * (el.clientHeight / scrollHeight),
    )
    thumb.style.top = `${top}px`
    thumb.style.height = `${height}px`
  }
  const onScroll = () => {
    if (raf !== null) return
    raf = requestAnimationFrame(update)
  }
  update()
  el.addEventListener('scroll', onScroll, { passive: true })
  const obs = new ResizeObserver(update)
  obs.observe(el)
  return () => {
    el.removeEventListener('scroll', onScroll)
    obs.disconnect()
    if (raf !== null) cancelAnimationFrame(raf)
  }
}, [])
```

- [ ] **Step 8: Replace the slice render with `virtualRows`.**

Find the slice render (the block currently between `<div style={{ height: contentHeight, position: 'relative' }}>` and the closing `</div>` of that block). It currently looks roughly like:
```tsx
<div style={{ height: contentHeight, position: 'relative' }}>
  <div style={{ transform: `translateY(${virtualWindow.offsetY}px)` }}>
    {visibleSlice.map((span) => {
      return (
        <WaterfallRow ... />
      )
    })}
  </div>
</div>
```

Replace the entire block with:
```tsx
<div style={{ height: contentHeight, position: 'relative' }}>
  {virtualRows.map((vrow) => {
    const span = visibleSpans[vrow.index]
    if (!span) return null
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
          onSpanClick={onSpanClick}
          onToggleExpand={toggleExpand}
        />
      </div>
    )
  })}
</div>
```

The `if (!span) return null` is a safety net — if `visibleSpans.length` shrinks between the virtualizer's last measurement and our render, the indices may transiently overshoot the array.

- [ ] **Step 9: Remove the `computeVirtualWindow` import + invocation.**

At the top of the file, delete:
```tsx
import { computeVirtualWindow } from '../lib/virtualWindow'
```

Inside `WaterfallChart`, delete the block (was around L335-L348):
```tsx
const virtualWindow = computeVirtualWindow({
  scrollTop: scrollPosition,
  containerHeight,
  rowHeight: ROW_HEIGHT,
  itemCount: visibleSpans.length,
})
const visibleSlice = visibleSpans.slice(
  virtualWindow.startIndex,
  virtualWindow.endIndex,
)
```

- [ ] **Step 10: Remove the `onScroll={handleScroll}` from the scroll container.**

Find the scroll container (around L443-L448):
```tsx
<div
  ref={containerRef}
  style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
  className="flex-1 overflow-y-auto"
  onScroll={handleScroll}
>
```

Change to:
```tsx
<div
  ref={containerRef}
  style={{ '--span-col-width': `${spanColWidth}px` } as React.CSSProperties}
  className="flex-1 overflow-y-auto"
>
```

(Drop the `onScroll` line.)

- [ ] **Step 11: Run typecheck**

```bash
cd console/web && pnpm typecheck
```

Expected: clean. If there are unused imports flagged (likely `computeVirtualWindow`), remove them.

- [ ] **Step 12: Run tests**

```bash
cd console/web && pnpm test
```

Expected: 159 tests pass. The virtualization swap doesn't touch any code that's unit-tested today — all tests cover `lib/*.ts` (pure data transforms), not the WaterfallChart component.

- [ ] **Step 13: Commit**

```bash
git add console/web/src/pages/Traces/components/WaterfallChart.tsx
git commit -m "feat(waterfall): swap hand-rolled virtualization for @tanstack/react-virtual

The hand-rolled fixed-height virtualization (lib/virtualWindow.ts)
translated the whole slice as a single block, so toggling
'hide engine routing' / 'collapse routing pairs' produced a visible
jump as offsetY snapped to the new value. Per-row absolute
positioning via useVirtualizer eliminates that — rows slide to
their new top in a single composited paint.

Scroll state no longer enters React. The library uses passive
native scroll listeners and a ResizeObserver internally, and
produces a fresh VirtualItem[] only when the visible range
actually changes. Removed DisplayState.scrollPosition, SET_SCROLL,
handleScroll, the rAF refs, and the scroll-cleanup useEffect.

The minimap kept its own ResizeObserver-driven containerHeight
useState (cheap, only fires on viewport resize) and gained a
ref-based thumb that tracks scrollTop directly via a passive
listener — that way scroll doesn't force a React re-render even
for the thumb.

Overscan bumped from 8 to 16 rows so fast trackpad flicks no
longer expose the edge of the rendered window."
```

---

## Task 6: Delete `lib/virtualWindow.{ts,test.ts}`

**Files:**
- Delete: `console/web/src/pages/Traces/lib/virtualWindow.ts`
- Delete: `console/web/src/pages/Traces/lib/virtualWindow.test.ts`

- [ ] **Step 1: Confirm nothing imports the helper anymore.**

```bash
grep -rn "virtualWindow\|computeVirtualWindow" console/web/src/
```

Expected output: zero matches. If anything remains, fix the import before deleting.

- [ ] **Step 2: Delete the files.**

```bash
git rm console/web/src/pages/Traces/lib/virtualWindow.ts console/web/src/pages/Traces/lib/virtualWindow.test.ts
```

Expected output:
```
rm 'console/web/src/pages/Traces/lib/virtualWindow.test.ts'
rm 'console/web/src/pages/Traces/lib/virtualWindow.ts'
```

- [ ] **Step 3: Run typecheck**

```bash
cd console/web && pnpm typecheck
```

Expected: clean.

- [ ] **Step 4: Run tests**

```bash
cd console/web && pnpm test
```

Expected: test count drops from 159 to 151 (8 cases removed with `virtualWindow.test.ts`), all pass.

- [ ] **Step 5: Commit**

```bash
git commit -m "chore(traces): delete lib/virtualWindow now that the lib swap landed

The pure compute helper is dead code after the @tanstack/react-virtual
swap. virtualWindow.test.ts goes with it — the library is
upstream-tested."
```

---

## Task 7: Live verification

This task does not produce a commit. It is the user-facing verification gate.

**Files:** none.

- [ ] **Step 1: Start the dev server.**

```bash
cd console/web && pnpm dev
```

Expected: vite starts and prints a local URL (e.g., `http://localhost:5173`).

- [ ] **Step 2: Open the traces page in a browser.**

Navigate to the URL printed by vite, then open the traces page (path varies by build; the route is `/traces` or a session-detail route off it). Find a trace with 2000+ spans (the user has a 2389-span trace they were testing with).

- [ ] **Step 3: Verify scrolling.**

Scroll the waterfall with both trackpad and wheel. Expected: smooth motion, no jank, no blank rows at the leading edge during fast scroll.

- [ ] **Step 4: Verify hover.**

Move the mouse across rows while the chart is at rest. Expected: hover highlight is instant (CSS-driven), no flicker. Move the mouse across rows while scrolling. Expected: no color flicker on rows entering the viewport.

- [ ] **Step 5: Verify filter toggles.**

Click the `hide engine routing` checkbox. Expected: rows reposition smoothly to their new top values, no visible mid-screen jump of the whole slice.
Click the `collapse routing pairs` checkbox. Expected: same — smooth reposition.

- [ ] **Step 6: Verify expand/collapse.**

Click the chevron on a span with children. Expected: descendants appear smoothly. The clicked row stays where it was; rows below it shift down.

- [ ] **Step 7: Verify selection.**

Click a span row. Expected: the row gets the `bg-panel border-l-2 border-l-accent` chrome, the span detail panel opens or updates to that span. Keyboard: Tab to a row, press Enter or Space, same behavior.

- [ ] **Step 8: Verify column resize.**

Drag the resize handle between the SPAN column and the timeline area. Expected: column width changes smoothly, no per-row jank, the time-axis header tracks the new width.

- [ ] **Step 9: Verify minimap thumb.**

Confirm the right-side minimap shows the trace overview and the thumb stays glued to your scroll position as you scroll up and down.

- [ ] **Step 10: Run typecheck + tests one last time on the final commit.**

```bash
cd console/web && pnpm typecheck && pnpm test
```

Expected: typecheck clean, 151 tests pass.

- [ ] **Step 11: Push and open / update the PR.**

```bash
git push
```

If pushing to `fix/traces-waterfall-freeze`, PR #158 picks it up automatically. If a separate PR is preferred, push to a fresh branch first:
```bash
git push -u origin perf/traces-waterfall-virtual-lib
gh pr create --base main --head perf/traces-waterfall-virtual-lib \
  --title "perf(traces): swap waterfall virtualization to @tanstack/react-virtual" \
  --body "Closes the three glitches on PR #158 follow-up. Spec at docs/superpowers/specs/2026-05-19-waterfall-virtualization-lib-swap-design.md; plan at docs/superpowers/plans/2026-05-19-waterfall-virtualization-lib-swap.md."
```

---

## Self-review

- **Spec coverage.** Every spec section maps to a task: dep install (T1), memo row + useCallback (T2), CSS variable for column width (T3), strip transitions (T4), library swap + minimap rewire (T5), delete dead helper (T6), live verification (T7). The spec's "out of scope" items (spanTree iterative rewrite, FlameGraph virtualization, scroll-to-span on selection) are NOT in any task — that's intentional.
- **Placeholders.** None. Every code step shows the exact code; every command step shows the exact expected output.
- **Type consistency.** `WaterfallRow` props match between Task 2 and Task 3 (Task 3 drops `spanColWidth`). `toggleExpand` is renamed in Task 2's prop list to `onToggleExpand` on the row side; the parent's local name stays `toggleExpand`. Confirmed consistent in Task 5 Step 8 where the row is used. `onSpanClick` matches the parent's `(span: VisualizationSpan) => void` signature throughout — `FlatSpanRow` extends `VisualizationSpan` (see `lib/spanTree.ts` line 18), so passing a `FlatSpanRow` to a handler expecting `VisualizationSpan` is sound.
- **Risk on Task 5.** This is the biggest task. The minimap thumb rewire is the most likely failure point — if the ref-based scroll listener doesn't fire (e.g. wrong ref target), the thumb sits at the top forever. The live verification step 9 catches that.
