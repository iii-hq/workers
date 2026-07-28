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
 *   ?paused=1           mount paused; the host posts `{type:'iii-demo',
 *                       active:true}` when the frame is actually on screen
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

/**
 * Keep every scroll inside this document.
 *
 * `scrollIntoView` walks the ancestor scroll chain past the iframe and into
 * the EMBEDDER's viewport, so the transcript's auto-follow (`MessageList`
 * pins the newest row, and pulls a fresh approval into view) drags the host
 * page around while the session plays inline on it. The replacement does the
 * same alignment in the nearest scrollport and stops there; it is the whole
 * behavior the demo needs, and the product code stays untouched.
 */
Element.prototype.scrollIntoView = function scrollWithinFrame(
  this: Element,
  arg?: boolean | ScrollIntoViewOptions,
) {
  const opts: ScrollIntoViewOptions =
    typeof arg === 'object' ? arg : { block: arg === false ? 'end' : 'start' }
  let port: HTMLElement | null = null
  for (let p = this.parentElement; p; p = p.parentElement) {
    const overflowY = getComputedStyle(p).overflowY
    if (
      (overflowY === 'auto' || overflowY === 'scroll') &&
      p.scrollHeight > p.clientHeight
    ) {
      port = p
      break
    }
  }
  if (!port) return
  const offset =
    this.getBoundingClientRect().top -
    port.getBoundingClientRect().top +
    port.scrollTop
  const height = this.getBoundingClientRect().height
  const top =
    opts.block === 'center'
      ? offset - (port.clientHeight - height) / 2
      : opts.block === 'end' || opts.block === 'nearest'
        ? offset - port.clientHeight + height
        : offset
  port.scrollTo({ top, behavior: opts.behavior ?? 'auto' })
}

/**
 * Inner scrollports keep the wheel they catch: when the transcript or the
 * trace list bottoms out, the scroll ends there. Surface with nothing of its
 * own to scroll still chains out to the embedding page (html and body keep
 * the default), and that chain is what un-pins the marketing site's
 * scroll-zoom. Below the lg breakpoint the whole demo is one scroller, so
 * containing it would trap the reader; the rule only covers the two-pane
 * layout.
 */
const contain = document.createElement('style')
contain.textContent =
  '@media (min-width: 1024px) { body * { overscroll-behavior: contain; } }'
document.head.appendChild(contain)

const root = document.getElementById('root')
if (!root) throw new Error('missing #root container')

/**
 * The host page pauses the demo by posting `{ type: 'iii-demo', active }`
 * when the overlay opens and closes, so a hidden iframe is not burning a
 * timer loop. Without a host it just plays.
 */
function mount() {
  const reactRoot = createRoot(root as HTMLElement)
  let active = params.get('paused') !== '1'
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
