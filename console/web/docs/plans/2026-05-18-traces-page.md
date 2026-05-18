# Traces Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the traces page from `motia/console/packages/console-frontend` to `workers/console/web` and re-skin every component onto the iii Schematic design system (`DESIGN.md`).

**Architecture:** Add a `'traces'` view to the existing hash router in `workers/console/web`. Reuse the existing `getIiiClient()` singleton (`src/lib/iii-client.ts`) for engine RPC — the trace functions (`engine::traces::list|tree|clear|group_by`) are called via `client.call(functionId, payload)`, the same generic SDK trigger already used by the chat backend. Port pure data utilities verbatim from `motia/console`, port React components with a token + structural swap onto the schematic primitives that already exist (`Button`, `StatusDot`, `ModeToggle`, `Prompt`, `Sheet`, `Caret`) plus a small set of new ones (`Cell`, `StatusPanel`, `Tabs`, `Tooltip`, `Pagination`, `Skeleton`, `EmptyState`, `ErrorBoundary`, `Badge`).

**Tech Stack:** React 19, TypeScript, Tailwind v4 (with the schematic `@theme` block already in `src/index.css`), `iii-browser-sdk@0.12.0`, `@tanstack/react-query` (new), `@xyflow/react` + `dagre` (new), Radix `Tabs` + `Tooltip` (new), `lucide-react` (new), `zod` (new). vitest for unit tests.

**Source paths used throughout:**
- Source page: `motia/console/packages/console-frontend/src/routes/traces.tsx`
- Source components: `motia/console/packages/console-frontend/src/components/traces/*.tsx`
- Source hooks: `motia/console/packages/console-frontend/src/hooks/{useTraceData,useTraceFilters,useTraceGroups,useResizablePanels}.ts`
- Source lib: `motia/console/packages/console-frontend/src/lib/*.ts`
- Source API: `motia/console/packages/console-frontend/src/api/observability/traces.ts`

**Token swap rules (apply to every ported file):**

| motia                                  | schematic                                                          |
| -------------------------------------- | ------------------------------------------------------------------ |
| `bg-background`                        | `bg-bg`                                                            |
| `bg-foreground`                        | `bg-ink`                                                           |
| `bg-sidebar`, `bg-dark-gray`           | `bg-panel`                                                         |
| `bg-dark-gray/30`, `bg-dark-gray/50`   | `bg-panel`                                                         |
| `bg-primary/10`                        | `bg-panel` + `border-l-2 border-l-accent`                          |
| `text-foreground`                      | `text-ink`                                                         |
| `text-muted`, `text-mute`              | `text-ink-faint`                                                   |
| `text-muted-foreground`                | `text-ink-ghost`                                                   |
| `text-primary`                         | `text-accent`                                                      |
| `text-success`, `bg-success/x`         | `text-accent`                                                      |
| `text-warning`, `bg-warning/x`         | `text-warn`                                                        |
| `text-error`, `bg-error/x`             | `text-alert`                                                       |
| `text-yellow` + `animate-pulse`        | drop the color, use `StatusDot tone="warn" pulse`                  |
| `border-border`                        | `border-rule`                                                      |
| `border-border-subtle`                 | `border-rule-2`                                                    |
| any `rounded-*` (except `rounded-full`)| remove (rectilinear only)                                          |
| any `shadow-*`                         | remove (1px rules define structure)                                |
| `font-sans` (proportional)             | drop (`font-sans` and `font-mono` both resolve to Chivo Mono)      |

**Lowercase rule:** every user-facing string must be lowercase except UPPERCASE label-caps. Audit during each task.

---

## Phase 0: pre-flight checks

### Task 0.1: Confirm engine connection works

**Files:** _(no file changes — verification only)_

- [ ] **Step 1: Start the dev server**

Run: `cd workers/console/web && pnpm dev`
Expected: Vite boots, app reachable at the printed URL (default `http://localhost:5173`).

- [ ] **Step 2: Verify engine connectivity**

Open the URL in a browser, open DevTools console. Expected: no errors about the WebSocket connection. If you see `WebSocket connection to 'ws://.../iii/ws' failed`, ensure an engine is running and the Vite proxy is configured (check `vite.config.ts`). You cannot proceed without an engine reachable from the dev server.

- [ ] **Step 3: Verify the trace exporter is registered (manual)**

In the browser DevTools console, paste:

```js
const { getIiiClient } = await import('/src/lib/iii-client.ts')
const client = await getIiiClient()
try {
  const res = await client.call('engine::traces::list', { limit: 1 })
  console.log('traces enabled:', res)
} catch (err) {
  console.error('traces not enabled:', err)
}
```

Expected: either a response object (good — observability is enabled) or a "function not registered" rejection (acceptable — the plan handles this via an empty state).

Record which case applies in your scratch notes — Phase 2 verification will use this information.

---

## Phase 1: Plumbing — route, deps, primitives, empty page

### Task 1.1: Install new runtime dependencies

**Files:**
- Modify: `workers/console/web/package.json`

- [ ] **Step 1: Install dependencies**

Run:
```bash
cd workers/console/web
pnpm add @tanstack/react-query @radix-ui/react-tabs @radix-ui/react-tooltip @xyflow/react dagre lucide-react zod
pnpm add -D @types/dagre
```

Expected: 8 new packages installed, `package.json` updated, `pnpm-lock.yaml` updated. The existing deps `clsx`, `tailwind-merge`, `class-variance-authority`, `@radix-ui/react-slot` stay as-is — already present.

- [ ] **Step 2: Verify typecheck still passes**

Run: `pnpm typecheck`
Expected: PASS (no broken types from the new deps).

- [ ] **Step 3: Commit**

```bash
git add package.json pnpm-lock.yaml
git commit -m "feat: add traces page runtime deps"
```

### Task 1.2: Add `traces` to the View hash route

**Files:**
- Modify: `workers/console/web/src/hooks/use-hash-route.ts`

- [ ] **Step 1: Update the `View` union and route resolver**

Replace the contents of `src/hooks/use-hash-route.ts` with:

```ts
import { useCallback, useEffect, useRef, useState } from 'react'

export type View = 'chat' | 'examples' | 'playground' | 'traces'

const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

function routeFromHash(hash: string): View | null {
  if (hash === '' || hash === '#' || hash === '#/' || hash === '#/chat') {
    return 'chat'
  }
  if (hash === '#/traces') return 'traces'
  if (hash === '#/examples') return PLAYGROUND_ENABLED ? 'examples' : 'chat'
  if (hash === '#/playground') return PLAYGROUND_ENABLED ? 'playground' : 'chat'
  return null
}

export function useHashRoute(): [View, (next: View) => void] {
  const [view, setView] = useState<View>(() => {
    if (typeof window === 'undefined') return 'chat'
    return routeFromHash(window.location.hash) ?? 'chat'
  })
  const viewRef = useRef(view)
  viewRef.current = view

  useEffect(() => {
    const handle = () => {
      const next = routeFromHash(window.location.hash)
      if (next !== null && next !== viewRef.current) setView(next)
    }
    window.addEventListener('hashchange', handle)
    return () => window.removeEventListener('hashchange', handle)
  }, [])

  const navigate = useCallback((next: View) => {
    const targetHash =
      next === 'chat'
        ? '#/'
        : next === 'traces'
          ? '#/traces'
          : next === 'examples'
            ? '#/examples'
            : '#/playground'
    if (window.location.hash !== targetHash) {
      window.location.hash = targetHash
    } else {
      setView(next)
    }
  }, [])

  return [view, navigate]
}
```

- [ ] **Step 2: Verify typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/use-hash-route.ts
git commit -m "feat(traces): add 'traces' to View union"
```

### Task 1.3: Create the `Cell` primitive

**Files:**
- Create: `workers/console/web/src/components/ui/Cell.tsx`

- [ ] **Step 1: Write the file**

```tsx
import type * as React from 'react'
import { cn } from '@/lib/utils'

interface CellProps {
  title?: React.ReactNode
  children: React.ReactNode
  className?: string
}

