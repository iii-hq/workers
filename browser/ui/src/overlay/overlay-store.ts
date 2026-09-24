/**
 * Which sessions the live preview shows, in stacking order (last = front),
 * shared between the overlay (which subscribes) and the surfaces that
 * take a card over (the expand control, which hides it). A tab the user
 * opened themselves never gets a card: its start event carries
 * `preview: false`, so the overlay skips it in every console window.
 */

import type { Host } from '@iii-dev/console-ui'

let order: readonly string[] = []
const listeners = new Set<() => void>()

function set(next: readonly string[]) {
  order = next
  for (const listener of [...listeners]) listener()
}

export function subscribeBrowserOverlay(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** Sessions on the deck, back to front. Empty for no overlay. */
export function browserOverlaySessions(): readonly string[] {
  return order
}

/** A session started: it lands on top. */
export function showBrowserOverlay(sessionId: string): void {
  if (order[order.length - 1] === sessionId) return
  set([...order.filter((id) => id !== sessionId), sessionId])
}

/** The user picked a card from the deck: it comes to the front. */
export function bringBrowserOverlayToFront(sessionId: string): void {
  if (!order.includes(sessionId)) return
  showBrowserOverlay(sessionId)
}

/** Drop the card for this session (hidden, expanded, or stopped). */
export function hideBrowserOverlay(sessionId: string): void {
  if (order.includes(sessionId)) set(order.filter((id) => id !== sessionId))
}

/**
 * Open the browser page on a session, the one explicit way in: the card
 * for it goes away since the page now shows it.
 */
export function openBrowserPane(host: Host, sessionId: string): void {
  hideBrowserOverlay(sessionId)
  host.panels?.open({ pageId: 'browser', context: { sessionId } })
}

/** Test seam. */
export function resetBrowserOverlayStore(): void {
  order = []
}
