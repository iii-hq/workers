import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { ConsoleExtensionsProvider } from '@/extensions/ConsoleExtensions'
import { App } from './App'
import faviconUrl from './icons/favicon.svg?url'
import './index.css'

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
      <ConsoleExtensionsProvider>
        <TooltipProvider delayDuration={150}>
          <App />
        </TooltipProvider>
      </ConsoleExtensionsProvider>
    </QueryClientProvider>
  </StrictMode>,
)
