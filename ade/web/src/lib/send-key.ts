import { useSyncExternalStore } from 'react'
import {
  bindingMatchesEvent,
  type Platform,
  shortcutPlatform,
} from '@/lib/keybindings/bindings'

/**
 * The key that sends a chat message from the composer (and the quick New
 * chat dialog):
 *
 *   - `enter`       — Enter sends, Shift+Enter breaks the line.
 *   - `mod-enter`   — ⌘↵ (Ctrl+Enter off a Mac) sends; Enter breaks the line.
 *   - `shift-enter` — Shift+Enter sends; Enter breaks the line.
 *
 * Mod+Enter sends whatever the choice, so it is never a stray newline for
 * anyone with the habit. Per browser, like the theme; Enter is the default.
 */
export type SendKey = 'enter' | 'mod-enter' | 'shift-enter'

export const DEFAULT_SEND_KEY: SendKey = 'enter'

/** Sends under every choice. */
const SEND_CHORD = 'Mod+Enter'

const KEY = 'iii-send-key'
const CHANGE_EVENT = 'iii:send-key'

const SEND_BINDINGS: Record<SendKey, string> = {
  enter: 'Enter',
  'mod-enter': 'Mod+Enter',
  'shift-enter': 'Shift+Enter',
}

function isSendKey(value: unknown): value is SendKey {
  return value === 'enter' || value === 'mod-enter' || value === 'shift-enter'
}

/** The chosen send key as a binding (`Enter`, `Mod+Enter`, `Shift+Enter`). */
export function sendBinding(sendKey: SendKey): string {
  return SEND_BINDINGS[sendKey]
}

/** True for an Enter that sends under `sendKey`. */
export function isSendEvent(
  sendKey: SendKey,
  event: KeyboardEvent | React.KeyboardEvent,
  platform: Platform = shortcutPlatform(),
): boolean {
  return (
    bindingMatchesEvent(sendBinding(sendKey), event, platform) ||
    bindingMatchesEvent(SEND_CHORD, event, platform)
  )
}

/**
 * True for an Enter that breaks the line under `sendKey`: Shift+Enter when
 * Enter sends, otherwise a bare Enter (and Shift+Enter too while ⌘↵ sends).
 * Any other chord (Alt+Enter, Ctrl+Enter on a Mac) is neither.
 */
export function isNewLineEvent(
  sendKey: SendKey,
  event: KeyboardEvent | React.KeyboardEvent,
  platform: Platform = shortcutPlatform(),
): boolean {
  const enter = bindingMatchesEvent('Enter', event, platform)
  const shiftEnter = bindingMatchesEvent('Shift+Enter', event, platform)
  if (sendKey === 'enter') return shiftEnter
  if (sendKey === 'shift-enter') return enter
  return enter || shiftEnter
}

export function loadSendKey(): SendKey {
  try {
    const stored = localStorage.getItem(KEY)
    return isSendKey(stored) ? stored : DEFAULT_SEND_KEY
  } catch {
    return DEFAULT_SEND_KEY
  }
}

export function saveSendKey(sendKey: SendKey): void {
  try {
    localStorage.setItem(KEY, sendKey)
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

export function useSendKey(): SendKey {
  return useSyncExternalStore(subscribe, loadSendKey, () => DEFAULT_SEND_KEY)
}