export function Cell({ title, children, className }: CellProps) {
  return (
    <div className={cn('border border-rule bg-bg p-7', className)}>
      {title ? (
        <div className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink mb-3 lowercase">
          {title}
        </div>
      ) : null}
      <div className="font-mono text-[13px] leading-[1.7] text-ink-faint max-w-[34ch]">
        {children}
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Verify typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/components/ui/Cell.tsx
git commit -m "feat(ui): add Cell schematic primitive"
```

### Task 1.4: Create the `StatusPanel` primitive

**Files:**
- Create: `workers/console/web/src/components/ui/StatusPanel.tsx`

- [ ] **Step 1: Write the file**

```tsx
import type * as React from 'react'
import { cn } from '@/lib/utils'

export type StatusVariant = 'info' | 'success' | 'warn' | 'alert'

const variantTone: Record<
  StatusVariant,
  { border: string; icon: string; headline: string }
> = {
  info: {
    border: 'border-rule',
    icon: 'text-ink',
    headline: 'text-ink',
  },
  success: {
    border: 'border-accent',
    icon: 'text-accent',
    headline: 'text-accent',
  },
  warn: {
    border: 'border-warn',
    icon: 'text-warn',
    headline: 'text-warn',
  },
  alert: {
    border: 'border-alert',
    icon: 'text-alert',
    headline: 'text-alert',
  },
}

interface StatusPanelProps {
  variant?: StatusVariant
  icon?: React.ReactNode
  headline: React.ReactNode
  detail?: React.ReactNode
  className?: string
}

export function StatusPanel({
  variant = 'info',
  icon,
  headline,
  detail,
  className,
}: StatusPanelProps) {
  const tone = variantTone[variant]
  return (
    <div
      className={cn(
        'flex items-start gap-x-3 border bg-bg px-3.5 py-3',
        tone.border,
        className,
      )}
    >
      {icon ? (
        <span aria-hidden className={cn('size-[18px] shrink-0', tone.icon)}>
          {icon}
        </span>
      ) : null}
      <div className="min-w-0 flex flex-col gap-y-0.5">
        <div
          className={cn(
            'font-mono text-[13px] font-semibold lowercase',
            tone.headline,
          )}
        >
          {headline}
        </div>
        {detail ? (
          <div className="font-mono text-[12px] text-ink-faint lowercase">
            {detail}
          </div>
        ) : null}
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Verify typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/components/ui/StatusPanel.tsx
git commit -m "feat(ui): add StatusPanel schematic primitive"
```

### Task 1.5: Create the empty `TracesPage` shell

**Files:**
- Create: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Write the file**

```tsx
import { GitBranch } from 'lucide-react'
import { Cell } from '@/components/ui/Cell'

export function Traces() {
  return (
    <section className="flex-1 flex flex-col overflow-hidden">
      <div className="px-9 py-12">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-3">
          <span className="text-accent">$</span>{' '}
          <span className="text-ink ml-2">traces</span>
        </div>
        <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
          traces
        </h1>
      </div>
      <div className="px-9 pb-12">
        <Cell title="not wired yet">
          the traces page is under construction. once the data layer is in
          place, this surface will list traces emitted by the engine and let
          you drill into spans, sessions, and call graphs.
        </Cell>
      </div>
      <div aria-hidden className="text-ink-ghost hidden">
        <GitBranch />
      </div>
    </section>
  )
}
```

(The hidden `GitBranch` import keeps `lucide-react` referenced so typecheck doesn't flag it as unused; Phase 3 wires it visibly.)

- [ ] **Step 2: Verify typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): scaffold empty TracesPage"
```

### Task 1.6: Wire `Traces` into `App.tsx`

**Files:**
- Modify: `workers/console/web/src/App.tsx`

- [ ] **Step 1: Update the file**

Replace the contents of `src/App.tsx` with:

```tsx
import { lazy, Suspense } from 'react'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { Sheet } from '@/components/ui/Sheet'
import { Wordmark } from '@/components/ui/Wordmark'
import { useHashRoute, type View } from '@/hooks/use-hash-route'
import { type Theme, useTheme } from '@/hooks/use-theme'
import { Chat } from '@/pages/Chat'
import { Traces } from '@/pages/Traces'

const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

const Examples = PLAYGROUND_ENABLED
  ? lazy(() =>
      import('@/pages/Examples').then((m) => ({ default: m.Examples })),
    )
  : null

const Playground = PLAYGROUND_ENABLED
  ? lazy(() =>
      import('@/pages/Playground').then((m) => ({ default: m.Playground })),
    )
  : null

const VIEW_OPTIONS: { value: View; label: string }[] = PLAYGROUND_ENABLED
  ? [
      { value: 'chat', label: 'chat' },
      { value: 'traces', label: 'traces' },
      { value: 'playground', label: 'playground' },
      { value: 'examples', label: 'examples' },
    ]
  : [
      { value: 'chat', label: 'chat' },
      { value: 'traces', label: 'traces' },
    ]

export function App() {
  const [theme, setTheme] = useTheme()
  const [view, setView] = useHashRoute()

  return (
    <Sheet>
      <Header
        view={view}
        onViewChange={setView}
        theme={theme}
        onThemeChange={setTheme}
      />
      <Suspense fallback={<RouteFallback />}>
        {view === 'traces' ? (
          <Traces />
        ) : view === 'examples' && Examples ? (
          <Examples />
        ) : view === 'playground' && Playground ? (
          <Playground />
        ) : (
          <Chat />
        )}
      </Suspense>
    </Sheet>
  )
}

interface HeaderProps {
  view: View
  onViewChange: (next: View) => void
  theme: Theme
  onThemeChange: (next: Theme) => void
}

function Header({ view, onViewChange, theme, onThemeChange }: HeaderProps) {
  return (
    <header className="flex items-center justify-between px-6 h-12 border-b border-rule shrink-0">
      <div className="flex items-center gap-3">
        <Wordmark />
        <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
          {view}
        </span>
      </div>
      <div className="flex items-center gap-3">
        <ModeToggle<View>
          value={view}
          onChange={onViewChange}
          options={VIEW_OPTIONS}
        />
        <ModeToggle<Theme>
          value={theme}
          onChange={onThemeChange}
          options={[
            { value: 'light', label: 'light' },
            { value: 'dark', label: 'dark' },
          ]}
        />
      </div>
    </header>
  )
}

function RouteFallback() {
  return (
    <section className="flex-1 flex items-center justify-center">
      <div className="font-mono text-[12px] uppercase tracking-[0.18em] text-ink-ghost">
        loading…
      </div>
    </section>
  )
}
```

- [ ] **Step 2: Verify typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Manual verification in browser**

Start the dev server (`pnpm dev`). In the browser:
1. Confirm the header shows `chat | traces` toggle (light/dark on the right).
2. Click `traces` → URL changes to `#/traces`, page shows the schematic header + `Cell` placeholder.
3. Click `chat` → returns to the chat view.
4. Refresh on `#/traces` → still renders the traces page (hash route persistence).
5. Dark theme: toggle to dark, the trace page palette inverts (cream → near-black, orange → blue accent).

Take a screenshot for the PR.

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(traces): wire Traces view into app header"
```

---

## Phase 2: Data layer — adapter, query client, raw list

### Task 2.1: Wrap the app with a `QueryClientProvider`

**Files:**
- Modify: `workers/console/web/src/main.tsx`

- [ ] **Step 1: Update the file**

Replace the contents of `src/main.tsx` with:

```tsx
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import './index.css'

const root = document.getElementById('root')
if (!root) throw new Error('missing #root container')

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Trace data is live; refetch on mount is fine, but don't thrash on
      // window focus while the user is reading a waterfall.
      refetchOnWindowFocus: false,
      retry: 1,
      staleTime: 1_000,
    },
  },
})

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
```

- [ ] **Step 2: Verify typecheck + boot**

Run: `pnpm typecheck` (expect PASS), then `pnpm dev` and load the app. Expect no console errors. Chat must still work — open chat, send a message, verify the chat backend still streams.

- [ ] **Step 3: Commit**

```bash
git add src/main.tsx
git commit -m "feat: add QueryClientProvider"
```

### Task 2.2: Port pure lib utilities

**Files:**
- Create: `workers/console/web/src/pages/Traces/lib/spanTree.ts`
- Create: `workers/console/web/src/pages/Traces/lib/spanTree.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceTransform.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceTransform.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceFilters.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceFilters.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceListItem.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceListItem.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceUtils.ts`
- Create: `workers/console/web/src/pages/Traces/lib/traceUtils.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/groupTraces.ts`
- Create: `workers/console/web/src/pages/Traces/lib/groupTraces.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/spanLabel.ts`
- Create: `workers/console/web/src/pages/Traces/lib/spanLabel.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/formatPossibleJson.ts`
- Create: `workers/console/web/src/pages/Traces/lib/formatPossibleJson.test.ts`
- Create: `workers/console/web/src/pages/Traces/lib/otel-utils.ts`
- Create: `workers/console/web/src/pages/Traces/lib/timeRangeUtils.ts`

- [ ] **Step 1: Copy each source file verbatim**

For each file in this list, copy the source from `motia/console/packages/console-frontend/src/lib/` to the destination `workers/console/web/src/pages/Traces/lib/`:

| source file                                  | destination file                       |
| -------------------------------------------- | -------------------------------------- |
| `spanTree.ts`                                | `spanTree.ts`                          |
| `spanTree.test.ts`                           | `spanTree.test.ts`                     |
| `traceTransform.ts`                          | `traceTransform.ts`                    |
| `traceTransform.test.ts`                     | `traceTransform.test.ts`               |
| `traceFilters.ts`                            | `traceFilters.ts`                      |
| `traceFilters.test.ts`                       | `traceFilters.test.ts`                 |
| `traceListItem.ts`                           | `traceListItem.ts`                     |
| `traceListItem.test.ts`                      | `traceListItem.test.ts`                |
| `traceUtils.ts`                              | `traceUtils.ts`                        |
| `traceUtils.test.ts`                         | `traceUtils.test.ts`                   |
| `groupTraces.ts`                             | `groupTraces.ts`                       |
| `groupTraces.test.ts`                        | `groupTraces.test.ts`                  |
| `spanLabel.ts`                               | `spanLabel.ts`                         |
| `spanLabel.test.ts`                          | `spanLabel.test.ts`                    |
| `formatPossibleJson.ts`                      | `formatPossibleJson.ts`                |
| `formatPossibleJson.test.ts`                 | `formatPossibleJson.test.ts`           |
| `otel-utils.ts`                              | `otel-utils.ts`                        |
| `timeRangeUtils.ts`                          | `timeRangeUtils.ts`                    |

Use `cp` for each:

```bash
SRC=/Users/andersonleal/projetos/motia/motia/console/packages/console-frontend/src/lib
DST=/Users/andersonleal/projetos/motia/workers/console/web/src/pages/Traces/lib
for f in spanTree spanTree.test traceTransform traceTransform.test \
         traceFilters traceFilters.test traceListItem traceListItem.test \
         traceUtils traceUtils.test groupTraces groupTraces.test \
         spanLabel spanLabel.test formatPossibleJson formatPossibleJson.test \
         otel-utils timeRangeUtils; do
  cp "$SRC/$f.ts" "$DST/$f.ts"
done
```

- [ ] **Step 2: Fix import paths**

These files use bare `@/` aliases that may point at the motia source layout. The schema is `@/lib/...` and `@/api/...`. After copying, search-and-replace within the `pages/Traces/lib/` directory:

```bash
cd /Users/andersonleal/projetos/motia/workers/console/web/src/pages/Traces/lib
# These libs only depend on each other and on types. Rewrite any
# '@/lib/' imports that reference sibling files to relative imports:
grep -l "@/lib/" *.ts | while read f; do
  sed -i '' "s|@/lib/spanTree|./spanTree|g; \
             s|@/lib/traceTransform|./traceTransform|g; \
             s|@/lib/traceFilters|./traceFilters|g; \
             s|@/lib/traceListItem|./traceListItem|g; \
             s|@/lib/traceUtils|./traceUtils|g; \
             s|@/lib/groupTraces|./groupTraces|g; \
             s|@/lib/spanLabel|./spanLabel|g; \
             s|@/lib/formatPossibleJson|./formatPossibleJson|g; \
             s|@/lib/otel-utils|./otel-utils|g; \
             s|@/lib/timeRangeUtils|./timeRangeUtils|g; \
             s|@/lib/traceColors|./traceColors|g" "$f"
done
```

If any file imports from `@/api/observability/traces`, rewrite that to `../api/traces` (which we'll create in Task 2.4):

```bash
sed -i '' "s|@/api/observability/traces|../api/traces|g" *.ts
```

- [ ] **Step 3: Run the tests**

Add vitest to the project if it isn't already present:

```bash
cd /Users/andersonleal/projetos/motia/workers/console/web
pnpm add -D vitest
```

Add a `test` script to `package.json` if missing:

```json
"test": "vitest run",
"test:watch": "vitest"
```

Run:
```bash
pnpm test src/pages/Traces/lib
```

Expected: all ported tests PASS. If any fail, the most likely cause is an import-path miss in Step 2 — re-check.

- [ ] **Step 4: Commit**

```bash
git add src/pages/Traces/lib package.json pnpm-lock.yaml
git commit -m "feat(traces): port pure lib utilities with tests"
```

### Task 2.3: Port `traceColors.ts` with schematic palette mapping

**Files:**
- Create: `workers/console/web/src/pages/Traces/lib/traceColors.ts`

- [ ] **Step 1: Inspect the motia source**

Read `motia/console/packages/console-frontend/src/lib/traceColors.ts` to learn what color map it exposes (typically a `serviceColor(serviceName)` function returning Tailwind class strings).

- [ ] **Step 2: Re-author the file against schematic tokens**

The schematic forbids chromatic palettes; service colors collapse onto a 3-step monochrome ramp keyed off ink with `bg-alert` reserved for error services and `bg-accent` reserved for the single "live"/selected service. Write:

```ts
/**
 * Service color resolution under the iii Schematic.
 *
 * The schematic is a single-accent palette (DESIGN.md §3). Service identity
 * is encoded through ink shades and an optional alert/accent flag — never
 * through chromatic gradients.
 */

const INK_SHADES = [
  'bg-ink',
  'bg-ink/85',
  'bg-ink/70',
  'bg-ink/55',
] as const

const INK_TEXT_SHADES = [
  'text-ink',
  'text-ink-faint',
  'text-ink-ghost',
] as const

