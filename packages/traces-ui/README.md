# @iii-dev/traces-ui

The traces page of the iii console as React components: the live activity
strip, the trace list (filters, grouping, saved views), the trace detail
(timeline, waterfall, worker breakdown) and the span panel. It reads traces
from a `TraceSource`, so the same page runs over an iii engine or over any
store that answers in the `engine::traces::*` shapes.

## Use it

```tsx
import { createEngineTraceSource, TracesView } from '@iii-dev/traces-ui'

// Over an iii engine, through a browser iii client.
const source = createEngineTraceSource(() => getClient())

export function TracesScreen() {
  return <TracesView source={source} />
}
```

`TracesView` needs a `QueryClientProvider` (`@tanstack/react-query` 5)
above it. Its queries are keyed by what they ask for, not by source: when a
page can switch sources (one environment, then another), mount each source
under its own `QueryClient`, and build that source's settings store over
the same client, since the page reads settings from the cache the store
writes to.

### A source of your own

```ts
import type { TraceSource } from '@iii-dev/traces-ui'

const source: TraceSource = {
  listTraces: (options) => api.traces.list(options), // TracesResponse
  traceSpans: (options) => api.traces.spans(options), // TraceSpansResponse
  groupTraces: (params) => api.traces.groupBy(params), // TracesGroupByResponse
  logs: (options) => api.logs.list(options), // OtelLogsResponse
  // Optional:
  hiddenFunctionIds: async () => new Set(), // spans that start hidden
  watch: (onTraceIds, onResync) => subscribe(onTraceIds, onResync),
}
```

Without `watch` the page shows what it read and reads again when the tab
becomes visible; with it, the page reads the traces whose ids it receives,
and everything again on `onResync` (after a reconnect).

### Optional props

| Prop | What it adds |
| --- | --- |
| `settings` | A `TracesSettingsStore`: saved views, the follow toggle and the span filters persist there (the page owns the `traces` key of the stored object). Without it they live in memory and saved views are hidden. |
| `sessions` | A `TracesSessions`: the list follows the active session (`iii.session.id`), the page opens its newest turn, and the trace header links back to it. |
| `subscribeFocus` | "Show this trace" requests from elsewhere on the host. |
| `commands` | A command palette to register the page's verbs and keys in; without it Escape closes the span panel, then the trace detail. |
| `functionCallRenderers` | Cards for function calls the host knows how to draw, and redaction of their raw request/response. |
| `header`, `className`, `initialTraceId` | A header above the strip, a class on the root, a trace to open on mount. |

`TracesProvider` and `useTraceSource` give a host's own components the
source the page reads.

## Styling

The components are styled with Tailwind 4 utilities. The host's Tailwind
build has to scan them (`@source` the package's `dist` or `src`) and
define the theme tokens they read:

- shadcn: `background`, `foreground`, `card`, `card-foreground`, `popover`,
  `popover-foreground`, `primary`, `primary-foreground`, `secondary`,
  `muted`, `muted-foreground`, `border`, `input`, `ring`, `destructive`.
- ink and surfaces: `bg`, `ink`, `ink-faint`, `ink-ghost`, `surface`,
  `surface-hover`, `panel-raised`, `card-edge`, `popover-edge`,
  `dialog-surface`, `dialog-edge`.
- status: `ok`, `ok-muted`, `warn`, `warn-muted`, `warn-strong`, `alert`,
  `alert-muted`, `alert-strong`.
- traces: `worker-1` … `worker-8` (each with `-border` and `-ink`),
  `worker-ink`, `trace-error`, `trace-error-ink`, `--strip-bar-mix`.
- plain CSS variables: `--success`, `--error`, the JSON highlighting
  `--syntax-plain`, `--syntax-key`, `--syntax-string`, `--syntax-literal`,
  `--syntax-punctuation`, `--syntax-comment`, and the motion
  `--motion-duration-control`, `--motion-duration-fast`,
  `--motion-duration-panel`, `--motion-ease-standard`.
- fonts: `font-inter` and `font-mono` utilities.

and the animation utilities `live-dot`, `pulse-dot`, `skeleton-pulse`,
`timeline-enter`, `timeline-glide`, `trace-flash`, `trace-playhead`,
`trace-running` and `trace-running-quiet`. For a working reference,
`ade/web/src/index.css` in this repository defines all of them.

## Develop

```sh
pnpm --filter @iii-dev/traces-ui typecheck
pnpm --filter @iii-dev/traces-ui test
pnpm --filter @iii-dev/traces-ui build   # dist/, what npm gets
```

Publish with `pnpm publish`: the published `main`, `types` and `exports`
come from `publishConfig`, which npm ignores. `dist/index.js` is one ESM
bundle with every dependency left external; the type declarations suit
`moduleResolution: "bundler"`.

Inside this repo the ADE console imports the package from source
(`@iii-dev/traces-ui` and `@iii-dev/traces-ui/src/...`); the published
package exposes the root entry only.
