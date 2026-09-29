/**
 * The spotlight: ONE fixed box in `document.body` that frames the console
 * element the current tour step talks about.
 *
 * Self-rolled on purpose. The whole animation is a CSS transition on
 * `translate`/`width`/`height` of a single element (styles.css), so a motion
 * library would add a dependency and a bundle for one moving rectangle. If
 * springs or a timeline are ever wanted, Motion One (`motion`, MIT, ~5 kB)
 * drops in here without touching the callers.
 */

const BOX_CLASS = 'onboarding-spotlight'
/** Breathing room between the target's edge and the box. */
const PAD = 6

let box: HTMLDivElement | null = null
let frame = 0
let painted = ''

function ensureBox(): HTMLDivElement {
  if (box?.isConnected) return box
  box = document.createElement('div')
  box.className = BOX_CLASS
  // Lives outside the injected subtree, so it carries the scope itself —
  // the injectable-UI rule for custom portals (styles.css is scope-only).
  box.setAttribute('data-iii-ui', 'onboarding')
  box.setAttribute('aria-hidden', 'true')
  document.body.appendChild(box)
  return box
}

/**
 * ponytail: one `getBoundingClientRect` per frame while a step is showing,
 * instead of listening for scroll, resize, pane drags, tab switches and
 * layout shifts separately. It follows every one of them for free and writes
 * to the DOM only when the rect actually moves. Swap for a ResizeObserver +
 * scroll listeners if a profile ever blames this.
 */
function track(selectors: readonly string[]) {
  const step = () => {
    frame = requestAnimationFrame(step)
    // Best selector first: the tour's own `onboarding-*` class, then the
    // console's stable hooks, so the box still lands on a console build that
    // predates the anchor classes.
    const target = firstMatch(selectors)
    const element = ensureBox()
    if (!target) {
      // The element can come back — a collapsed sidebar, another tab — so keep
      // watching and just stop drawing.
      element.dataset.visible = 'false'
      return
    }
    const rect = target.getBoundingClientRect()
    const next = `${Math.round(rect.left - PAD)},${Math.round(rect.top - PAD)},${Math.round(
      rect.width + PAD * 2,
    )},${Math.round(rect.height + PAD * 2)}`
    element.dataset.visible = 'true'
    if (next === painted) return
    painted = next
    const [left, top, width, height] = next.split(',')
    element.style.translate = `${left}px ${top}px`
    element.style.width = `${width}px`
    element.style.height = `${height}px`
  }
  frame = requestAnimationFrame(step)
}

/**
 * The first match the operator can actually SEE.
 *
 * `document.querySelector` returns the first in DOM order, which is not
 * always the live one. A tab can mount several chat columns, each carrying
 * its own `.onboarding-composer`. A pane that is animating away stays mounted
 * with `inert` and `aria-hidden` until the transition ends. And a
 * desktop-only anchor can sit inside a `hidden sm:flex` header on a narrow
 * layout — present in the document, never painted. Framing any of those puts
 * the box over nothing, and lets `waitForAnchor` call a panel open before it
 * is on screen.
 *
 * ponytail: judged by layout box and inert/aria-hidden ancestry rather than
 * by matching the active conversation id. The box only has to land on
 * something visible; plumbing the session through would buy a precision this
 * has no use for.
 */
function firstMatch(selectors: readonly string[]): Element | null {
  for (const selector of selectors) {
    for (const found of document.querySelectorAll(selector)) {
      if (isShowing(found)) return found
    }
  }
  return null
}

/** On screen: it has a layout box, and nothing above it is inert or hidden. */
function isShowing(element: Element): boolean {
  if (element.getClientRects().length === 0) return false
  return element.closest('[inert], [aria-hidden="true"]') === null
}

/**
 * Resolve once one of `selectors` is in the document, or give up after
 * `timeoutMs`. Reports whether the element actually landed.
 *
 * This is what tells `the engine stored the layout` apart from `the panel is
 * on screen`. `console::workspace::open` returns in a few milliseconds, but
 * the console re-reads that layout on a five-second poll, so a panel another
 * worker opens can take up to five seconds to mount. Watching for the element
 * is the only signal that covers the whole trip.
 *
 * ponytail: one `querySelector` per animation frame — the budget the
 * spotlight already spends — rather than a MutationObserver over the
 * console's whole tree. It stops as soon as the element lands. A background
 * tab throttles the frames, which only means the wait finishes when the
 * operator comes back to look.
 */
export function waitForAnchor(selectors: readonly string[] | null | undefined, timeoutMs: number): Promise<boolean> {
  if (!selectors || selectors.length === 0) return Promise.resolve(false)
  return new Promise((resolve) => {
    const deadline = Date.now() + timeoutMs
    const look = () => {
      if (firstMatch(selectors)) return resolve(true)
      // The ceiling is not an error. A console that never mounts the panel —
      // an older build, a screen it does not have — must still let the
      // operator past the step.
      if (Date.now() >= deadline) return resolve(false)
      requestAnimationFrame(look)
    }
    look()
  })
}

/**
 * A scroll animation is not reachable from CSS, so the stylesheet's
 * `prefers-reduced-motion` block cannot cover this one — it is asked for
 * here.
 */
const reducedMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true

/** Frame the first selector that matches; an empty list clears the box. */
export function showSpotlight(selectors: readonly string[] | null | undefined): void {
  hideSpotlight()
  if (!selectors || selectors.length === 0) return
  firstMatch(selectors)?.scrollIntoView({
    block: 'center',
    inline: 'nearest',
    behavior: reducedMotion() ? 'auto' : 'smooth',
  })
  track(selectors)
}

export function hideSpotlight(): void {
  if (frame) cancelAnimationFrame(frame)
  frame = 0
  painted = ''
  if (box) box.dataset.visible = 'false'
}

/** Drop the box entirely — for when the page unmounts. */
export function disposeSpotlight(): void {
  hideSpotlight()
  box?.remove()
  box = null
}