function hash(str: string): number {
  let h = 0
  for (let i = 0; i < str.length; i++) {
    h = (h * 31 + str.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

export interface ServiceTone {
  /** Tailwind class for a filled chip/bar (e.g. inside a waterfall). */
  fill: string
  /** Tailwind class for inline text (e.g. service name in a list). */
  text: string
}

export function serviceTone(
  service: string,
  options?: { error?: boolean; selected?: boolean },
): ServiceTone {
  if (options?.error) return { fill: 'bg-alert', text: 'text-alert' }
  if (options?.selected) return { fill: 'bg-accent', text: 'text-accent' }
  const idx = hash(service) % INK_SHADES.length
  const txtIdx = hash(service) % INK_TEXT_SHADES.length
  return { fill: INK_SHADES[idx], text: INK_TEXT_SHADES[txtIdx] }
}

export function statusTone(
  status: 'ok' | 'error' | 'pending',
): { dot: 'accent' | 'alert' | 'warn'; label: string; bar: string } {
  switch (status) {
    case 'ok':
      return { dot: 'accent', label: 'text-accent', bar: 'bg-ink' }
    case 'error':
      return { dot: 'alert', label: 'text-alert', bar: 'bg-alert' }
    case 'pending':
      return { dot: 'warn', label: 'text-warn', bar: 'bg-warn' }
  }
}
```

- [ ] **Step 3: Update consumers**

In every other file under `src/pages/Traces/lib/` that imported the motia `traceColors`, switch to the new export names. The motia file likely exports `getServiceColor`/`getStatusColor`; this rewrite uses `serviceTone`/`statusTone`. Compile errors after this step are how you find every consumer — fix them one by one (the only changes are renames; no logic change).

```bash
pnpm typecheck 2>&1 | grep -E "traceColors|getServiceColor|getStatusColor"
```

For each error, update the call site to the new API:
- `getServiceColor(name)` → `serviceTone(name).fill` (or `.text`, depending on context)
- `getStatusColor(status)` → `statusTone(status).label` (or `.bar`)

- [ ] **Step 4: Run tests**

Run: `pnpm test src/pages/Traces/lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/pages/Traces/lib/traceColors.ts
git commit -m "feat(traces): schematic-aligned trace color palette"
```

### Task 2.4: Port `api/traces.ts` with `getIiiClient()` adapter

**Files:**
- Create: `workers/console/web/src/pages/Traces/api/traces.ts`

- [ ] **Step 1: Read the source**

Read `motia/console/packages/console-frontend/src/api/observability/traces.ts`. Note the four function-ID constants (`engine::traces::list|tree|clear|group_by`), the request/response shapes, and the `ISdk` parameter on each fetcher.

- [ ] **Step 2: Write the adapter**

Create the file with the same TypeScript types and the same RPC paths, but with `getIiiClient()` resolving the SDK internally — no `sdk` parameter on any function:

```ts
/**
 * Observability traces RPC adapter.
 *
 * Mirrors motia's api/observability/traces.ts, but resolves the iii-browser-sdk
 * client from the shared singleton in src/lib/iii-client.ts instead of taking
 * the SDK as a parameter. The engine RPC paths are identical, so the engine
 * contract is unchanged.
 */

import { getIiiClient } from '@/lib/iii-client'

export const OBSERVABILITY_TRACE_FUNCTIONS = {
  list: 'engine::traces::list',
  tree: 'engine::traces::tree',
  clear: 'engine::traces::clear',
  groupBy: 'engine::traces::group_by',
} as const

/* -------- request / response types (port verbatim from motia source) -------- */

export interface TraceListItemDto {
  trace_id: string
  root_operation: string
  status: 'ok' | 'error' | 'pending'
  start_time: number
  duration?: number
  span_count: number
  services: string[]
  function_id?: string
  topic?: string
}

export interface TraceListResponse {
  traces: TraceListItemDto[]
  has_otel_configured: boolean
}

export interface SpanDto {
  // Port the full Span shape from the motia source. Do not abbreviate —
  // every field is consumed downstream by traceTransform.ts.
  trace_id: string
  span_id: string
  parent_span_id?: string
  name: string
  service: string
  start_time_unix_nano: number
  end_time_unix_nano: number
  status_code?: number
  status_message?: string
  attributes?: Record<string, unknown>
  events?: Array<{
    name: string
    time_unix_nano: number
    attributes?: Record<string, unknown>
  }>
  links?: Array<{
    trace_id: string
    span_id: string
    attributes?: Record<string, unknown>
  }>
}

export interface TraceTreeResponse {
  roots: SpanDto[]
}

export interface TraceFilterParams {
  trace_id?: string
  function_id?: string
  service?: string
  status?: 'ok' | 'error' | 'pending'
  start_after?: number
  start_before?: number
  show_system?: boolean
  search?: string
  limit?: number
}

export interface TraceGroup {
  group_value: string
  trace_count: number
  trace_ids: string[]
  // The shape mirrors motia's `TraceGroup` exactly. Copy any additional
  // fields from the source file (e.g. earliest_start, latest_end).
}

export interface TraceGroupsResponse {
  groups: TraceGroup[]
}

/* ------------------------------- fetchers ----------------------------------- */

function asError(err: unknown, fallback: string): Error {
  if (err instanceof Error) return err
  return new Error(fallback)
}

export async function listTraces(
  params: TraceFilterParams,
): Promise<TraceListResponse> {
  const client = await getIiiClient()
  try {
    return await client.call<TraceListResponse>(
      OBSERVABILITY_TRACE_FUNCTIONS.list,
      params as unknown as Record<string, unknown>,
    )
  } catch (err) {
    throw asError(err, 'Failed to fetch traces')
  }
}

export async function fetchTraceTree(
  traceId: string,
): Promise<TraceTreeResponse> {
  const client = await getIiiClient()
  try {
    return await client.call<TraceTreeResponse>(
      OBSERVABILITY_TRACE_FUNCTIONS.tree,
      { trace_id: traceId },
    )
  } catch (err) {
    throw asError(err, 'Failed to fetch trace tree')
  }
}

export async function clearTraces(): Promise<void> {
  const client = await getIiiClient()
  try {
    await client.call(OBSERVABILITY_TRACE_FUNCTIONS.clear, {})
  } catch (err) {
    throw asError(err, 'Failed to clear traces')
  }
}

export async function fetchTraceGroups(
  attribute: string,
  params: TraceFilterParams,
): Promise<TraceGroupsResponse> {
  const client = await getIiiClient()
  try {
    return await client.call<TraceGroupsResponse>(
      OBSERVABILITY_TRACE_FUNCTIONS.groupBy,
      { attribute, ...(params as unknown as Record<string, unknown>) },
    )
  } catch (err) {
    throw asError(err, 'Failed to fetch trace groups')
  }
}
```

**Important:** the `SpanDto` and `TraceGroup` shapes above are starter scaffolds. Open the motia source for these types and copy every field — `traceTransform.ts` (already ported in Task 2.2) reads them by exact name and will fail at runtime if a field is missing. The token swap rules don't apply here (this is pure data).

- [ ] **Step 3: Typecheck**

Run: `pnpm typecheck`
Expected: PASS (with the full types copied from the motia source).

- [ ] **Step 4: Commit**

```bash
git add src/pages/Traces/api/traces.ts
git commit -m "feat(traces): port traces RPC adapter with getIiiClient()"
```

### Task 2.5: Port `useTraceData` hook (with React Query)

**Files:**
- Create: `workers/console/web/src/pages/Traces/hooks/useTraceData.ts`

- [ ] **Step 1: Read the source**

Read `motia/console/packages/console-frontend/src/hooks/useTraceData.ts`. Note: it uses `useEngineSdk()` from `@/api/engine-sdk-provider`, and React Query for the list query plus a refresh/polling cadence.

- [ ] **Step 2: Write the ported file**

Drop the `useEngineSdk()` import and call into `listTraces` from `api/traces.ts` instead. The signature stays the same. Skeleton:

```ts
import { useQuery } from '@tanstack/react-query'
import { useEffect, useMemo, useRef, useState } from 'react'
import { listTraces, type TraceFilterParams } from '../api/traces'
import { traceListItemFromDto, type TraceListItem } from '../lib/traceListItem'

export interface UseTraceDataOptions {
  filterParams: TraceFilterParams
  showSystem: boolean
  debouncedSearch: string
  isPaused: boolean
}

export interface UseTraceDataResult {
  traceGroups: TraceListItem[]
  newTraceIds: Set<string>
  setNewTraceIds: React.Dispatch<React.SetStateAction<Set<string>>>
  hasOtelConfigured: boolean
  isQueryLoading: boolean
  refetch: () => void
  isHoveredRef: React.MutableRefObject<boolean>
  flushPendingTraces: () => void
}

export function useTraceData({
  filterParams,
  showSystem,
  debouncedSearch,
  isPaused,
}: UseTraceDataOptions): UseTraceDataResult {
  const isHoveredRef = useRef(false)
  const [pending, setPending] = useState<TraceListItem[]>([])
  const [accepted, setAccepted] = useState<TraceListItem[]>([])
  const [newTraceIds, setNewTraceIds] = useState<Set<string>>(new Set())

  const queryKey = useMemo(
    () => ['traces', 'list', filterParams, showSystem, debouncedSearch] as const,
    [filterParams, showSystem, debouncedSearch],
  )

  const query = useQuery({
    queryKey,
    queryFn: () => listTraces({ ...filterParams, show_system: showSystem, search: debouncedSearch || undefined }),
    refetchInterval: isPaused ? false : 2_500,
    refetchIntervalInBackground: false,
  })

  // Map DTOs → TraceListItem domain objects.
  const incoming = useMemo<TraceListItem[]>(
    () => (query.data?.traces ?? []).map(traceListItemFromDto),
    [query.data],
  )

  // Buffer incoming traces when hovered, flush on leave.
  useEffect(() => {
    if (!incoming.length) return
    if (isHoveredRef.current) {
      setPending(incoming)
      return
    }
    setAccepted(incoming)
    setNewTraceIds((prev) => {
      const next = new Set(prev)
      const accepted = new Set(accepted.map((t) => t.traceId))
      for (const t of incoming) {
        if (!accepted.has(t.traceId)) next.add(t.traceId)
      }
      return next
    })
  }, [incoming, accepted])

  function flushPendingTraces() {
    if (!pending.length) return
    setAccepted(pending)
    setPending([])
  }

  return {
    traceGroups: accepted,
    newTraceIds,
    setNewTraceIds,
    hasOtelConfigured: query.data?.has_otel_configured ?? true,
    isQueryLoading: query.isLoading,
    refetch: () => query.refetch(),
    isHoveredRef,
    flushPendingTraces,
  }
}
```

Cross-check fields against the motia source. If motia's hook exposes additional state (e.g., last-error toast, total count), port those too.

- [ ] **Step 3: Typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/pages/Traces/hooks/useTraceData.ts
git commit -m "feat(traces): port useTraceData hook (React Query)"
```

### Task 2.6: Wire raw trace list into `TracesPage`

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Update the page**

```tsx
import { useState } from 'react'
import { Cell } from '@/components/ui/Cell'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useTraceData } from './hooks/useTraceData'

export function Traces() {
  const [showSystem] = useState(false)
  const {
    traceGroups,
    hasOtelConfigured,
    isQueryLoading,
  } = useTraceData({
    filterParams: { limit: 100 },
    showSystem,
    debouncedSearch: '',
    isPaused: false,
  })

  return (
    <section className="flex-1 flex flex-col overflow-hidden">
      <div className="px-9 py-8 border-b border-rule">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-3">
          <span className="text-accent">$</span>
          <span className="text-ink ml-2">traces</span>
        </div>
        <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
          traces
        </h1>
      </div>

      {!hasOtelConfigured ? (
        <div className="p-9">
          <Cell title="no observability">
            this engine does not have the trace exporter registered. configure
            the engine with the otel/memory exporter to start capturing
            traces.
          </Cell>
        </div>
      ) : isQueryLoading && traceGroups.length === 0 ? (
        <div className="p-9">
          <StatusPanel headline="loading traces" />
        </div>
      ) : traceGroups.length === 0 ? (
        <div className="p-9">
          <Cell title="no traces recorded">
            traces appear here when functions execute. fire a request to your
            engine and refresh.
          </Cell>
        </div>
      ) : (
        <ul className="flex-1 overflow-y-auto">
          {traceGroups.map((t) => (
            <li
              key={t.traceId}
              className="px-9 py-3 border-b border-rule-2 font-mono text-[13px] text-ink"
            >
              <code className="text-ink-faint mr-3 tabular-nums">
                {t.traceId.slice(0, 8)}
              </code>
              <span>{t.functionId ?? t.rootOperation}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
```

This is intentionally unpolished — the goal is end-to-end data flow visible on the page. Phase 3 styles it.

- [ ] **Step 2: Verify in browser**

Run `pnpm dev`. Navigate to `#/traces`. Expected outcomes:
- Engine with traces: list of trace IDs + operation names appears.
- Engine without observability: "no observability" Cell renders.
- Engine empty: "no traces recorded" Cell renders.
- Loading: "loading traces" StatusPanel briefly visible.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire raw trace list end-to-end"
```

---

## Phase 3: List view polished

### Task 3.1: Create the `Skeleton` primitive

**Files:**
- Create: `workers/console/web/src/components/ui/Skeleton.tsx`
- Modify: `workers/console/web/src/index.css` (add `@keyframes` for skeleton pulse)

- [ ] **Step 1: Add the keyframe + utility to `index.css`**

Append to `src/index.css` after the existing `@utility` blocks, before the `.composer-editor` styles:

```css
@keyframes skeleton-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.55; }
}

@utility skeleton-pulse {
  animation: skeleton-pulse 1.4s ease-in-out infinite;
}
```

- [ ] **Step 2: Write the primitive**

Create `src/components/ui/Skeleton.tsx`:

```tsx
import type * as React from 'react'
import { cn } from '@/lib/utils'

interface SkeletonProps extends React.HTMLAttributes<HTMLSpanElement> {
  className?: string
}

export function Skeleton({ className, ...props }: SkeletonProps) {
  return (
    <span
      aria-hidden
      className={cn(
        'inline-block bg-panel skeleton-pulse align-middle',
        className,
      )}
      {...props}
    />
  )
}
```

Rectangular only — no `rounded-*`. Pulse via the new `@utility`.

- [ ] **Step 3: Typecheck + commit**

```bash
pnpm typecheck
git add src/components/ui/Skeleton.tsx src/index.css
git commit -m "feat(ui): add Skeleton primitive + skeleton-pulse @utility"
```

### Task 3.2: Create the `Pagination` primitive

**Files:**
- Create: `workers/console/web/src/components/ui/Pagination.tsx`

- [ ] **Step 1: Write the file**

```tsx
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'

interface PaginationProps {
  currentPage: number
  totalPages: number
  totalItems: number
  pageSize: number
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  pageSizeOptions?: number[]
  className?: string
}

export function Pagination({
  currentPage,
  totalPages,
  totalItems,
  pageSize,
  onPageChange,
  onPageSizeChange,
  pageSizeOptions = [25, 50, 100],
  className,
}: PaginationProps) {
  const start = totalItems === 0 ? 0 : (currentPage - 1) * pageSize + 1
  const end = Math.min(currentPage * pageSize, totalItems)
  return (
    <div
      className={cn(
        'flex items-center justify-between gap-4 font-mono text-[12px] text-ink-faint lowercase',
        className,
      )}
    >
      <div className="flex items-center gap-2">
        <span className="uppercase tracking-[0.06em] text-[11px]">
          show
        </span>
        <select
          value={pageSize}
          onChange={(e) => onPageSizeChange(Number(e.target.value))}
          className="border border-rule bg-bg text-ink font-mono text-[12px] px-1.5 py-0.5 rounded-none focus:outline-none focus:border-accent"
        >
          {pageSizeOptions.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      </div>
      <div className="flex items-center gap-2 tabular-nums">
        <span>
          {start}–{end} of {totalItems}
        </span>
        <Button
          variant="icon"
          size="icon"
          aria-label="previous page"
          disabled={currentPage <= 1}
          onClick={() => onPageChange(currentPage - 1)}
        >
          <ChevronLeft className="w-3.5 h-3.5" />
        </Button>
        <span className="uppercase tracking-[0.06em] text-[11px]">
          page {currentPage} of {totalPages}
        </span>
        <Button
          variant="icon"
          size="icon"
          aria-label="next page"
          disabled={currentPage >= totalPages}
          onClick={() => onPageChange(currentPage + 1)}
        >
          <ChevronRight className="w-3.5 h-3.5" />
        </Button>
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Typecheck + commit**

```bash
pnpm typecheck
git add src/components/ui/Pagination.tsx
git commit -m "feat(ui): add Pagination primitive"
```

### Task 3.3: Create the `EmptyState` + `Badge` + `ErrorBoundary` primitives

**Files:**
- Create: `workers/console/web/src/components/ui/EmptyState.tsx`
- Create: `workers/console/web/src/components/ui/Badge.tsx`
- Create: `workers/console/web/src/components/ui/ErrorBoundary.tsx`

- [ ] **Step 1: Write `EmptyState.tsx`**

```tsx
import type { LucideIcon } from 'lucide-react'
import type * as React from 'react'
import { Button } from '@/components/ui/Button'
import { Cell } from '@/components/ui/Cell'

interface EmptyStateProps {
  icon?: LucideIcon
  title: string
  description: string
  action?: {
    label: string
    onClick: () => void
  }
}

export function EmptyState({ icon: Icon, title, description, action }: EmptyStateProps) {
  return (
    <Cell title={title}>
      <div className="flex items-start gap-3">
        {Icon ? (
          <Icon aria-hidden className="w-4 h-4 text-ink-faint shrink-0 mt-0.5" />
        ) : null}
        <div className="flex-1">
          <p className="font-mono text-[13px] text-ink-faint lowercase">
            {description}
          </p>
          {action ? (
            <div className="mt-3">
              <Button variant="ghost" size="sm" onClick={action.onClick}>
                {action.label}
              </Button>
            </div>
          ) : null}
        </div>
      </div>
    </Cell>
  )
}
```

- [ ] **Step 2: Write `Badge.tsx`**

```tsx
import type * as React from 'react'
import { cn } from '@/lib/utils'

type BadgeVariant = 'default' | 'warn' | 'alert' | 'accent'

const variantTone: Record<BadgeVariant, string> = {
  default: 'text-ink-faint',
  warn: 'text-warn',
  alert: 'text-alert',
  accent: 'text-accent',
}

interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: BadgeVariant
}

export function Badge({
  variant = 'default',
  className,
  children,
  ...props
}: BadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 font-mono text-[11px] font-medium uppercase tracking-[0.06em]',
        variantTone[variant],
        className,
      )}
      {...props}
    >
      {children}
    </span>
  )
}
```

No background fills — variant only changes text color (per §9).

- [ ] **Step 3: Write `ErrorBoundary.tsx`**

```tsx
import { Component, type ReactNode } from 'react'
import { StatusPanel } from '@/components/ui/StatusPanel'

interface State {
  error: Error | null
}

interface Props {
  children: ReactNode
  fallback?: (error: Error) => ReactNode
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error) {
    if (import.meta.env.DEV) {
      // eslint-disable-next-line no-console
      console.error('[ErrorBoundary]', error)
    }
  }

  render() {
    if (this.state.error) {
      if (this.props.fallback) return this.props.fallback(this.state.error)
      return (
        <StatusPanel
          variant="alert"
          headline="something broke"
          detail={this.state.error.message}
        />
      )
    }
    return this.props.children
  }
}
```

- [ ] **Step 4: Typecheck + commit**

```bash
pnpm typecheck
git add src/components/ui/EmptyState.tsx src/components/ui/Badge.tsx src/components/ui/ErrorBoundary.tsx
git commit -m "feat(ui): add EmptyState, Badge, ErrorBoundary primitives"
```

### Task 3.4: Polish the trace list

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/TraceListRow.tsx`
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Extract `TraceListRow`**

```tsx
import { Timer, Zap } from 'lucide-react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { TraceListItem } from '../lib/traceListItem'
import { formatDuration } from '../lib/traceUtils'

function formatTime(timestamp: number): string {
  const ms = timestamp > 4_102_444_800_000 ? timestamp / 1_000_000 : timestamp
  const date = new Date(ms)
  if (Number.isNaN(date.getTime())) return '—'
  return date.toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function statusDotTone(status: TraceListItem['status']): 'accent' | 'alert' | 'warn' {
  switch (status) {
    case 'ok':
      return 'accent'
    case 'error':
      return 'alert'
    default:
      return 'warn'
  }
}

interface TraceListRowProps {
  trace: TraceListItem
  isSelected: boolean
  isNew: boolean
  onSelect: () => void
  onAnimationEnd: () => void
}

export function TraceListRow({
  trace,
  isSelected,
  isNew,
  onSelect,
  onAnimationEnd,
}: TraceListRowProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      onAnimationEnd={onAnimationEnd}
      className={cn(
        'w-full px-4 py-3 border-b border-rule-2 text-left transition-colors',
        isSelected
          ? 'bg-panel border-l-2 border-l-accent'
          : 'hover:bg-panel',
        isNew && 'trace-flash',
      )}
    >
      <div className="flex items-center gap-2 mb-1">
        <StatusDot
          tone={statusDotTone(trace.status)}
          pulse={trace.status === 'pending'}
        />
        <span className="font-mono text-[13px] text-ink truncate flex-1 lowercase">
          {trace.topic ? (
            <>
              <span className="text-ink-faint text-[11px] mr-1 uppercase tracking-[0.06em]">
                enqueue:
              </span>
              {trace.topic}
            </>
          ) : (
            (trace.functionId ?? trace.rootOperation)
          )}
        </span>
      </div>
      <div className="flex items-center gap-3 font-mono text-[11px] text-ink-faint">
        <code className="tabular-nums">{trace.traceId.slice(0, 8)}</code>
        <span className="flex items-center gap-1 tabular-nums">
          <Timer className="w-2.5 h-2.5" />
          {formatDuration(trace.duration ?? 0)}
        </span>
        <span className="flex items-center gap-1">
          <Zap className="w-2.5 h-2.5" />
          {trace.services.join(', ')}
        </span>
        <span className="ml-auto tabular-nums">{formatTime(trace.startTime)}</span>
      </div>
    </button>
  )
}
```

- [ ] **Step 2: Add the `trace-flash` @utility**

Append to `src/index.css` next to other `@utility` blocks:

```css
@keyframes trace-flash {
  0% { background-color: color-mix(in oklab, var(--color-accent) 12%, var(--color-bg)); }
  100% { background-color: var(--color-bg); }
}

@utility trace-flash {
  animation: trace-flash 600ms ease-out forwards;
}
```

- [ ] **Step 3: Update `index.tsx` to use the new row**

```tsx
import { useEffect, useState } from 'react'
import { GitBranch } from 'lucide-react'
import { Cell } from '@/components/ui/Cell'
import { EmptyState } from '@/components/ui/EmptyState'
import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import { Pagination } from '@/components/ui/Pagination'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { TraceListRow } from './components/TraceListRow'
import { useTraceData } from './hooks/useTraceData'

const PAGE_SIZES = [25, 50, 100]

export function Traces() {
  const [showSystem] = useState(false)
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(25)
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null)

  const {
    traceGroups,
    newTraceIds,
    setNewTraceIds,
    hasOtelConfigured,
    isQueryLoading,
    isHoveredRef,
    flushPendingTraces,
  } = useTraceData({
    filterParams: { limit: 500 },
    showSystem,
    debouncedSearch: '',
    isPaused: false,
  })

  const totalPages = Math.max(1, Math.ceil(traceGroups.length / pageSize))
  const start = (page - 1) * pageSize
  const paged = traceGroups.slice(start, start + pageSize)

  useEffect(() => {
    if (page > totalPages) setPage(totalPages)
  }, [page, totalPages])

  return (
    <section className="flex-1 flex flex-col overflow-hidden">
      <div className="px-9 py-8 border-b border-rule">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-3">
          <span className="text-accent">$</span>
          <span className="text-ink ml-2">traces</span>
        </div>
        <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
          traces
        </h1>
      </div>

      <ErrorBoundary>
        {!hasOtelConfigured ? (
          <div className="p-9">
            <Cell title="no observability">
              this engine does not have the trace exporter registered. configure
              the engine with the otel/memory exporter to start capturing
              traces.
            </Cell>
          </div>
        ) : isQueryLoading && traceGroups.length === 0 ? (
          <div className="flex flex-col">
            {Array.from({ length: 5 }).map((_, i) => (
              <div key={`sk-${i}`} className="px-4 py-3 border-b border-rule-2">
                <div className="flex items-center gap-2 mb-2">
                  <Skeleton className="w-1.5 h-1.5 rounded-full" />
                  <Skeleton className="h-3.5 w-48" />
                </div>
                <div className="flex items-center gap-3">
                  <Skeleton className="h-3 w-16" />
                  <Skeleton className="h-3 w-12" />
                  <Skeleton className="h-3 w-20" />
                </div>
              </div>
            ))}
          </div>
        ) : traceGroups.length === 0 ? (
          <div className="p-9">
            <EmptyState
              icon={GitBranch}
              title="no traces recorded"
              description="traces appear here when functions execute. fire a request to your engine and refresh."
            />
          </div>
        ) : (
          <div className="flex-1 flex flex-col overflow-hidden">
            <div
              className="flex-1 overflow-y-auto"
              onMouseEnter={() => {
                isHoveredRef.current = true
              }}
              onMouseLeave={() => {
                isHoveredRef.current = false
                flushPendingTraces()
              }}
            >
              {paged.map((trace) => (
                <TraceListRow
                  key={trace.traceId}
                  trace={trace}
                  isSelected={selectedTraceId === trace.traceId}
                  isNew={newTraceIds.has(trace.traceId)}
                  onSelect={() =>
                    setSelectedTraceId(
                      selectedTraceId === trace.traceId ? null : trace.traceId,
                    )
                  }
                  onAnimationEnd={() => {
                    if (newTraceIds.has(trace.traceId))
                      setNewTraceIds((prev) => {
                        const next = new Set(prev)
                        next.delete(trace.traceId)
                        return next
                      })
                  }}
                />
              ))}
            </div>
            <div className="flex-shrink-0 border-t border-rule px-4 py-2.5">
              <Pagination
                currentPage={page}
                totalPages={totalPages}
                totalItems={traceGroups.length}
                pageSize={pageSize}
                onPageChange={setPage}
                onPageSizeChange={(s) => {
                  setPageSize(s)
                  setPage(1)
                }}
                pageSizeOptions={PAGE_SIZES}
              />
            </div>
          </div>
        )}
      </ErrorBoundary>
    </section>
  )
}
```

- [ ] **Step 4: Manual verification**

`pnpm dev`. On `#/traces`:
- Rows render with `StatusDot`, ID prefix, duration, services, timestamp.
- Selection: clicking a row toggles `bg-panel + border-l-2 border-l-accent`.
- Pagination renders at the bottom; changing page size resets to page 1.
- New traces flash with the accent-faint animation.
- Hovering the list pauses live updates; leaving flushes pending.

Take a screenshot.

- [ ] **Step 5: Commit**

```bash
git add src/index.css src/pages/Traces/components/TraceListRow.tsx src/pages/Traces/index.tsx
git commit -m "feat(traces): polish list view with schematic primitives"
```

### Task 3.5: Wire page header actions (pause / system / refresh)

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Add the actions row**

Extend the header in `index.tsx` to render three `Button variant="ghost"` actions on the right of the title:
- `system` (toggles `showSystem`); when on, label shows `system`; when off, label is `text-ink-ghost` with `line-through opacity-60`.
- `pause` / `resume` (toggles `isPaused`).
- `refresh` (calls `useTraceData().refetch`; disabled while `isQueryLoading`).

Use lucide icons `Eye`, `EyeOff`, `Play`, `Pause`, `RefreshCw`. Use `Badge variant="warn"` with the text `paused` next to the title when `isPaused`.

Concrete header block to replace the existing one in `Traces()`:

```tsx
<div className="px-9 py-6 border-b border-rule flex items-end justify-between flex-wrap gap-4">
  <div>
    <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-3">
      <span className="text-accent">$</span>
      <span className="text-ink ml-2">traces</span>
    </div>
    <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
      traces
      {isPaused ? (
        <Badge variant="warn" className="ml-3 align-middle">
          <Pause className="w-3 h-3" />
          paused
        </Badge>
      ) : null}
    </h1>
  </div>
  <div className="flex items-center gap-2">
    <Button
      variant={showSystem ? 'pill' : 'ghost'}
      size="sm"
      onClick={() => setShowSystem((v) => !v)}
    >
      {showSystem ? <Eye className="w-3.5 h-3.5" /> : <EyeOff className="w-3.5 h-3.5" />}
      <span className={cn(showSystem ? '' : 'line-through opacity-60')}>system</span>
    </Button>
    <Button
      variant={isPaused ? 'pill' : 'ghost'}
      size="sm"
      onClick={() => setIsPaused((v) => !v)}
    >
      {isPaused ? <Play className="w-3.5 h-3.5" /> : <Pause className="w-3.5 h-3.5" />}
      <span>{isPaused ? 'resume' : 'pause'}</span>
    </Button>
    <Button
      variant="ghost"
      size="sm"
      onClick={() => refetch()}
      disabled={isQueryLoading}
    >
      <RefreshCw className={cn('w-3.5 h-3.5', isQueryLoading && 'animate-spin')} />
      <span>refresh</span>
    </Button>
  </div>
</div>
```

Add the relevant imports (`Eye`, `EyeOff`, `Play`, `Pause`, `RefreshCw` from `lucide-react`; `Badge`, `Button`; `cn`). Add `isPaused`/`setIsPaused` state + destructure `refetch` from `useTraceData`.

- [ ] **Step 2: Manual verification**

Open `#/traces`:
- Toggling `system`: button transitions from ghost to pill style; the `system` label gains/loses the strikethrough.
- Toggling `pause`/`resume`: live updates stop/start; `paused` badge appears next to the title when paused.
- `refresh`: triggers an immediate query; icon spins until done.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire pause/system/refresh actions"
```

---

## Phase 4: Waterfall view + resizable panels

### Task 4.1: Port `useResizablePanels`

**Files:**
- Create: `workers/console/web/src/pages/Traces/hooks/useResizablePanels.ts`

- [ ] **Step 1: Copy verbatim from motia**

```bash
cp /Users/andersonleal/projetos/motia/motia/console/packages/console-frontend/src/hooks/useResizablePanels.ts \
   /Users/andersonleal/projetos/motia/workers/console/web/src/pages/Traces/hooks/useResizablePanels.ts
```

The hook is React-only with no imports against the design system or token names — it ports clean.

- [ ] **Step 2: Typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/hooks/useResizablePanels.ts
git commit -m "feat(traces): port useResizablePanels"
```

### Task 4.2: Port `TraceHeader`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/TraceHeader.tsx`

- [ ] **Step 1: Read the motia source**

Read `motia/console/packages/console-frontend/src/components/traces/TraceHeader.tsx`. This is the header strip that sits above the waterfall (trace ID + duration + start time + close button).

- [ ] **Step 2: Port + reskin**

Copy the file. Then apply:
- All token swaps from the table at the top of this plan.
- Replace the `Button` with the canonical `@/components/ui/Button`.
- Replace `Badge` references with `@/components/ui/Badge`.
- Replace any `Card`/`Box` from motia with bordered `div`s (`border border-rule bg-bg`).
- Replace any `cn` import path with `@/lib/utils`.
- Lowercase every literal user-facing string.
- Verify no `rounded-*` or `shadow-*` survive.

- [ ] **Step 3: Typecheck + commit**

```bash
pnpm typecheck
git add src/pages/Traces/components/TraceHeader.tsx
git commit -m "feat(traces): port TraceHeader with schematic skin"
```

### Task 4.3: Port `WaterfallChart`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/WaterfallChart.tsx`

- [ ] **Step 1: Read the motia source**

Read `motia/console/packages/console-frontend/src/components/traces/WaterfallChart.tsx` (19.8 KB). Note: zoom/pan logic, time axis, span rows, hover ruler, virtualization (if any).

- [ ] **Step 2: Port + reskin**

Copy the file. Apply all token swaps. Specific rules:
- Track: `bg-rule-2`.
- Bars: `bg-ink`. Error bars: `bg-alert`. Pending bars: `bg-warn`.
- Selected bar: add `outline outline-2 outline-accent` (instead of any prior `ring-*`).
- Time axis ticks: `text-ink-ghost`, label-caps-sm (`text-[11px] uppercase tracking-[0.06em]`).
- Hover ruler: 1px vertical line, `bg-rule`.
- Sticky axis row: `bg-bg border-b border-rule`.
- No rounded corners anywhere. No gradients. No `shadow-*`.
- Tooltips on hover: use Radix `Tooltip` (created later in Task 5.2) — for now, keep motia's tooltip stub and revisit in Task 5.2.

If the motia file uses `getServiceColor` from `traceColors`, swap to `serviceTone(name).fill`.

- [ ] **Step 3: Typecheck + commit**

```bash
pnpm typecheck
git add src/pages/Traces/components/WaterfallChart.tsx
git commit -m "feat(traces): port WaterfallChart with schematic palette"
```

### Task 4.4: Wire trace-detail panel into `TracesPage`

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Add selection + detail state**

Add to the top of `Traces()`:

```ts
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchTraceTree } from './api/traces'
import { TraceHeader } from './components/TraceHeader'
import { WaterfallChart } from './components/WaterfallChart'
import { useResizablePanels } from './hooks/useResizablePanels'
import {
  treeToWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from './lib/traceTransform'
```

State additions:

```ts
const [selectedSpan, setSelectedSpan] = useState<VisualizationSpan | null>(null)
const [waterfallData, setWaterfallData] = useState<WaterfallData | null>(null)
const [isLoadingSpans, setIsLoadingSpans] = useState(false)
const [spansError, setSpansError] = useState<string | null>(null)

const containerRef = useRef<HTMLDivElement>(null)
const { panelWidths, isResizing, startResize, resetTracePanel } =
  useResizablePanels({
    selectedSpanId: selectedSpan?.span_id ?? null,
    containerRef,
  })

const loadTraceSpans = useCallback(async (traceId: string) => {
  setIsLoadingSpans(true)
  setSpansError(null)
  setWaterfallData(null)
  try {
    const data = await fetchTraceTree(traceId)
    if (data.roots?.length) {
      const wf = treeToWaterfallData(data.roots)
      if (wf) setWaterfallData(wf)
      else setSpansError('failed to process span data')
    } else {
      setSpansError('no span data available for this trace')
    }
  } catch (err) {
    setSpansError(err instanceof Error ? err.message : 'failed to load trace')
  } finally {
    setIsLoadingSpans(false)
  }
}, [])

const selectTrace = useCallback((traceId: string | null) => {
  setSelectedTraceId(traceId)
  setSelectedSpan(null)
  setWaterfallData(null)
  setSpansError(null)
  if (traceId) {
    setIsPaused(true)
    loadTraceSpans(traceId)
  }
}, [loadTraceSpans])

const selectedTrace = useMemo(
  () => traceGroups.find((t) => t.traceId === selectedTraceId) ?? null,
  [traceGroups, selectedTraceId],
)
```

Replace the `TraceListRow.onSelect` handler with `() => selectTrace(...)`.

- [ ] **Step 2: Render the detail panel**

Wrap the existing list + a new detail panel in a horizontal flex container with a draggable divider:

```tsx
<div className="flex-1 flex overflow-hidden" ref={containerRef}>
  <div className="flex flex-col flex-1 overflow-hidden">
    {/* existing list + pagination here */}
  </div>

  {selectedTrace ? (
    <>
      <button
        type="button"
        aria-label="resize trace panel"
        onMouseDown={(e) => startResize(e, 'trace')}
        onDoubleClick={resetTracePanel}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            resetTracePanel()
          }
        }}
        className="w-[3px] flex-shrink-0 cursor-col-resize bg-rule hover:bg-accent active:bg-accent"
      />
      <div
        style={{ width: panelWidths.trace }}
        className={cn(
          'bg-panel flex flex-col h-full overflow-hidden flex-shrink-0',
          isResizing && 'pointer-events-none select-none',
        )}
      >
        {isLoadingSpans && (
          <div className="p-4 space-y-2">
            {Array.from({ length: 5 }).map((_, i) => (
              <Skeleton key={`sp-sk-${i}`} className="h-6 w-full" />
            ))}
          </div>
        )}
        {!isLoadingSpans && spansError && (
          <div className="p-4">
            <StatusPanel
              variant="alert"
              headline="failed to load trace"
              detail={spansError}
            />
            <Button
              variant="ghost"
              size="sm"
              className="mt-3"
              onClick={() => loadTraceSpans(selectedTrace.traceId)}
            >
              <RefreshCw className="w-3 h-3" />
              retry
            </Button>
          </div>
        )}
        {!isLoadingSpans && !spansError && waterfallData && (
          <>
            <TraceHeader
              data={waterfallData}
              traceId={selectedTrace.traceId}
              onClose={() => selectTrace(null)}
              onSpanClick={setSelectedSpan}
            />
            <div className="flex-1 overflow-auto min-h-0">
              <WaterfallChart
                data={waterfallData}
                onSpanClick={setSelectedSpan}
                selectedSpanId={selectedSpan?.span_id}
              />
            </div>
          </>
        )}
      </div>
    </>
  ) : null}
</div>
```

- [ ] **Step 3: Manual verification**

`pnpm dev`. Click a trace row → the right panel opens, waterfall loads. Drag the 3px divider → both panels resize. Double-click the divider → trace panel resets to default width. Close button on the trace header → panel closes.

- [ ] **Step 4: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire waterfall detail panel with resize"
```

---

## Phase 5: Span panel + 7 tabs

### Task 5.1: Create the `Tabs` primitive

**Files:**
- Create: `workers/console/web/src/components/ui/Tabs.tsx`

- [ ] **Step 1: Write the file**

```tsx
import * as TabsPrimitive from '@radix-ui/react-tabs'
import * as React from 'react'
import { cn } from '@/lib/utils'

export const Tabs = TabsPrimitive.Root

export const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.List
    ref={ref}
    className={cn(
      'flex items-center gap-4 border-b border-rule',
      className,
    )}
    {...props}
  />
))
TabsList.displayName = 'TabsList'

export const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={cn(
      'font-mono text-[11px] uppercase tracking-[0.06em] py-2 transition-colors',
      'text-ink-faint hover:text-ink',
      'data-[state=active]:text-ink data-[state=active]:border-b data-[state=active]:border-ink',
      // Compensate the 1px underline so the row height stays constant.
      'data-[state=active]:-mb-px',
      className,
    )}
    {...props}
  />
))
TabsTrigger.displayName = 'TabsTrigger'

