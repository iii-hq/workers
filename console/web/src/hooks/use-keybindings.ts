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
  matchDigitIndex,
  matchesKeybinding,
} from '@/lib/keybindings/registry'

/** A handler takes the selected index for a `digitIndex` shortcut, and
 *  nothing for every other one. */
export type KeybindingHandlers = Partial<
  Record<KeybindingActionId, (index: number) => void>
>

/** Whether the keystroke is going into a field the user is typing in. */
function isTyping(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null
  if (!element) return false
  if (element.isContentEditable) return true
  const tag = element.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
}

/** A modal owns the keyboard while it is open; only the palette toggle reaches through. */
export function insideDialog(target: EventTarget | null): boolean {
  const element = target as Element | null
  return (
    typeof element?.closest === 'function' &&
    element.closest('[role="dialog"],[role="alertdialog"]') !== null
  )
}

/**
 * A field may hand specific shortcuts back with
 * `data-keybindings-allow="workspace.selectByIndex panel.split"`. A search box
 * that opens focused would otherwise swallow the navigation keys for as long
 * as it holds the caret: after `t` opens a workspace, its page search has the
 * focus, so `\` and the workspace digits typed characters instead of moving.
 * Opt in per field and per action, never wholesale, so a field keeps every
 * key it actually needs to spell its own query.
 */
export function allowsWhileTyping(
  target: EventTarget | null,
  actionId: KeybindingActionId,
): boolean {
  const element = target as HTMLElement | null
  const allow = element?.dataset?.keybindingsAllow
  if (!allow) return false
  return allow.split(/[\s,]+/).includes(actionId)
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
      const inDialog = insideDialog(event.target)
      for (const definition of KEYBINDINGS) {
        if (definition.scope !== 'global') continue
        if (event.repeat) continue
        if (inDialog && !definition.firesWhileTyping) continue
        if (
          typing &&
          !definition.firesWhileTyping &&
          !allowsWhileTyping(event.target, definition.id)
        )
          continue
        const run = handlersRef.current[definition.id]
        if (!run) continue
        if (definition.digitIndex) {
          const index = matchDigitIndex(definition.id, event, platform)
          if (index === null) continue
          event.preventDefault()
          run(index)
          return
        }
        if (!matchesKeybinding(definition.id, event, platform)) continue
        event.preventDefault()
        run(0)
        return
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
}
