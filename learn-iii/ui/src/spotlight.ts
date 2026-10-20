/**
 * The spotlight: ONE fixed box in `document.body` that frames the console
 * element the current tour step talks about.
 *
 * Self-rolled on purpose. The whole animation is a CSS transition on
 * `translate`/`width`/`height` of a single element (styles.css), so a motion
 * library would add a dependency and a bundle for one moving rectangle. If
 * springs or a timeline are ever wanted, Motion One (`motion`, MIT, ~5 kB)
 * drops in here without touching the callers.
 *
 * The box carries what it frames: a halo that breathes (the shared pulse,
 * so reduced motion stills it) and a label naming the step, so the eye
 * lands on the element and knows why it is lit without reading the card.
 */

import { uiClasses } from '@iii-dev/console-ui'

const BOX_CLASS = 'learn-iii-spotlight'
/** Breathing room between the target's edge and the box. */
const PAD = 6
/** Room the label keeps from the window's right edge when it sits beside the box. */
const LABEL_EDGE = 16

/** Where the box sits, in viewport px; `null` while it frames nothing. */
export interface SpotlightRect {
  left: number
  top: number
  width: number
  height: number
}

let box: HTMLDivElement | null = null
let label: HTMLSpanElement | null = null
let frame = 0
let painted = ''
let rect: SpotlightRect | null = null
const movers = new Set<(rect: SpotlightRect | null) => void>()

function moved(next: SpotlightRect | null): void {
  rect = next
  for (const mover of [...movers]) mover(next)
}

/**
 * Hear where the box goes — the tour card keeps out of its way. Called at
 * once with where it is now, then on every move; returns the remover.
 */
export function onSpotlightMove(listener: (rect: SpotlightRect | null) => void): () => void {
  movers.add(listener)
  listener(rect)
  return () => {
    movers.delete(listener)
  }
}

function ensureBox(): HTMLDivElement {
  if (box?.isConnected) return box
  box = document.createElement('div')
  box.className = BOX_CLASS
  // Lives outside the injected subtree, so it carries the scope itself —
  // the injectable-UI rule for custom portals (styles.css is scope-only).
  box.setAttribute('data-iii-ui', 'learn-iii')
  box.setAttribute('aria-hidden', 'true')
  const halo = document.createElement('span')
  halo.className = `learn-iii-spotlight-halo ${uiClasses.pulse}`
  label = document.createElement('span')
  label.className = 'learn-iii-spotlight-label'
  box.append(halo, label)
  document.body.appendChild(box)
  return box
}

/**
 * How often, in frames, the label's side is checked again while the box sits
 * still: what lies around the target changes without the target moving (a
 * tab switch mounts a new pane under the label).
 */
const LABEL_RECHECK_FRAMES = 30

/**
 * ponytail: one `getBoundingClientRect` per frame while a step is showing,
 * instead of listening for scroll, resize, pane drags, tab switches and
 * layout shifts separately. It follows every one of them for free and writes
 * to the DOM only when the rect actually moves. Swap for a ResizeObserver +
 * scroll listeners if a profile ever blames this.
 */
function track(selectors: readonly string[]) {
  let sinceCheck = 0
  const step = () => {
    frame = requestAnimationFrame(step)
    // Best selector first: the tour's own `learn-iii-*` class, then the
    // console's stable hooks, so the box still lands on a console build that
    // predates the anchor classes.
    const target = firstMatch(selectors)
    const element = ensureBox()
    if (!target) {
      // The element can come back — a collapsed sidebar, another tab — so keep
      // watching and just stop drawing.
      element.dataset.visible = 'false'
      if (painted) {
        painted = ''
        moved(null)
      }
      return
    }
    const rect = target.getBoundingClientRect()
    const next = `${Math.round(rect.left - PAD)},${Math.round(rect.top - PAD)},${Math.round(
      rect.width + PAD * 2,
    )},${Math.round(rect.height + PAD * 2)}`
    element.dataset.visible = 'true'
    const [left, top, width, height] = next.split(',').map(Number)
    if (next === painted) {
      sinceCheck += 1
      if (sinceCheck >= LABEL_RECHECK_FRAMES) {
        sinceCheck = 0
        element.dataset.labelSide = labelSide(target, { left, top, width, height })
      }
      return
    }
    painted = next
    sinceCheck = 0
    element.style.translate = `${left}px ${top}px`
    element.style.width = `${width}px`
    element.style.height = `${height}px`
    element.dataset.labelSide = labelSide(target, { left, top, width, height })
    moved({ left, top, width, height })
  }
  frame = requestAnimationFrame(step)
}