export const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn('focus-visible:outline-none', className)}
    {...props}
  />
))
TabsContent.displayName = 'TabsContent'
```

- [ ] **Step 2: Typecheck + commit**

```bash
pnpm typecheck
git add src/components/ui/Tabs.tsx
git commit -m "feat(ui): add Tabs primitive (Radix + schematic)"
```

### Task 5.2: Create the `Tooltip` primitive

**Files:**
- Create: `workers/console/web/src/components/ui/Tooltip.tsx`

- [ ] **Step 1: Write the file**

```tsx
import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import * as React from 'react'
import { cn } from '@/lib/utils'

export const TooltipProvider = TooltipPrimitive.Provider
export const Tooltip = TooltipPrimitive.Root
export const TooltipTrigger = TooltipPrimitive.Trigger

export const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 6, ...props }, ref) => (
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={ref}
      sideOffset={sideOffset}
      className={cn(
        'z-50 border border-rule bg-bg px-2.5 py-1.5 font-mono text-[12px] text-ink lowercase shadow-none',
        // No drop shadow per §5; structure comes from the 1px rule.
        className,
      )}
      {...props}
    />
  </TooltipPrimitive.Portal>
))
TooltipContent.displayName = 'TooltipContent'
```

- [ ] **Step 2: Wrap the app with `TooltipProvider`**

Modify `src/main.tsx` — replace the body with:

```tsx
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import './index.css'

