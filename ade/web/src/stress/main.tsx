/**
 * The traces stress page (`/stress.html`, dev server only): the real
 * `TracesV2` surface mounted over a generated trace of any size, with no
 * engine behind it. `pnpm test:stress` drives it with Playwright and reads
 * CPU, layout, heap and DOM figures off the Chrome DevTools protocol.
 *
 * URL params:
 *   ?spans=2000      span count (default 2000)
 *   ?seed=7          generator seed
 *   ?stream=1        start mid-flight and let the test advance the stream
 *   ?theme=dark      light by default
 *   ?payload=600     bytes per captured payload
 *
 * The page exposes `window.__stress` for the driver: `ready` resolves once
 * the client is installed, `advance(fraction)` moves the stream and fires
 * the activity tick, `calls` counts the engine reads the page made.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TooltipProvider as HintProvider } from '@/components/shadcn/tooltip'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { __setIiiClientDepsForTests } from '@/lib/iii-client'
import { TracesV2 } from '@/pages/TracesV2'
import { STRESS_TRACE_ID } from '@/pages/TracesV2/fixtures/stress-traces'
import { createStressSdk } from './stress-sdk'
import '../index.css'

declare global {
  interface Window {
    __stress?: {
      ready: Promise<void>
      spanCount: number
      advance: (fraction: number) => Promise<void>
      calls: Record<string, number>
    }
  }
}

const params = new URLSearchParams(window.location.search)
const num = (key: string, fallback: number) => {
  const v = Number(params.get(key))
  return Number.isFinite(v) && v > 0 ? v : fallback
}
const spanCount = num('spans', 2000)
const stress = createStressSdk({
  spanCount,
  seed: num('seed', 7),
  stream: params.get('stream') === '1',
  payloadBytes: num('payload', 600),
})

document.documentElement.dataset.theme =
  params.get('theme') === 'dark' ? 'dark' : 'light'

__setIiiClientDepsForTests({
  resolveWsUrl: () => 'ws://stress.invalid/ws',
  resolveNamespace: async () => undefined,
  registerWorker: () => stress.sdk,
})

window.__stress = {
  ready: Promise.resolve(),
  spanCount,
  advance: (fraction) => stress.advance(fraction),
  calls: stress.calls,
}

const root = document.getElementById('root')
if (!root) throw new Error('missing #root container')
createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={new QueryClient()}>
      <TooltipProvider>
        <HintProvider>
          <div className="flex h-screen flex-col bg-bg font-sans text-ink">
            <TracesV2 initialTraceId={STRESS_TRACE_ID} />
          </div>
        </HintProvider>
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>,
)
