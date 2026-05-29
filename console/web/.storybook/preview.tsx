import type { Decorator, Preview } from '@storybook/react-vite'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useEffect } from 'react'
import { TooltipProvider } from '@/components/ui/Tooltip'
import '../src/index.css'

/* Mirror main.tsx so every story sees the same data-layer + tooltip context
   the real app provides. A single client is fine: stories are read-mostly and
   `staleTime` keeps refetch noise down. */
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: 1,
      staleTime: 1_000,
    },
  },
})

const withProviders: Decorator = (Story) => (
  <QueryClientProvider client={queryClient}>
    <TooltipProvider delayDuration={150}>
      <Story />
    </TooltipProvider>
  </QueryClientProvider>
)

/* Drive the iii Schematic theme from the toolbar. index.html sets
   `data-theme` pre-paint in the real app; here the toolbar global stands in
   for that, and we stamp the body utility classes the app applies in
   index.html so the canvas background + base ink match the running console. */
const withTheme: Decorator = (Story, context) => {
  const theme = context.globals.theme === 'dark' ? 'dark' : 'light'
  useEffect(() => {
    document.documentElement.dataset.theme = theme
    document.body.classList.add('bg-bg', 'text-ink', 'font-sans')
  }, [theme])
  return <Story />
}

const preview: Preview = {
  parameters: {
    controls: {
      matchers: { color: /(background|color)$/i, date: /Date$/i },
    },
    // The iii Schematic surface owns its own background via the `bg-bg`
    // token stamped on the body; the addon's color swatches would fight it.
    backgrounds: { disable: true },
  },
  initialGlobals: { theme: 'light' },
  globalTypes: {
    theme: {
      description: 'iii Schematic theme',
      toolbar: {
        title: 'Theme',
        icon: 'mirror',
        items: [
          { value: 'light', title: 'Light' },
          { value: 'dark', title: 'Dark' },
        ],
        dynamicTitle: true,
      },
    },
  },
  decorators: [withTheme, withProviders],
}

export default preview
