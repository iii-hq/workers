import { useSyncExternalStore } from 'react'

/**
 * What New chat opens — the sidebar row, a tab strip's plus, ⌘N and the
 * palette alike:
 *
 *   - `page`   — a blank chat: the welcome screen with the composer centred.
 *   - `dialog` — the quick dialog: project, first message and model in one
 *                step, and the chat starts with that message on its way.
 *
 * Per browser, like the theme. A blank chat is the default: it is the full
 * composer (attachments, mentions, voice, starter prompts) with nothing in
 * front of it.
 */
export type NewChatMode = 'page' | 'dialog'

export const DEFAULT_NEW_CHAT_MODE: NewChatMode = 'page'

const KEY = 'iii-new-chat-mode'
const CHANGE_EVENT = 'iii:new-chat-mode'

export function loadNewChatMode(): NewChatMode {
  try {
    const stored = localStorage.getItem(KEY)
    return stored === 'dialog' || stored === 'page'
      ? stored
      : DEFAULT_NEW_CHAT_MODE
  } catch {
    return DEFAULT_NEW_CHAT_MODE
  }
}

export function saveNewChatMode(mode: NewChatMode): void {
  try {
    localStorage.setItem(KEY, mode)
  } catch {
    // best-effort persistence; this tab still follows the choice
  }
  window.dispatchEvent(new Event(CHANGE_EVENT))
}

function subscribe(onChange: () => void): () => void {
  // Another tab's change arrives as a storage event; this tab's as ours.
  const onStorage = (event: StorageEvent) => {
    if (event.key === KEY) onChange()
  }
  window.addEventListener(CHANGE_EVENT, onChange)
  window.addEventListener('storage', onStorage)
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange)
    window.removeEventListener('storage', onStorage)
  }
}

export function useNewChatMode(): NewChatMode {
  return useSyncExternalStore(
    subscribe,
    loadNewChatMode,
    () => DEFAULT_NEW_CHAT_MODE,
  )
}
