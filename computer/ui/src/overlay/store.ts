/**
 * Which desktop sessions the live preview shows, in stacking order (last =
 * front), shared between the overlay, its expand control and the computer
 * page (the ios-simulator overlay's store). A card never covers a session the
 * page shows live in this window: the page's own viewport input fires
 * `computer::activity` too, and must not pop a preview of itself.
 */

import type { Host } from '@iii-dev/console-ui'

/** A hidden card stays hidden while activity keeps coming at least this often. */
const DISMISS_IDLE_MS = 120_000

let order: readonly string[] = []
const listeners = new Set<() => void>()
/** Sessions the page shows live right now (one count per mount). */
const onPage = new Map<string, number>()
/** Sessions the user hid, with the last activity seen since. */
const dismissed = new Map<string, number>()
/** Sessions that gave no picture (a Wayland desktop): never on the deck. */
const blind = new Set<string>()

function set(next: readonly string[]) {
  order = next
  for (const listener of [...listeners]) listener()
}

export function subscribeOverlay(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function overlaySessions(): readonly string[] {
  return order
}

/**
 * Bring a session's card up. Plain activity (`computer::act`) respects a hide
 * until it pauses for DISMISS_IDLE_MS; `force` (a new `computer::run`) doesn't,
 * and it retries a session marked blind too: a capture error before the first
 * frame can be passing, and a desktop that truly has no picture marks itself
 * again on its first error.
 */
export function showCard(sessionId: string, force = false, now = Date.now()): void {
  if (onPage.has(sessionId)) return
  if (force) blind.delete(sessionId)
  else if (blind.has(sessionId)) return
  const hiddenAt = dismissed.get(sessionId)
  if (!force && hiddenAt !== undefined && now - hiddenAt < DISMISS_IDLE_MS) {
    dismissed.set(sessionId, now)
    return
  }
  dismissed.delete(sessionId)
  if (order[order.length - 1] === sessionId) return
  set([...order.filter((id) => id !== sessionId), sessionId])
}

export function bringToFront(sessionId: string): void {
  if (order.includes(sessionId)) showCard(sessionId, true)
}

/** Drop the card (the page took it over, or it has no picture). */
export function hideCard(sessionId: string): void {
  if (order.includes(sessionId)) set(order.filter((id) => id !== sessionId))
}

/** The session has no picture to show: off the deck until it stops or a run retries it. */
export function markBlind(sessionId: string): void {
  blind.add(sessionId)
  hideCard(sessionId)
}

/** The session stopped: drop its card and everything remembered about it. */
export function forgetSession(sessionId: string): void {
  blind.delete(sessionId)
  dismissed.delete(sessionId)
  hideCard(sessionId)
}

/** The user's hide: activity won't bring it straight back. */
export function dismissCard(sessionId: string, now = Date.now()): void {
  dismissed.set(sessionId, now)
  hideCard(sessionId)
}

/** The page shows this session live: no card for it until released. */
export function claimOnPage(sessionId: string): () => void {
  onPage.set(sessionId, (onPage.get(sessionId) ?? 0) + 1)
  hideCard(sessionId)
  return () => {
    const left = (onPage.get(sessionId) ?? 1) - 1
    if (left > 0) onPage.set(sessionId, left)
    else onPage.delete(sessionId)
  }
}

/** Open the computer page on this session; its card goes away. */
export function openComputerPane(host: Host, sessionId: string): void {
  hideCard(sessionId)
  host.panels?.open({ pageId: 'computer', context: { sessionId } })
}

/** Test seam. */
export function resetOverlayStore(): void {
  order = []
  onPage.clear()
  dismissed.clear()
  blind.clear()
}
