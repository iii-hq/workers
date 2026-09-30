/**
 * Which simulators the live preview shows, in stacking order (last = front),
 * shared between the overlay, the expand control and the Simulators page.
 * Keys are `<tenant>/<udid>`. A card never covers a simulator the page shows
 * live in this window, and a boot started from the page never gets one: its
 * `device-changed` event carries `preview: false`.
 */

import type { Host } from '@iii-dev/console-ui'

/** A hidden card stays hidden while activity keeps coming at least this often. */
const DISMISS_IDLE_MS = 120_000

let order: readonly string[] = []
const listeners = new Set<() => void>()
/** Keys the Simulators page shows live right now (one count per mount). */
const onPage = new Map<string, number>()
/** Keys the user hid, with the last activity seen since. */
const dismissed = new Map<string, number>()

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

export function overlayCards(): readonly string[] {
  return order
}

/**
 * Bring a simulator's card up. Plain agent activity respects a hide until it
 * pauses for DISMISS_IDLE_MS; `force` (a boot, `ios-simulator::open`) doesn't.
 */
export function showCard(key: string, force = false): void {
  if (onPage.has(key)) return
  const hiddenAt = dismissed.get(key)
  if (!force && hiddenAt !== undefined && Date.now() - hiddenAt < DISMISS_IDLE_MS) {
    dismissed.set(key, Date.now())
    return
  }
  dismissed.delete(key)
  if (order[order.length - 1] === key) return
  set([...order.filter((k) => k !== key), key])
}

export function bringToFront(key: string): void {
  if (order.includes(key)) showCard(key)
}

export function hideCard(key: string): void {
  if (order.includes(key)) set(order.filter((k) => k !== key))
}

/** The user's hide: agent activity won't bring it straight back. */
export function dismissCard(key: string): void {
  dismissed.set(key, Date.now())
  hideCard(key)
}

/** The page shows this simulator live: no card for it until released. */
export function claimOnPage(key: string): () => void {
  onPage.set(key, (onPage.get(key) ?? 0) + 1)
  hideCard(key)
  return () => {
    const left = (onPage.get(key) ?? 1) - 1
    if (left > 0) onPage.set(key, left)
    else onPage.delete(key)
  }
}

/** Open the Simulators page on this simulator; its card goes away. */
export function openSimulatorPane(host: Host, key: string): void {
  hideCard(key)
  const [tenant, udid] = key.split('/')
  host.panels?.open({ pageId: 'ios-simulator', context: { tenant, udid } })
}
