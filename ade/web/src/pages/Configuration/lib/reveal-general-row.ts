import { hashForView } from '@/hooks/use-hash-route'

/**
 * Scroll a General row into view and flash it, once it is mounted (the page
 * may still be transitioning in). The first id that exists wins, so a hidden
 * row falls back to its neighbour.
 */
export function revealGeneralRow(anchors: string[]) {
  if (anchors.length === 0 || typeof window === 'undefined') return
  const reduceMotion = window.matchMedia?.(
    '(prefers-reduced-motion: reduce)',
  ).matches
  let frames = 0
  const tick = () => {
    const target = anchors
      .map((anchor) => document.getElementById(anchor))
      .find((element): element is HTMLElement => element !== null)
    if (!target) {
      if (++frames < 60) window.requestAnimationFrame(tick)
      return
    }
    target.scrollIntoView({
      block: 'center',
      behavior: reduceMotion ? 'auto' : 'smooth',
    })
    target.setAttribute('data-search-flash', '')
    window.setTimeout(() => target.removeAttribute('data-search-flash'), 1600)
  }
  window.requestAnimationFrame(tick)
}

/**
 * Open the General settings tab and scroll to one of `anchors`: the way a
 * control elsewhere in the ADE (the composer's Auto mode gear) links to the
 * row that configures it. Settings already open just scroll.
 */
export function openGeneralSettingsRow(anchors: string[]) {
  if (typeof window === 'undefined') return
  const target = hashForView('configuration')
  if (window.location.hash !== target) window.location.hash = target
  revealGeneralRow(anchors)
}
