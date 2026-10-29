/**
 * The chats open as tabs in the chat header, the way an editor keeps its
 * open files: a chat you open joins the end of the row; past
 * MAX_OPEN_CHAT_TABS the one you used least recently leaves (never the one
 * on screen); closing a tab hands the screen to its right neighbour, else
 * its left. The row outlives a reload (localStorage).
 */

export const MAX_OPEN_CHAT_TABS = 5
const STORAGE_KEY = 'console:chat:open-tabs'

export interface OpenChatTabs {
  /** Tab order, left to right. */
  ids: readonly string[]
  /** Use order, least recent first; holds exactly the ids in `ids`. */
  recency: readonly string[]
}

export const NO_OPEN_CHAT_TABS: OpenChatTabs = { ids: [], recency: [] }

/** `id` is on screen: give it a tab (evicting the stalest) and mark it used. */
export function openChatTab(
  state: OpenChatTabs,
  id: string,
  max = MAX_OPEN_CHAT_TABS,
): OpenChatTabs {
  let recency = [...state.recency.filter((other) => other !== id), id]
  if (state.ids.includes(id)) {
    return state.recency.at(-1) === id ? state : { ids: state.ids, recency }
  }
  let ids = [...state.ids, id]
  while (ids.length > max) {
    const stalest = recency.find((other) => other !== id && ids.includes(other))
    if (stalest === undefined) break
    ids = ids.filter((other) => other !== stalest)
    recency = recency.filter((other) => other !== stalest)
  }
  return { ids, recency }
}

/** Close `id`'s tab; `next` is the tab to show when `id` was on screen. */
export function closeChatTab(
  state: OpenChatTabs,
  id: string,
): { state: OpenChatTabs; next: string | null } {
  const index = state.ids.indexOf(id)
  if (index === -1) return { state, next: null }
  const ids = state.ids.filter((other) => other !== id)
  return {
    state: { ids, recency: state.recency.filter((other) => other !== id) },
    next: state.ids[index + 1] ?? state.ids[index - 1] ?? null,
  }
}

/** Drop tabs whose chat is gone (deleted, or not one a tab shows). */
export function pruneChatTabs(
  state: OpenChatTabs,
  keep: (id: string) => boolean,
): OpenChatTabs {
  if (state.ids.every(keep)) return state
  return {
    ids: state.ids.filter(keep),
    recency: state.recency.filter(keep),
  }
}

export function loadOpenChatTabs(): OpenChatTabs {
  try {
    const raw = JSON.parse(
      globalThis.localStorage?.getItem(STORAGE_KEY) ?? 'null',
    )
    const ids = Array.isArray(raw?.ids)
      ? raw.ids.filter((id: unknown): id is string => typeof id === 'string')
      : []
    const known = new Set(ids)
    const recency = Array.isArray(raw?.recency)
      ? raw.recency.filter(
          (id: unknown): id is string =>
            typeof id === 'string' && known.has(id),
        )
      : []
    // Every tab appears in the use order exactly once.
    for (const id of ids) if (!recency.includes(id)) recency.unshift(id)
    return { ids: ids.slice(-MAX_OPEN_CHAT_TABS), recency }
  } catch {
    return NO_OPEN_CHAT_TABS
  }
}

export function saveOpenChatTabs(state: OpenChatTabs): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(state))
  } catch {
    // Private mode or a full quota: the row just doesn't survive a reload.
  }
}
