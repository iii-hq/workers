import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import * as Lucide from 'lucide-react'
import * as React from 'react'
import { StrictMode } from 'react'
import * as JsxRuntime from 'react/jsx-runtime'
import * as ReactDOM from 'react-dom'
import * as ReactDOMClient from 'react-dom/client'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { routeFromHash, standaloneRouteFromHash } from '@/hooks/use-hash-route'
import { buildConsoleApi } from '@/lib/console-api'
import { installRandomUUIDPolyfill } from '@/lib/crypto-polyfill'
import { getIiiClient } from '@/lib/iii-client'
import { registerServiceWorker } from '@/lib/register-service-worker'
import { setUiAssetsStatus } from '@/lib/ui-slots'
import { App } from './App'
import './index.css'
import { Standalone } from './Standalone'

// Back-fills crypto.randomUUID on insecure origins (http://<LAN-IP>) —
// iii-browser-sdk ≤ 0.21.6 calls it bare on every invocation. The module
// also self-installs on import (which biome sorts before './App', the
// SDK-bearing graph); this explicit call is the tree-shake-proof anchor
// (main.test.ts guards both). Runs before the injectable-UI boot below —
// getIiiClient() is exactly the SDK path that needs it.
installRandomUUIDPolyfill()

/**
 * Injectable-UI boot contract: the global goes up before anything else can
 * run, so the `/vendor/*` shims (resolved through the static import map in
 * index.html) can re-export the console's own React instance to injected
 * scripts. `api` fills in once the shared engine client resolves; injected
 * modules are only ever imported by the loader, which starts after that —
 * so they never observe `api: null`.
 */
const bootGlobal: NonNullable<Window['__III_CONSOLE__']> = {
  React,
  ReactDOM,
  ReactDOMClient,
  JsxRuntime,
  // The whole icon set, so `lucide-react` can stay external in worker
  // builds too (/vendor/lucide-react.js).
  Lucide,
  api: null,
}
window.__III_CONSOLE__ = bootGlobal

// The app renders before the asynchronous engine bootstrap settles. Mark the
// injected-UI slots as loading synchronously so configuration editors do not
// mistake a not-yet-registered override for a genuinely absent one.
setUiAssetsStatus('loading')
const injectableUiRuntime = getIiiClient().then((client) => {
  const api = buildConsoleApi(client)
  bootGlobal.api = api
  Object.freeze(bootGlobal)
  return { client, api }
})
void injectableUiRuntime.catch((err) => {
  setUiAssetsStatus('unavailable')
  console.error('[iii-ui] loader not started — engine client failed', err)
})

const favicon =
  document.querySelector<HTMLLinkElement>('link[rel="icon"]') ??
  document.createElement('link')
favicon.rel = 'icon'
favicon.type = 'image/svg+xml'
favicon.href = new URL('./icons/icon.svg', document.baseURI).href
if (!favicon.isConnected) document.head.appendChild(favicon)
registerServiceWorker()

const root = document.getElementById('root')
if (!root) throw new Error('missing #root container')

// `#/worker/<scope>` and `#/traces` boot the standalone shell — one surface,
// no workspace — instead of the app. The two never share a document:
// crossing that line by hash reloads into the other. Settings are the one
// hash both shells own (`#/configuration…` opens the overlay in place).
const standalone = standaloneRouteFromHash(window.location.hash) !== null
window.addEventListener('hashchange', () => {
  const hash = window.location.hash
  const now = standaloneRouteFromHash(hash) !== null
  const crosses = standalone
    ? !now && routeFromHash(hash) !== 'configuration'
    : now
  if (crosses) window.location.reload()
})
const Root = standalone ? Standalone : App

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
        <Root injectableUiRuntime={injectableUiRuntime} />
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>,
)
