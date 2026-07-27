/**
 * Entry for the landing-page demo build (`npm run build:demo`).
 *
 * Standalone page, not part of the console SPA: no engine client, no
 * injectable-UI loader, no router. It renders one component over a scripted
 * turn and is embedded by the marketing site in an iframe, which is what
 * keeps the console's Tailwind reset from touching the host page.
 *
 * URL params:
 *   ?theme=dark|light   follow the host page's theme (default light)
 *   ?loop=0             play once instead of looping
 */

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { LandingDemo } from './LandingDemo'
import { setScenarioSpeed } from './scenario'
import '../index.css'

const params = new URLSearchParams(window.location.search)
document.documentElement.dataset.theme =
  params.get('theme') === 'dark' ? 'dark' : 'light'

/* Reduced motion: fill the turn in at once and hold it, rather than
   animating a minute of typing at someone who asked for less of that. */
if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
  setScenarioSpeed(0.02)
}

const root = document.getElementById('root')
if (!root) throw new Error('missing #root container')

/**
 * The host page pauses the demo by posting `{ type: 'iii-demo', active }`
 * when the overlay opens and closes, so a hidden iframe is not burning a
 * timer loop. Without a host it just plays.
 */
function mount() {
  const reactRoot = createRoot(root as HTMLElement)
  let active = true
  /* Bumped to replay: a fresh key remounts the player from the top. */
  let runKey = 0

  const render = () =>
    reactRoot.render(
      <StrictMode>
        <TooltipProvider delayDuration={150}>
          <LandingDemo
            key={runKey}
            active={active}
            loop={params.get('loop') === '1'}
          />
        </TooltipProvider>
      </StrictMode>,
    )

  window.addEventListener('message', (event: MessageEvent) => {
    const data = event.data
    if (!data || typeof data !== 'object') return
    if (data.type === 'iii-demo-theme') {
      /* The host's theme button flipped while the overlay is open. */
      document.documentElement.dataset.theme =
        data.theme === 'dark' ? 'dark' : 'light'
      return
    }
    if (data.type === 'iii-demo-replay') {
      runKey += 1
      render()
      return
    }
    if (data.type !== 'iii-demo') return
    const next = !!data.active
    if (next === active) return
    active = next
    render()
  })

  /* Once the viewer clicks inside the frame (the approval buttons are real),
     keystrokes land here, not on the host page — forward the one the host
     cares about so Escape keeps closing the overlay. */
  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      window.parent?.postMessage({ type: 'iii-demo-close' }, '*')
    }
  })

  render()
}

mount()