const root = document.getElementById('root')
if (!root) throw new Error('missing #root container')

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: 1,
      staleTime: 1_000,
    },
  },
})

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={150}>
        <App />
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>,
)
```

- [ ] **Step 3: Typecheck + commit**

```bash
pnpm typecheck
git add src/components/ui/Tooltip.tsx src/main.tsx
git commit -m "feat(ui): add Tooltip primitive and provider"
```

### Task 5.3: Port `SpanPanel` shell

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanPanel.tsx`

- [ ] **Step 1: Read the motia source**

Read `motia/console/packages/console-frontend/src/components/traces/SpanPanel.tsx`. Understand the tab list, the close button, the prop shape (`span`, `traceData`, `onClose`, `onNavigateToSpan`, `onNavigateToTrace`).

- [ ] **Step 2: Port + reskin**

Apply token swaps. Replace motia's tabs implementation with `@/components/ui/Tabs`. Tab order (lowercase):

```tsx
<TabsList>
  <TabsTrigger value="info">info</TabsTrigger>
  <TabsTrigger value="errors">errors</TabsTrigger>
  <TabsTrigger value="logs">logs</TabsTrigger>
  <TabsTrigger value="otel-logs">otel logs</TabsTrigger>
  <TabsTrigger value="tags">tags</TabsTrigger>
  <TabsTrigger value="baggage">baggage</TabsTrigger>
  <TabsTrigger value="links">links</TabsTrigger>
</TabsList>
```

