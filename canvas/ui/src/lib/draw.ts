/**
 * Draw-on animation for a freshly rendered mermaid SVG — every stroke
 * sketches itself in (dashoffset sweep, staggered), text fades up after.
 * Shared by the page preview and the chat cards so a diagram "draws" the
 * same way everywhere it appears.
 *
 * Inline dash styles are removed once the sweep lands: mermaid uses real
 * dasharray for dotted/dashed edges, and leaving the animation values
 * behind would repaint those as solid.
 */

const MAX_ANIMATED_PATHS = 400
const STROKE_MS = 420
const STAGGER_MS = 14
const MAX_DELAY_MS = 900

export function animateSvgDrawIn(container: HTMLElement): void {
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return
  const svg = container.querySelector('svg')
  if (!svg) return

  const paths = Array.from(svg.querySelectorAll('path')).slice(
    0,
    MAX_ANIMATED_PATHS,
  )
  let lastDelay = 0
  for (const [i, p] of paths.entries()) {
    let len = 0
    try {
      len = p.getTotalLength()
    } catch {
      continue
    }
    if (!Number.isFinite(len) || len <= 0) continue
    const delay = Math.min(i * STAGGER_MS, MAX_DELAY_MS)
    lastDelay = Math.max(lastDelay, delay)
    p.style.transition = 'none'
    p.style.strokeDasharray = `${len}`
    p.style.strokeDashoffset = `${len}`
    requestAnimationFrame(() => {
      p.style.transition = `stroke-dashoffset ${STROKE_MS}ms ease ${delay}ms`
      p.style.strokeDashoffset = '0'
    })
  }

  const labels = Array.from(
    svg.querySelectorAll<SVGElement>('text, foreignObject, image'),
  )
  for (const [i, el] of labels.entries()) {
    const delay = Math.min(160 + i * 10, MAX_DELAY_MS)
    el.style.transition = 'none'
    el.style.opacity = '0'
    requestAnimationFrame(() => {
      el.style.transition = `opacity 260ms ease ${delay}ms`
      el.style.opacity = '1'
    })
  }

  window.setTimeout(
    () => {
      for (const p of paths) {
        p.style.removeProperty('transition')
        p.style.removeProperty('stroke-dasharray')
        p.style.removeProperty('stroke-dashoffset')
      }
      for (const el of labels) {
        el.style.removeProperty('transition')
        el.style.removeProperty('opacity')
      }
    },
    lastDelay + STROKE_MS + 400,
  )
}
