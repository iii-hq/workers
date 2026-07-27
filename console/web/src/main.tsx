import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import * as React from 'react'
import { StrictMode } from 'react'
import * as JsxRuntime from 'react/jsx-runtime'
import * as ReactDOM from 'react-dom'
import * as ReactDOMClient from 'react-dom/client'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { buildConsoleApi } from '@/lib/console-api'
import { installRandomUUIDPolyfill } from '@/lib/crypto-polyfill'
import { getIiiClient } from '@/lib/iii-client'
import { startUiLoader } from '@/lib/ui-loader'
import { App } from './App'
import faviconUrl from './icons/favicon.svg?url'
import './index.css'

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
  api: null,
}
window.__III_CONSOLE__ = bootGlobal

getIiiClient()
  .then((client) => {
    bootGlobal.api = buildConsoleApi(client)
    Object.freeze(bootGlobal)
    startUiLoader(client, bootGlobal.api)
  })
  .catch((err) => {
    console.error('[iii-ui] loader not started — engine client failed', err)
  })

const favicon =
  document.querySelector<HTMLLinkElement>('link[rel="icon"]') ??
  document.createElement('link')
favicon.rel = 'icon'
favicon.type = 'image/svg+xml'
favicon.href = faviconUrl
if (!favicon.isConnected) document.head.appendChild(favicon)

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