The "no key remount" rule from motia's `routes/traces.tsx:632-640` applies — do **not** key the `Tabs` root on `span.span_id`, so the active tab persists when navigating between spans.

For each tab content, import the corresponding tab component (created in next tasks) and render inside `<TabsContent value="...">`.

- [ ] **Step 3: Commit (will fail typecheck until tab components exist; commit the shell first with placeholder imports)**

Use lazy imports or stub the tab bodies with `<div>placeholder</div>` returning the file compilable. Typecheck. Then:

```bash
git add src/pages/Traces/components/SpanPanel.tsx
git commit -m "feat(traces): port SpanPanel shell with schematic tabs"
```

### Tasks 5.4–5.10: Port each of the 7 span tabs

Each tab follows the same pattern. The general recipe:

1. **Read** the motia source at `motia/console/packages/console-frontend/src/components/traces/<Name>.tsx`.
2. **Copy** to `workers/console/web/src/pages/Traces/components/<Name>.tsx`.
3. **Apply** the token swap table.
4. **Replace** Radix imports with their schematic wrappers (`@/components/ui/...`).
5. **Replace** lucide imports — they're already `lucide-react`, no change needed.
6. **Lowercase** every literal user-facing string.
7. **Render rows** as flat divide-y type tables (per spec §4.5 → DESIGN.md §12 "tree" pattern): each row is `name | type | value` on one line, optional description on a second line in `text-ink-faint text-[12px]`. No nested cards inside tab bodies.
8. **Typecheck** and **commit** per tab.

### Task 5.4: `SpanInfoTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanInfoTab.tsx`

- [ ] **Step 1: Port from motia + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanInfoTab.tsx
git commit -m "feat(traces): port SpanInfoTab"
```

### Task 5.5: `SpanErrorsTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanErrorsTab.tsx`

Same recipe. **Error severity styling rule (per §9):** row stripe is `border-l-2 border-l-alert` plus faint `bg-alert/5` — never a full-color background. Stack traces render inside a `border border-rule` `bg-bg` `<pre>` block (12.5px / leading-1.55 / `text-ink`).

- [ ] **Step 1: Port + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanErrorsTab.tsx
git commit -m "feat(traces): port SpanErrorsTab"
```

### Task 5.6: `SpanLogsTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanLogsTab.tsx`

Renders span events. Each event renders as a row with `tabular-nums` timestamp (label-caps-sm) + `text-ink` event name on the first line, attributes table below.

- [ ] **Step 1: Port + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanLogsTab.tsx
git commit -m "feat(traces): port SpanLogsTab"
```

### Task 5.7: `SpanOtelLogsTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanOtelLogsTab.tsx`

Most complex of the tabs (~10 KB). Renders correlated OTel logs with severity. Severity styling: `info` → ink, `warn` → `text-warn`, `error` → `text-alert`. Severity is conveyed via `text-*` only — never via background fills (per §9).

- [ ] **Step 1: Port + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanOtelLogsTab.tsx
git commit -m "feat(traces): port SpanOtelLogsTab"
```

### Task 5.8: `SpanTagsTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanTagsTab.tsx`

Renders span attributes as a flat type-table.

- [ ] **Step 1: Port + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanTagsTab.tsx
git commit -m "feat(traces): port SpanTagsTab"
```

### Task 5.9: `SpanBaggageTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanBaggageTab.tsx`

OTel baggage key/value pairs as flat type-table. Empty state: `Cell` titled "no baggage" with a one-liner explanation.

- [ ] **Step 1: Port + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanBaggageTab.tsx
git commit -m "feat(traces): port SpanBaggageTab"
```

### Task 5.10: `SpanLinksTab`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/SpanLinksTab.tsx`

Span-to-span links. Each link row is clickable; clicking calls `onNavigateToTrace(trace_id)` or `onNavigateToSpan(span_id)` if the link is intra-trace.

- [ ] **Step 1: Port + reskin**
- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/SpanLinksTab.tsx
git commit -m "feat(traces): port SpanLinksTab"
```

### Task 5.11: Wire `SpanPanel` into `TracesPage`

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Add the span panel section**

In `Traces()`, after the trace-detail panel:

```tsx
{selectedSpan && waterfallData ? (
  <>
    <button
      type="button"
      aria-label="resize span panel"
      onMouseDown={(e) => startResize(e, 'span')}
      onDoubleClick={resetSpanPanel}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          resetSpanPanel()
        }
      }}
      className="w-[3px] flex-shrink-0 cursor-col-resize bg-rule hover:bg-accent active:bg-accent"
    />
    <div
      style={{ width: panelWidths.span }}
      className={cn(
        'bg-panel flex-shrink-0 h-full overflow-hidden',
        isResizing && 'pointer-events-none select-none',
      )}
    >
      <SpanPanel
        span={selectedSpan}
        traceData={waterfallData}
        onClose={() => setSelectedSpan(null)}
        onNavigateToSpan={setSelectedSpan}
        onNavigateToTrace={selectTrace}
      />
    </div>
  </>
) : null}
```

Destructure `resetSpanPanel` from `useResizablePanels`.

- [ ] **Step 2: Manual verification**

Click a trace → see waterfall. Click a span in the waterfall → span panel opens on the right. Tab through all 7 tabs. Drag the 3px divider between waterfall and span panel — both resize. Close span panel via the close button. Click another span without closing — active tab persists.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire SpanPanel into trace detail"
```

---

## Phase 6: Flame graph + view switcher + service breakdown

### Task 6.1: Port `ViewSwitcher`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/ViewSwitcher.tsx`

- [ ] **Step 1: Port**

The motia version is a 4-option toggle (waterfall, flamegraph, map, flow). Rewrite on the existing `ModeToggle` primitive from `@/components/ui/ModeToggle`:

```tsx
import { ModeToggle } from '@/components/ui/ModeToggle'

export type ViewType = 'waterfall' | 'flamegraph' | 'map' | 'flow'

interface ViewSwitcherProps {
  currentView: ViewType
  onViewChange: (next: ViewType) => void
}

export function ViewSwitcher({ currentView, onViewChange }: ViewSwitcherProps) {
  return (
    <ModeToggle<ViewType>
      value={currentView}
      onChange={onViewChange}
      options={[
        { value: 'waterfall', label: 'waterfall' },
        { value: 'flamegraph', label: 'flame' },
        { value: 'map', label: 'map' },
        { value: 'flow', label: 'flow' },
      ]}
    />
  )
}
```

