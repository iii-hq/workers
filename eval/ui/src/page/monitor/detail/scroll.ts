/** Brings an element into view, without motion when the viewer asks for none. */
export function scrollToElement(element: HTMLElement | null | undefined): void {
  if (!element) return
  let reduced = false
  try {
    reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    // matchMedia can be missing in an embedded view; smooth is the default then.
  }
  element.scrollIntoView({ block: 'center', behavior: reduced ? 'auto' : 'smooth' })
  element.focus({ preventScroll: true })
}
