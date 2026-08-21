/**
 * The console's one global key listener.
 *
 * Everything with `scope: 'global'` in the registry is dispatched from here,
 * so the set of keys the console claims is readable in one place instead of
 * spread across whichever components happened to want one.
 */

import { useEffect, useRef } from 'react'
import {
  bindingMatchesEvent,
  isSequence,
  type KeyEventLike,
  type Platform,
  shortcutPlatform,
  splitSequence,
} from '@/lib/keybindings/bindings'
import {
  KEYBINDINGS,
  type KeybindingActionId,
  type KeybindingDefinition,
  matchDigitIndex,
  matchesKeybinding,
  sequencesFor,
} from '@/lib/keybindings/registry'
import { paneCommands, type RegisteredPageCommand } from '@/lib/page-commands'
import { paneRootOf } from '@/lib/pane-focus'

/** How long a chord prefix waits for its next key before it is forgotten. */
export const CHORD_TIMEOUT_MS = 1500

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
  actionId: KeybindingActionId | string,
): boolean {
  const element = target as HTMLElement | null
  const allow = element?.dataset?.keybindingsAllow
  if (!allow) return false
  return allow.split(/[\s,]+/).includes(actionId)
}

export type DispatchEvent = KeyEventLike &
  Pick<KeyboardEvent, 'isComposing' | 'repeat' | 'target'> & {
    defaultPrevented?: boolean
    preventDefault: () => void
  }

interface ChordCandidate {
  chords: readonly string[]
  run: () => void
}

interface PendingChord {
  candidates: ChordCandidate[]
  depth: number
  timer: ReturnType<typeof setTimeout>
}

/** The keyed commands of the pane that holds the focus, if any. */
export type PaneCommandsSource = (
  paneId: string,
) => readonly RegisteredPageCommand[]

/**
 * The keystroke-to-handler logic, separated from the window listener so it
 * can be driven in a test. A sequence binding (`G C`) is matched one chord at
 * a time: the prefix arms a pending state that the next keystroke completes,
 * advances or cancels, and that the timeout forgets.
 */
export function createKeyDispatcher(
  getHandlers: () => KeybindingHandlers,
  platform: Platform = shortcutPlatform(),
  getPaneCommands: PaneCommandsSource = paneCommands,
) {
  let pending: PendingChord | null = null
  const cancel = () => {
    if (pending) clearTimeout(pending.timer)
    pending = null
  }
  const arm = (candidates: PendingChord['candidates'], depth: number): void => {
    cancel()
    pending = {
      candidates,
      depth,
      timer: setTimeout(cancel, CHORD_TIMEOUT_MS),
    }
  }
  const standsDown = (
    definition: KeybindingDefinition,
    event: DispatchEvent,
    typing: boolean,
    inDialog: boolean,
  ): boolean =>
    (inDialog && !definition.firesWhileTyping) ||
    (typing &&
      !definition.firesWhileTyping &&
      !allowsWhileTyping(event.target, definition.id))

  const onKeyDown = (event: DispatchEvent): void => {
    // A shortcut must not fire off the keystroke that finishes a character
    // being composed by an IME, nor one a component already answered.
    if (event.isComposing || event.repeat || event.defaultPrevented) return
    const typing = isTyping(event.target)
    const inDialog = insideDialog(event.target)

    if (pending) {
      const { candidates, depth } = pending
      cancel()
      const advanced = candidates.filter((candidate) =>
        bindingMatchesEvent(candidate.chords[depth] ?? '', event, platform),
      )
      if (advanced.length > 0) {
        event.preventDefault()
        const complete = advanced.find(
          (candidate) => candidate.chords.length === depth + 1,
        )
        if (complete) {
          complete.run()
          return
        }
        arm(advanced, depth + 1)
        return
      }
      // Anything else ends the chord and is handled as an ordinary key.
    }

    const prefixed: ChordCandidate[] = []
    for (const definition of KEYBINDINGS) {
      if (definition.scope !== 'global') continue
      if (standsDown(definition, event, typing, inDialog)) continue
      const run = getHandlers()[definition.id]
      if (!run) continue
      if (definition.digitIndex) {
        const index = matchDigitIndex(definition.id, event, platform)
        if (index === null) continue
        event.preventDefault()
        run(index)
        return
      }
      if (matchesKeybinding(definition.id, event, platform)) {
        event.preventDefault()
        run(0)
        return
      }
      for (const chords of sequencesFor(definition.id, platform)) {
        if (bindingMatchesEvent(chords[0] ?? '', event, platform)) {
          prefixed.push({ chords, run: () => run(0) })
        }
      }
    }

    // The focused pane's own commands come after the console's keys, so a
    // page can never shadow a global chord, and only while focus is inside
    // that pane, so two panes of the same page never both answer.
    const paneId = paneRootOf(event.target)?.dataset.workspacePaneId
    if (paneId) {
      for (const entry of getPaneCommands(paneId)) {
        const { command } = entry
        if (inDialog && !command.firesWhileTyping) continue
        if (
          typing &&
          !command.firesWhileTyping &&
          !allowsWhileTyping(event.target, entry.key)
        )
          continue
        if (command.enabled?.() === false) continue
        for (const binding of entry.bindings) {
          if (isSequence(binding)) {
            const chords = splitSequence(binding)
            if (bindingMatchesEvent(chords[0] ?? '', event, platform)) {
              prefixed.push({ chords, run: command.run })
            }
            continue
          }
          if (bindingMatchesEvent(binding, event, platform)) {
            event.preventDefault()
            command.run()
            return
          }
        }
      }
    }

    if (prefixed.length > 0) {
      event.preventDefault()
      arm(prefixed, 1)
    }
  }
  return { onKeyDown, cancel }
}

export function useKeybindings(handlers: KeybindingHandlers): void {
  // Read through a ref so the listener binds once: handlers are rebuilt every
  // render by every caller that passes inline arrows.
  const handlersRef = useRef(handlers)
  handlersRef.current = handlers

  useEffect(() => {
    if (typeof window === 'undefined') return
    const dispatcher = createKeyDispatcher(() => handlersRef.current)
    const onKeyDown = (event: KeyboardEvent) => dispatcher.onKeyDown(event)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      dispatcher.cancel()
    }
  }, [])
}
