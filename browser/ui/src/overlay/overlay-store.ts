/**
 * Which sessions the live preview shows, in stacking order (last = front),
 * shared between the overlay (which subscribes) and the surfaces that
 * already show a tab on purpose (which dismiss). A tab the user opened
 * themselves — the page's own new-tab button, "Open in browser" on a scrape
 * result — never needs a thumbnail; and a session-started event may land
 * before or after the call that opened it resolves, so a dismissal both
 * removes the card and blocks a late event for that id.
 */

import type { Host } from '@iii-dev/console-ui'

let order: readonly string[] = []
const dismissed = new Set<string>()
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

/** A session started: it lands on top, unless it was dismissed already. */
export function showBrowserOverlay(sessionId: string): void {
  if (dismissed.has(sessionId)) return
  if (order[order.length - 1] === sessionId) return
  set([...order.filter((id) => id !== sessionId), sessionId])
}

/** The user picked a card from the deck: it comes to the front. */
export function bringBrowserOverlayToFront(sessionId: string): void {
  if (!order.includes(sessionId)) return
  showBrowserOverlay(sessionId)
}

/** Drop the card for this session and keep it away for its lifetime. */
export function dismissBrowserOverlay(sessionId: string): void {
  dismissed.add(sessionId)
  if (order.includes(sessionId)) set(order.filter((id) => id !== sessionId))
}

/** A session ended: nothing to show, nothing left to remember. */
export function forgetBrowserOverlay(sessionId: string): void {
  dismissed.delete(sessionId)
  if (order.includes(sessionId)) set(order.filter((id) => id !== sessionId))
}

/**
 * Open the browser page on a session, the one explicit way in: the card
 * for it goes away since the page now shows it.
 */
export function openBrowserPane(host: Host, sessionId: string): void {
  dismissBrowserOverlay(sessionId)
  host.panels?.open({ pageId: 'browser', context: { sessionId } })
}

/** Test seam. */
export function resetBrowserOverlayStore(): void {
  order = []
  dismissed.clear()
}
