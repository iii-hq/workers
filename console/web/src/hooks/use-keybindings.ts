/**
 * The console's one global key listener.
 *
 * Everything with `scope: 'global'` in the registry is dispatched from here,
 * so the set of keys the console claims is readable in one place instead of
 * spread across whichever components happened to want one.
 */

import { useEffect, useRef } from 'react'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  KEYBINDINGS,
  type KeybindingActionId,
  matchesKeybinding,
} from '@/lib/keybindings/registry'

export type KeybindingHandlers = Partial<Record<KeybindingActionId, () => void>>

/** Whether the keystroke is going into a field the user is typing in. */
function isTyping(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null
  if (!element) return false
  if (element.isContentEditable) return true
  const tag = element.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
}

export function useKeybindings(handlers: KeybindingHandlers): void {
  // Read through a ref so the listener binds once: handlers are rebuilt every
  // render by every caller that passes inline arrows.
  const handlersRef = useRef(handlers)
  handlersRef.current = handlers

  useEffect(() => {
    if (typeof window === 'undefined') return
    const platform = shortcutPlatform()
    const onKeyDown = (event: KeyboardEvent) => {
      // A shortcut must not fire off the keystroke that finishes a character
      // being composed by an IME.
      if (event.isComposing) return
      const typing = isTyping(event.target)
      for (const definition of KEYBINDINGS) {
        if (definition.scope !== 'global') continue
        if (typing && !definition.firesWhileTyping) continue
        const run = handlersRef.current[definition.id]
        if (!run) continue
        if (!matchesKeybinding(definition.id, event, platform)) continue
        event.preventDefault()
        run()
        return
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
}
