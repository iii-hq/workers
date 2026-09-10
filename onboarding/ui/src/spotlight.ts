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

function firstMatch(selectors: readonly string[]): Element | null {
  for (const selector of selectors) {
    const found = document.querySelector(selector)
    if (found) return found
  }
  return null
}

/** Frame the first selector that matches; an empty list clears the box. */
export function showSpotlight(selectors: readonly string[] | null | undefined): void {
  hideSpotlight()
  if (!selectors || selectors.length === 0) return
  firstMatch(selectors)?.scrollIntoView({
    block: 'center',
    inline: 'nearest',
    behavior: 'smooth',
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