type LabelSide = 'above' | 'end' | 'below' | 'none'

/** What the label must not sit on: anything the operator clicks or types in. */
const CONTROL = 'button, a[href], input, textarea, select, [role="tab"], [role="option"], [role="menuitem"]'

/**
 * Where the label goes: above the box, beside it, or under it — the first
 * that fits in the window and covers no control outside the target (the
 * tab next to a lit tab, the button under the tab row). When every side
 * would cover one, the label stays off: the card names the step anyway, and
 * a label over the control the operator is about to press is worse than
 * none.
 */
function labelSide(target: Element, box: SpotlightRect): LabelSide {
  if (!label || label.hidden) return 'none'
  const w = label.offsetWidth
  const h = label.offsetHeight
  const at: Record<Exclude<LabelSide, 'none'>, SpotlightRect> = {
    above: { left: box.left - 2, top: box.top - 8 - h, width: w, height: h },
    end: { left: box.left + box.width + 8, top: box.top + box.height / 2 - h / 2, width: w, height: h },
    below: { left: box.left - 2, top: box.top + box.height + 8, width: w, height: h },
  }
  for (const side of ['above', 'end', 'below'] as const) {
    const spot = at[side]
    const fits =
      spot.left >= 0 &&
      spot.top >= 0 &&
      spot.left + spot.width + LABEL_EDGE <= window.innerWidth &&
      spot.top + spot.height <= window.innerHeight
    if (fits && !coversControl(spot, target)) return side
  }
  return 'none'
}

/**
 * Whether a control outside `target` lies under `spot`, sampled at its two
 * ends and middle. The box and label are `pointer-events: none`, so the
 * hit test looks straight through them at the console.
 */
function coversControl(spot: SpotlightRect, target: Element): boolean {
  const y = spot.top + spot.height / 2
  for (const x of [spot.left + 2, spot.left + spot.width / 2, spot.left + spot.width - 2]) {
    for (const hit of document.elementsFromPoint(x, y)) {
      const control = hit.closest(CONTROL)
      if (control && !target.contains(control)) return true
    }
  }
  return false
}

/**
 * The first match the operator can actually SEE.
 *
 * `document.querySelector` returns the first in DOM order, which is not
 * always the live one. A tab can mount several chat columns, each carrying
 * its own `.learn-iii-composer`. A pane that is animating away stays mounted
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
 * on screen`. `console::workspace::open` returns once the layout is stored;
 * the console re-reads it when `console::workspace::changed` rings (or, on an
 * older console without that trigger, on its five-second poll), so the panel
 * mounts a round trip or more after the call returns. Watching for the
 * element is the only signal that covers the whole trip.
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

/**
 * Frame the first selector that matches, named by `caption` (`2 · Workspaces`);
 * an empty list clears the box.
 */
export function showSpotlight(selectors: readonly string[] | null | undefined, caption?: string): void {
  hideSpotlight()
  if (!selectors || selectors.length === 0) return
  ensureBox()
  if (label) {
    label.textContent = caption ?? ''
    label.hidden = !caption
  }
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
  if (rect) moved(null)
}

/** Drop the box entirely — for when the page unmounts. */
export function disposeSpotlight(): void {
  hideSpotlight()
  box?.remove()
  box = null
  label = null
}
