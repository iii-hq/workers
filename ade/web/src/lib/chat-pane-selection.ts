/**
 * Which chat each workspace chat pane shows. The conversation store has one
 * active chat; a pane that only mirrored it showed whatever was picked last
 * anywhere, so picking Chat B in tab B also put Chat B in tab A. Each pane
 * now keeps its own chat, remembered per `<tab>:<pane>` (localStorage, so it
 * survives a reload), and the store's active chat is the one in the pane the
 * person used last: the LEAD pane.
 *
 * - The lead follows the active chat (a palette pick, a page that started a
 *   turn, a chat opened from inside the pane); the other panes keep theirs.
 * - A pane becomes the lead when it is pressed or focused, or when it mounts
 *   while no lead is on screen (switching workspace tabs), and hands its own
 *   chat back to the store then.
 * - An outside request to show a chat while no chat pane is mounted waits
 *   for the pane that opens for it, so that pane shows the request instead
 *   of the chat it remembered.
 *
 * Two browser tabs of the console share localStorage, so a pane remembered
 * only there came back after a reload on whatever the other tab picked
 * last. Each browser tab keeps its own copy in sessionStorage (per tab, and
 * it survives a reload); localStorage only seeds a tab opened fresh.
 */

import { onChatShownRequest } from '@/lib/chat-requests'

const STORAGE_KEY = 'console:chat:pane-chats'
/** Panes remembered at most; the oldest entries leave first. */
const MAX_REMEMBERED = 64

export function chatPaneKey(tabId: string, paneId: string): string {
  return `${tabId}:${paneId}`
}

function readAll(store: Storage | undefined): Record<string, string> {
  try {
    const raw = JSON.parse(store?.getItem(STORAGE_KEY) ?? 'null')
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
    const out: Record<string, string> = {}
    for (const [key, value] of Object.entries(raw)) {
      if (typeof value === 'string' && value.length > 0) out[key] = value
    }
    return out
  } catch {
    return {}
  }
}

const tabStore = () => globalThis.sessionStorage as Storage | undefined
const sharedStore = () => globalThis.localStorage as Storage | undefined

/**
 * The chat a pane showed last in this browser tab, else the one any tab
 * showed in it last (a tab opened fresh), or `null` for a pane never used.
 */
export function loadPaneChat(paneKey: string): string | null {
  return readAll(tabStore())[paneKey] ?? readAll(sharedStore())[paneKey] ?? null
}

function write(store: Storage | undefined, paneKey: string, id: string) {
  const all = readAll(store)
  if (all[paneKey] === id) return
  delete all[paneKey]
  all[paneKey] = id
  const keys = Object.keys(all)
  for (const stale of keys.slice(0, Math.max(0, keys.length - MAX_REMEMBERED)))
    delete all[stale]
  try {
    store?.setItem(STORAGE_KEY, JSON.stringify(all))
  } catch {
    // best-effort
  }
}

export function savePaneChat(paneKey: string, conversationId: string): void {
  write(tabStore(), paneKey, conversationId)
  write(sharedStore(), paneKey, conversationId)
}

const mounted: string[] = []
let lead: string | null = null
let pendingShown: string | null = null

onChatShownRequest((conversationId) => {
  // A mounted lead follows the active chat on its own.
  pendingShown = mounted.length === 0 ? conversationId : null
})

/**
 * Register a mounted pane. It becomes the lead when none is on screen; a
 * lead that leaves hands over to a pane still mounted. Returns the release
 * to call on unmount.
 */
export function mountChatPane(paneKey: string): () => void {
  mounted.push(paneKey)
  // The first pane to mount after the request was the one opened for it.
  pendingShown = null
  if (lead === null || !mounted.includes(lead)) lead = paneKey
  return () => {
    const index = mounted.indexOf(paneKey)
    if (index !== -1) mounted.splice(index, 1)
    if (lead === paneKey) lead = mounted[0] ?? null
  }
}

export function claimChatPaneLead(paneKey: string): void {
  lead = paneKey
}

export function isChatPaneLead(paneKey: string): boolean {
  return lead === paneKey
}

/**
 * The chat an outside request asked for while no chat pane was mounted; the
 * next pane to mount shows it (and clears it by mounting).
 */
export function pendingShownChat(): string | null {
  return pendingShown
}

/** Test seam: forget every pane, lead and pending request. */
export function resetChatPanesForTest(): void {
  mounted.length = 0
  lead = null
  pendingShown = null
}