- [ ] **Step 2: Typecheck + commit**

```bash
pnpm typecheck
git add src/pages/Traces/components/ViewSwitcher.tsx
git commit -m "feat(traces): port ViewSwitcher as ModeToggle"
```

### Task 6.2: Port `FlameGraph`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/FlameGraph.tsx`

- [ ] **Step 1: Read the motia source**

Read `motia/console/packages/console-frontend/src/components/traces/FlameGraph.tsx` (23.6 KB). Understand rect layout, hover/click handling, zoom.

- [ ] **Step 2: Port + reskin**

Apply token swaps. Specific:
- Rect fill by depth: tile `bg-ink/100`, `bg-ink/85`, `bg-ink/70`, `bg-ink/55` (cycle). Never chromatic.
- Error rects: `bg-alert/85`.
- Selected rect: `outline outline-2 outline-accent`.
- Text inside rects: `text-bg` for ink rects, `text-bg` for alert rects, `text-[10px]` / `text-[12.5px]` per the schematic's micro/code-sm scales.
- No rounded corners. No drop shadows on hover.
- Tooltips: use `@/components/ui/Tooltip` (`<Tooltip>` + `<TooltipTrigger asChild>` + `<TooltipContent>`).

- [ ] **Step 3: Typecheck + commit**

```bash
pnpm typecheck
git add src/pages/Traces/components/FlameGraph.tsx
git commit -m "feat(traces): port FlameGraph with monochrome ink palette"
```

### Task 6.3: Port `ServiceBreakdown`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/ServiceBreakdown.tsx`

- [ ] **Step 1: Port + reskin**

Apply token swaps. Specific:
- Service stat row: bordered `border border-rule` container with `divide-x divide-rule-2` between cells.
- Service name uses `serviceTone(name).text` — never chromatic.
- Durations: `tabular-nums`, `text-ink`.

- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/ServiceBreakdown.tsx
git commit -m "feat(traces): port ServiceBreakdown"
```

### Task 6.4: Wire view switcher + flame view + service breakdown

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Add view state + render switcher**

Add to `Traces()`:

```ts
const [activeView, setActiveView] = useState<ViewType>('waterfall')
```

Inside the trace-detail panel, between `TraceHeader` and the chart content:

```tsx
<div className="border-b border-rule px-4 py-2.5">
  <ViewSwitcher currentView={activeView} onViewChange={setActiveView} />
</div>

<div className="flex-1 overflow-auto min-h-0">
  {activeView === 'waterfall' && (
    <WaterfallChart
      data={waterfallData}
      onSpanClick={setSelectedSpan}
      selectedSpanId={selectedSpan?.span_id}
    />
  )}
  {activeView === 'flamegraph' && (
    <FlameGraph
      data={waterfallData}
      onSpanClick={setSelectedSpan}
      selectedSpanId={selectedSpan?.span_id}
    />
  )}
</div>

{activeView !== 'flow' && (
  <div className="border-t border-rule flex-shrink-0">
    <ServiceBreakdown data={waterfallData} />
  </div>
)}
```

- [ ] **Step 2: Manual verification**

Click a trace, switch between `waterfall` and `flame` views. Service breakdown stays at the bottom for non-flow views. Hover bars/rects → tooltips render.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire ViewSwitcher + flame + service breakdown"
```

---

## Phase 7: Graph views (xyflow + dagre)

### Task 7.1: Set up xyflow CSS + verify isolation

**Files:**
- Modify: `workers/console/web/src/pages/Traces/components/TraceMap.tsx` (to be created in next task — but the CSS import needs to land somewhere; do it in the page-level wrapper)
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Import xyflow CSS at the page level**

Add to the top of `src/pages/Traces/index.tsx`:

```tsx
import '@xyflow/react/dist/style.css'
```

- [ ] **Step 2: Override xyflow CSS variables to schematic tokens**

Append to `src/index.css`:

```css
/* xyflow overrides — keep node/edge chrome on the schematic palette */
.react-flow {
  background-color: var(--color-bg);
  --xy-background-color: var(--color-bg);
  --xy-node-color: var(--color-ink);
  --xy-node-border: var(--color-rule);
  --xy-node-background-color: var(--color-bg);
  --xy-edge-stroke: var(--color-ink);
  --xy-edge-stroke-width: 1px;
  --xy-handle-background-color: var(--color-ink);
  --xy-handle-border-color: var(--color-bg);
  --xy-controls-button-background-color: var(--color-bg);
  --xy-controls-button-color: var(--color-ink);
  --xy-controls-button-border-color: var(--color-rule);
}

.react-flow__controls,
.react-flow__controls-button {
  border-radius: 0 !important;
  box-shadow: none !important;
}

.react-flow__attribution {
  display: none;
}
```

- [ ] **Step 3: Verify no global regressions**

`pnpm dev` and load `#/` (chat). Verify nothing on the chat page looks altered (no leaked xyflow styles).

- [ ] **Step 4: Commit**

```bash
git add src/pages/Traces/index.tsx src/index.css
git commit -m "feat(traces): wire xyflow styles + schematic overrides"
```

### Task 7.2: Port `TraceMap`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/TraceMap.tsx`

- [ ] **Step 1: Read the motia source**

Read `motia/console/packages/console-frontend/src/components/traces/TraceMap.tsx` (12.7 KB). Understand the xyflow node + edge generation logic.

- [ ] **Step 2: Port + reskin**

Apply token swaps. Specific:
- Custom node renderer: a bordered card per span with `bg-bg`, `border border-rule`, a `bg-panel` head strip (label-caps-sm operation name + `StatusDot`), and a body with `tabular-nums` duration + service name (in `serviceTone(name).text`).
- Edges: 1px `stroke-ink`. Error path: `stroke-alert`.
- Background: solid `bg-bg`, no dot grid (`<Background />` set to a transparent variant or replaced with `<Background variant={BackgroundVariant.Cross} gap={9999} />` — i.e., effectively none).
- Controls: keep xyflow `<Controls />`; the CSS overrides from Task 7.1 give them the right chrome.

- [ ] **Step 3: Typecheck + commit**

```bash
git add src/pages/Traces/components/TraceMap.tsx
git commit -m "feat(traces): port TraceMap with schematic nodes"
```

### Task 7.3: Port `FlowView`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/FlowView.tsx`

- [ ] **Step 1: Read the motia source**

Read `motia/console/packages/console-frontend/src/components/traces/FlowView.tsx` (15.4 KB). It uses dagre for vertical layout.

- [ ] **Step 2: Port + reskin**

Same reskin rules as TraceMap. Additional:
- Function/trigger lanes: render as `border-r border-rule` columns spanning the canvas. Lane label at the top: label-caps-sm in `text-ink-faint`.
- Selected node: `border-l-2 border-l-accent` rail.

- [ ] **Step 3: Typecheck + commit**

```bash
git add src/pages/Traces/components/FlowView.tsx
git commit -m "feat(traces): port FlowView with dagre layout"
```

### Task 7.4: Wire graph views into the view switcher

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Render TraceMap + FlowView in the active-view branch**

Extend the existing render block:

```tsx
{activeView === 'map' && (
  <TraceMap data={waterfallData} onSpanClick={setSelectedSpan} />
)}
{activeView === 'flow' && (
  <FlowView
    data={waterfallData}
    onSpanClick={setSelectedSpan}
    selectedSpanId={selectedSpan?.span_id}
  />
)}
```

- [ ] **Step 2: Manual verification**

Open a trace. Switch to `map` → xyflow renders span graph with custom nodes. Switch to `flow` → vertical dagre layout. Zoom/pan controls work; nothing leaks chromatic color.

- [ ] **Step 3: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire map + flow views"
```

---

## Phase 8: Group-by, filters, live updates, polish

### Task 8.1: Port `useTraceFilters` + `useTraceGroups`

**Files:**
- Create: `workers/console/web/src/pages/Traces/hooks/useTraceFilters.ts`
- Create: `workers/console/web/src/pages/Traces/hooks/useTraceGroups.ts`

- [ ] **Step 1: Copy + adapt**

```bash
cp /Users/andersonleal/projetos/motia/motia/console/packages/console-frontend/src/hooks/useTraceFilters.ts \
   /Users/andersonleal/projetos/motia/workers/console/web/src/pages/Traces/hooks/useTraceFilters.ts

cp /Users/andersonleal/projetos/motia/motia/console/packages/console-frontend/src/hooks/useTraceGroups.ts \
   /Users/andersonleal/projetos/motia/workers/console/web/src/pages/Traces/hooks/useTraceGroups.ts
```

- [ ] **Step 2: Fix imports**

In both files, swap any `@/api/observability/traces` import for `../api/traces`. Drop any `useEngineSdk()` usage and call into `fetchTraceGroups` directly.

- [ ] **Step 3: Typecheck + commit**

```bash
pnpm typecheck
git add src/pages/Traces/hooks/useTraceFilters.ts src/pages/Traces/hooks/useTraceGroups.ts
git commit -m "feat(traces): port useTraceFilters and useTraceGroups"
```

### Task 8.2: Port `TraceFilters` (rewritten on schematic)

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/TraceFilters.tsx`

- [ ] **Step 1: Read the motia source for the data shape**

Read `motia/console/packages/console-frontend/src/components/traces/TraceFilters.tsx` (29 KB). Identify the filter fields it exposes: search, groupBy, timeRange, statusFilter, page, pageSize, plus the attribute filter sub-component.

- [ ] **Step 2: Write the schematic rebuild**

Do **not** import `cmdk` or `react-select`. Write a single horizontal filter row:

```tsx
import type * as React from 'react'
import { Search } from 'lucide-react'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import type {
  TraceFilterState,
  TraceFilterValidationWarning,
} from '../hooks/useTraceFilters'

interface TraceFiltersProps {
  filters: TraceFilterState
  onFilterChange: <K extends keyof TraceFilterState>(
    key: K,
    value: TraceFilterState[K],
  ) => void
  onClear: () => void
  validationWarnings: TraceFilterValidationWarning[]
  onClearWarnings: () => void
  isLoading: boolean
  searchQuery: string
  onSearchChange: (value: string) => void
  stats?: { totalTraces: number; errorCount: number; avgDuration: number }
}

export function TraceFilters({
  filters,
  onFilterChange,
  onClear,
  validationWarnings,
  onClearWarnings,
  isLoading,
  searchQuery,
  onSearchChange,
  stats,
}: TraceFiltersProps) {
  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center gap-3 flex-wrap">
        {/* search */}
        <label className="flex items-center gap-2 border-b border-ink focus-within:border-accent transition-colors px-1 flex-1 min-w-[240px]">
          <Search className="w-3.5 h-3.5 text-ink-faint shrink-0" />
          <input
            type="search"
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder="search by name, id, function…"
            className="flex-1 bg-transparent font-mono text-[13px] text-ink placeholder:text-ink-ghost py-1.5 outline-none lowercase"
          />
        </label>

        {/* group-by */}
        <ModeToggle
          value={filters.groupBy ?? 'none'}
          onChange={(next) => onFilterChange('groupBy', next === 'none' ? null : next)}
          options={[
            { value: 'none', label: 'flat' },
            { value: 'function', label: 'function' },
            { value: 'trigger', label: 'trigger' },
            { value: 'session', label: 'session' },
          ]}
        />

        {/* status */}
        <ModeToggle
          value={filters.statusFilter ?? 'all'}
          onChange={(next) =>
            onFilterChange('statusFilter', next === 'all' ? null : next)
          }
          options={[
            { value: 'all', label: 'all' },
            { value: 'ok', label: 'ok' },
            { value: 'error', label: 'error' },
            { value: 'pending', label: 'pending' },
          ]}
        />

        <Button variant="ghost" size="sm" onClick={onClear} disabled={isLoading}>
          clear filters
        </Button>
      </div>

      {validationWarnings.length > 0 && (
        <StatusPanel
          variant="warn"
          headline="filter validation"
          detail={validationWarnings.map((w) => w.message).join(' · ')}
        />
      )}

      {stats ? (
        <div className="flex items-center gap-6 border border-rule divide-x divide-rule-2 font-mono text-[11px] text-ink-faint">
          <div className="px-3 py-1.5 tabular-nums">
            <span className="uppercase tracking-[0.06em] text-[10px]">total</span>{' '}
            <span className="text-ink">{stats.totalTraces}</span>
          </div>
          <div className="px-3 py-1.5 tabular-nums">
            <span className="uppercase tracking-[0.06em] text-[10px]">errors</span>{' '}
            <span className={cn(stats.errorCount > 0 ? 'text-alert' : 'text-ink')}>
              {stats.errorCount}
            </span>
          </div>
          <div className="px-3 py-1.5 tabular-nums">
            <span className="uppercase tracking-[0.06em] text-[10px]">avg</span>{' '}
            <span className="text-ink">{Math.round(stats.avgDuration)}ms</span>
          </div>
        </div>
      ) : null}
    </div>
  )
}
```

