import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { installRandomUUIDPolyfill } from '@/lib/crypto-polyfill'
import { App } from './App'
import faviconUrl from './icons/favicon.svg?url'
import './index.css'

// Back-fills crypto.randomUUID on insecure origins (http://<LAN-IP>) —
// iii-browser-sdk ≤ 0.21.6 calls it bare on every invocation. The module
// also self-installs on import (which biome sorts before './App', the
// SDK-bearing graph); this explicit call is the tree-shake-proof anchor
// (main.test.ts guards both).
installRandomUUIDPolyfill()

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