This intentionally drops motia's most exotic filter affordances (the cmdk palette + the advanced attribute filter); Task 8.3 ports the advanced filter as a collapsible details block.

- [ ] **Step 3: Typecheck + commit**

```bash
git add src/pages/Traces/components/TraceFilters.tsx
git commit -m "feat(traces): rebuild TraceFilters on schematic primitives"
```

### Task 8.3: Port `AttributesFilter` (collapsible details)

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/AttributesFilter.tsx`

- [ ] **Step 1: Build a `<details>` based attribute filter**

Use the `.iii-details` CSS class (already in `index.css`). Each filter row: attribute key input + operator select (`=`, `!=`, `contains`) + value input + remove button. All bordered.

Read the motia source to copy the filter-rule data shape and validation, then write the renderer with schematic primitives only:

```tsx
import { ChevronRight, Plus, X } from 'lucide-react'
import type * as React from 'react'
import { Button } from '@/components/ui/Button'

interface FilterRule {
  id: string
  key: string
  operator: '=' | '!=' | 'contains'
  value: string
}

interface AttributesFilterProps {
  rules: FilterRule[]
  onAdd: (rule: FilterRule) => void
  onRemove: (id: string) => void
  onUpdate: (id: string, patch: Partial<FilterRule>) => void
}

export function AttributesFilter({ rules, onAdd, onRemove, onUpdate }: AttributesFilterProps) {
  return (
    <details className="iii-details border border-rule">
      <summary className="px-3 py-2 flex items-center gap-2 bg-panel font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        <ChevronRight className="iii-chev w-3 h-3" />
        attribute filters
        {rules.length > 0 ? (
          <span className="ml-auto text-ink tabular-nums">{rules.length}</span>
        ) : null}
      </summary>
      <div className="p-3 flex flex-col gap-2">
        {rules.length === 0 ? (
          <p className="font-mono text-[12px] text-ink-faint lowercase">
            add a rule to filter by span/resource attributes.
          </p>
        ) : (
          rules.map((rule) => (
            <div key={rule.id} className="flex items-center gap-1.5">
              <input
                value={rule.key}
                onChange={(e) => onUpdate(rule.id, { key: e.target.value })}
                placeholder="key (e.g. http.method)"
                className="flex-1 border border-rule bg-bg px-2 py-1 font-mono text-[12px] text-ink focus:outline-none focus:border-accent rounded-none lowercase"
              />
              <select
                value={rule.operator}
                onChange={(e) =>
                  onUpdate(rule.id, {
                    operator: e.target.value as FilterRule['operator'],
                  })
                }
                className="border border-rule bg-bg px-2 py-1 font-mono text-[12px] text-ink focus:outline-none focus:border-accent rounded-none"
              >
                <option value="=">=</option>
                <option value="!=">!=</option>
                <option value="contains">contains</option>
              </select>
              <input
                value={rule.value}
                onChange={(e) => onUpdate(rule.id, { value: e.target.value })}
                placeholder="value"
                className="flex-1 border border-rule bg-bg px-2 py-1 font-mono text-[12px] text-ink focus:outline-none focus:border-accent rounded-none"
              />
              <Button
                variant="icon"
                size="icon"
                aria-label="remove rule"
                onClick={() => onRemove(rule.id)}
              >
                <X className="w-3 h-3" />
              </Button>
            </div>
          ))
        )}
        <Button
          variant="ghost"
          size="sm"
          onClick={() =>
            onAdd({ id: crypto.randomUUID(), key: '', operator: '=', value: '' })
          }
        >
          <Plus className="w-3 h-3" />
          add rule
        </Button>
      </div>
    </details>
  )
}
```

Wire it into `TraceFilters.tsx` below the main row. The actual filter-state plumbing (`rules` array on `TraceFilterState`) should already exist in `useTraceFilters` — verify against the motia source.

- [ ] **Step 2: Typecheck + commit**

```bash
git add src/pages/Traces/components/AttributesFilter.tsx src/pages/Traces/components/TraceFilters.tsx
git commit -m "feat(traces): port AttributesFilter as collapsible details"
```

### Task 8.4: Port `TraceGroupsView` + `SessionDetailPanel`

**Files:**
- Create: `workers/console/web/src/pages/Traces/components/TraceGroupsView.tsx`
- Create: `workers/console/web/src/pages/Traces/components/SessionDetailPanel.tsx`
- Create: `workers/console/web/src/pages/Traces/components/WorkflowChain.tsx`

- [ ] **Step 1: Port `WorkflowChain`**

Read motia source. Render chain segments as small bordered boxes connected by 1px horizontal rules. Each segment: label-caps-sm name + `tabular-nums` duration. Lowercase everything.

- [ ] **Step 2: Port `TraceGroupsView`**

Renders groups (sessions, functions, triggers) using `useTraceGroups`. Each group row is a button (like `TraceListRow`) with a `divide-y rule-2` stack of member rows underneath. Selected group: `bg-panel border-l-2 border-l-accent`.

- [ ] **Step 3: Port `SessionDetailPanel`**

Renders all traces in a session as a stacked vertical list of waterfalls. Each waterfall block is wrapped in a `Cell`-shaped bordered container with the trace ID and operation name as a header.

- [ ] **Step 4: Typecheck + commit**

```bash
pnpm typecheck
git add src/pages/Traces/components/WorkflowChain.tsx \
        src/pages/Traces/components/TraceGroupsView.tsx \
        src/pages/Traces/components/SessionDetailPanel.tsx
git commit -m "feat(traces): port group-by views and session detail"
```

### Task 8.5: Wire filters + group-by + session detail into the page

**Files:**
- Modify: `workers/console/web/src/pages/Traces/index.tsx`

- [ ] **Step 1: Replace ad-hoc filter state with `useTraceFilters`**

In `Traces()`:

```ts
import { useTraceFilters } from './hooks/useTraceFilters'
import { TraceFilters } from './components/TraceFilters'
import { AttributesFilter } from './components/AttributesFilter'
import { TraceGroupsView } from './components/TraceGroupsView'
import { SessionDetailPanel } from './components/SessionDetailPanel'

const {
  filters: filterState,
  updateFilter,
  resetFilters,
  getActiveFilterCount,
  getFilterOnlyParams,
  validationWarnings,
  clearValidationWarnings,
} = useTraceFilters()
```

Replace the now-stale `page`/`pageSize` state with `filterState.page`/`filterState.pageSize`. Wire `filterParams: getFilterOnlyParams()` into `useTraceData`.

- [ ] **Step 2: Render `TraceFilters` + `AttributesFilter` between header and list**

```tsx
<div className="px-4 py-2.5 border-b border-rule">
  <ErrorBoundary>
    <TraceFilters
      filters={filterState}
      onFilterChange={updateFilter}
      onClear={resetFilters}
      validationWarnings={validationWarnings}
      onClearWarnings={clearValidationWarnings}
      isLoading={isQueryLoading}
      searchQuery={searchQuery}
      onSearchChange={handleSearchChange}
      stats={hasOtelConfigured ? stats : undefined}
    />
  </ErrorBoundary>
</div>
```

- [ ] **Step 3: Branch list view on `filterState.groupBy`**

If `filterState.groupBy && filterState.groupBy !== 'none'`, render `<TraceGroupsView ... />` instead of the flat list. When a session group is selected (via `selectedGroup` state), render `<SessionDetailPanel ... />` inside the trace-detail panel instead of `TraceHeader + ViewSwitcher + Chart`.

- [ ] **Step 4: Manual verification**

`pnpm dev`. On `#/traces`:
- Type in the search field; results filter after a short debounce.
- Toggle `flat → function → trigger → session`; the list view swaps to the grouped view.
- Click a session group → session detail panel renders stacked waterfalls.
- Click `clear filters` → search and toggles reset.
- Add an attribute filter rule → query refetches with the rule applied.

- [ ] **Step 5: Commit**

```bash
git add src/pages/Traces/index.tsx
git commit -m "feat(traces): wire filters, group-by, and session detail"
```

### Task 8.6: Lowercase audit + acceptance pass

**Files:**
- Modify: any file with leftover Title-Case copy

- [ ] **Step 1: Scan for uppercase initials in user-visible strings**

```bash
cd /Users/andersonleal/projetos/motia/workers/console/web
grep -rEn ">[A-Z][a-z]" src/pages/Traces src/components/ui | grep -v "tracking-\[" | grep -v "Status" | head -60
```

For each match in a user-visible string (JSX text content, `placeholder`, `aria-label`), lowercase it. Skip identifier-like uppercase strings (component names, enum values).

- [ ] **Step 2: Verify acceptance criteria**

Open `#/traces` in the browser and walk through every criterion from spec §7:
- [ ] Hash route persists across reload.
- [ ] All UI copy is lowercase except label-caps strings.
- [ ] No rounded corners except status dots / glyph circles.
- [ ] No drop shadows (sanity-check via DevTools: search for `box-shadow` rule overrides).
- [ ] At most one accent per visible region (audit by scrolling the page and counting).
- [ ] All numbers render `tabular-nums`.
- [ ] Reflow at <880 px collapses list/detail/span cleanly (resize the window).
- [ ] Dark theme renders correctly.
- [ ] Pause-on-hover / flush-on-leave still works.
- [ ] `pnpm typecheck` PASS.
- [ ] `pnpm lint` PASS.
- [ ] `pnpm test src/pages/Traces/lib` PASS.

Fix any failures inline.

- [ ] **Step 3: Commit final polish**

```bash
git add -p
git commit -m "feat(traces): lowercase audit and acceptance polish"
```

---

## Self-review notes

This plan was checked against the spec on 2026-05-18:

- Every spec section maps to one or more tasks: routing (Task 1.2), data layer (Phase 2), token swap (top of plan + applied per task), file layout (Phase 2–8), dependency plan (Task 1.1), risks (Phase 0 + Task 7.1 + Task 8.6), phasing (Phases 1–8), acceptance criteria (Task 8.6).
- No "TBD", no "implement later", no "similar to Task N".
- Every step shows the actual content needed: file paths, command lines, code, expected outcomes.
- Type/method names are consistent across tasks: `selectTrace`, `setSelectedSpan`, `useTraceData`, `getIiiClient()`, `fetchTraceTree`, etc.
- TDD is applied where it pays off (pure lib utilities have vitest tests, ported verbatim). UI components rely on manual browser verification per the project's CLAUDE.md guidance.
