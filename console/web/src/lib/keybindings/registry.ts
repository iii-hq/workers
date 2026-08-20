/**
 * Every keyboard shortcut the console has, in one list.
 *
 * The point of a registry is that nothing else gets to hold this knowledge:
 * the shortcut overlay is generated from it, the command palette reads a row's
 * chord from it, and the global key handler dispatches from it. A shortcut
 * that is not here does not exist, and one that is here cannot be documented
 * wrong.
 *
 * `scope` says who dispatches, not who may press: a `palette` binding is live
 * only while the palette owns the keyboard, so it is documented here and
 * handled there. Only `global` reaches the window listener.
 */

import {
  bindingMatchesEvent,
  conflictIdentity,
  digitFromEvent,
  type KeyEventLike,
  type Platform,
  parseBinding,
  shortcutPlatform,
} from './bindings'

export type { Platform }

export type KeybindingScope = 'global' | 'palette'

export type KeybindingActionId =
  | 'palette.toggle'
  | 'shortcuts.open'
  | 'app.settings'
  | 'workspace.selectByIndex'
  | 'workspace.create'
  | 'panel.split'
  | 'palette.next'
  | 'palette.previous'
  | 'palette.cycleFilter'
  | 'palette.choose'
  | 'palette.close'
  | 'chat.previousTurn'
  | 'chat.nextTurn'

/**
 * One list when the chord is the same everywhere, or a chord per platform
 * when it cannot be: a Mac's free modifier is another platform's window
 * manager (see BROWSER_RESERVED).
 */
export type PlatformBindings =
  | readonly string[]
  | { readonly mac: readonly string[]; readonly other: readonly string[] }

export type KeybindingDefinition = {
  id: KeybindingActionId
  /** What the shortcut does, as the overlay says it. */
  title: string
  /** Heading it sits under in the overlay. */
  group: string
  scope: KeybindingScope
  bindings: PlatformBindings
  /**
   * Whether the shortcut still fires while the caret is in a field. Off by
   * default: a bare `?` typed into the composer is a question mark, not a
   * command. The palette is the deliberate exception, because it is how you
   * leave wherever you are.
   */
  firesWhileTyping?: boolean
  /** Extra words the palette should match on. */
  keywords?: readonly string[]
  /**
   * Selects by position: the binding stores one representative chord and the
   * shortcut fires for 1 through 9, so the overlay shows a range and no other
   * shortcut may take a digit out from under it.
   */
  digitIndex?: boolean
}

const DIGITS = ['1', '2', '3', '4', '5', '6', '7', '8', '9'] as const

export const KEYBINDINGS: readonly KeybindingDefinition[] = [
  {
    id: 'palette.toggle',
    title: 'Search the console',
    group: 'Console',
    scope: 'global',
    bindings: ['Mod+K'],
    firesWhileTyping: true,
    keywords: ['palette', 'command', 'find', 'jump'],
  },
  {
    id: 'shortcuts.open',
    title: 'Show keyboard shortcuts',
    group: 'Console',
    scope: 'global',
    bindings: ['?'],
    keywords: ['keys', 'help', 'shortcut'],
  },
  {
    id: 'chat.previousTurn',
    title: 'Previous turn in the chat',
    group: 'Chat',
    scope: 'global',
    bindings: ['['],
    keywords: ['turn', 'back', 'scroll', 'rail'],
  },
  {
    id: 'chat.nextTurn',
    title: 'Next turn in the chat',
    group: 'Chat',
    scope: 'global',
    bindings: [']'],
    keywords: ['turn', 'forward', 'scroll', 'rail'],
  },
  // The emacs pair is Mac-only on purpose: ctrl+N opens a browser window on
  // Windows and Linux, where the same menu item is ⌘N on a Mac and ctrl is
  // free. ctrl+P follows it rather than quietly eating Print on one platform
  // and meaning "previous" on the other.
  // Bare keys, not chords. The browser has already taken almost every
  // Mod+key worth having (see BROWSER_RESERVED and MAC_RESERVED), and a page
  // that fights it ships shortcuts that silently do nothing. Single keys are
  // free, they are what GitHub and Linear use for the same reason, and the
  // console already had one in `?`. None of them fire while you are typing.
  {
    id: 'workspace.selectByIndex',
    title: 'Select workspace 1 to 9',
    group: 'Workspace',
    scope: 'global',
    bindings: ['1'],
    digitIndex: true,
    keywords: ['tab', 'switch', 'workspace'],
  },
  {
    id: 'workspace.create',
    title: 'New workspace',
    group: 'Workspace',
    scope: 'global',
    bindings: ['t'],
    keywords: ['tab', 'new', 'create'],
  },
  {
    id: 'panel.split',
    title: 'Split the workspace',
    group: 'Workspace',
    scope: 'global',
    bindings: ['\\'],
    keywords: ['panel', 'column', 'split'],
  },
  {
    id: 'app.settings',
    title: 'Open settings',
    group: 'Console',
    scope: 'global',
    bindings: [','],
    keywords: ['configuration', 'preferences'],
  },
  {
    id: 'palette.next',
    title: 'Next result',
    group: 'Search',
    scope: 'palette',
    bindings: { mac: ['ArrowDown', 'Ctrl+N'], other: ['ArrowDown'] },
  },
  {
    id: 'palette.previous',
    title: 'Previous result',
    group: 'Search',
    scope: 'palette',
    bindings: { mac: ['ArrowUp', 'Ctrl+P'], other: ['ArrowUp'] },
  },
  {
    id: 'palette.cycleFilter',
    title: 'Cycle the filter',
    group: 'Search',
    scope: 'palette',
    bindings: ['Tab', 'Shift+Tab'],
  },
  {
    id: 'palette.choose',
    title: 'Open the selected row',
    group: 'Search',
    scope: 'palette',
    bindings: ['Enter'],
  },
  {
    id: 'palette.close',
    title: 'Close the palette',
    group: 'Search',
    scope: 'palette',
    bindings: ['Escape'],
  },
]

const BY_ID = new Map(
  KEYBINDINGS.map((definition) => [definition.id, definition]),
)

export function keybinding(id: KeybindingActionId): KeybindingDefinition {
  const definition = BY_ID.get(id)
  // The id type is closed, so a miss is a registry that lost an entry.
  if (!definition) throw new Error(`unknown keybinding: ${id}`)
  return definition
}

export function resolveBindings(
  bindings: PlatformBindings,
  platform: Platform,
): readonly string[] {
  return Array.isArray(bindings)
    ? bindings
    : (bindings as { mac: readonly string[]; other: readonly string[] })[
        platform
      ]
}

export function bindingsFor(
  id: KeybindingActionId,
  platform: Platform = shortcutPlatform(),
): readonly string[] {
  return resolveBindings(keybinding(id).bindings, platform)
}

export function matchesKeybinding(
  id: KeybindingActionId,
  event: KeyEventLike,
  platform: Platform = shortcutPlatform(),
): boolean {
  return bindingsFor(id, platform).some((binding) =>
    bindingMatchesEvent(binding, event, platform),
  )
}

/**
 * The index a keystroke selects, for a `digitIndex` shortcut: the stored
 * chord's modifiers with the pressed digit swapped in. Returns null when this
 * keystroke is not that shortcut.
 */
export function matchDigitIndex(
  id: KeybindingActionId,
  event: KeyEventLike,
  platform: Platform = shortcutPlatform(),
): number | null {
  if (!keybinding(id).digitIndex) return null
  const digit = digitFromEvent(event)
  if (!digit) return null
  for (const binding of bindingsFor(id, platform)) {
    const parsed = parseBinding(binding)
    if (!parsed || !DIGITS.includes(parsed.key as (typeof DIGITS)[number])) {
      continue
    }
    const candidate = binding.replace(/[1-9]$/, digit)
    if (bindingMatchesEvent(candidate, event, platform)) {
      return Number(digit) - 1
    }
  }
  return null
}

/** Overlay order: groups in first-seen order, entries in registry order. */
export function keybindingGroups(): Array<[string, KeybindingDefinition[]]> {
  const groups = new Map<string, KeybindingDefinition[]>()
  for (const definition of KEYBINDINGS) {
    const bucket = groups.get(definition.group)
    if (bucket) bucket.push(definition)
    else groups.set(definition.group, [definition])
  }
  return [...groups]
}

export type KeybindingConflict = {
  chord: string
  ids: KeybindingActionId[]
}

/**
 * Two shortcuts collide when they share a chord AND a scope: the palette's
 * `Escape` is free to be something else globally, because only one of them is
 * listening at a time.
 */
export function keybindingConflicts(platform: Platform): KeybindingConflict[] {
  const seen = new Map<string, KeybindingActionId[]>()
  for (const definition of KEYBINDINGS) {
    for (const binding of resolveBindings(definition.bindings, platform)) {
      // A digit-index row occupies all nine digits, not just the one it
      // stores, or something else could claim `5` and both would fire.
      const variants = definition.digitIndex
        ? DIGITS.map((digit) => binding.replace(/[1-9]$/, digit))
        : [binding]
      for (const variant of variants) {
        const chord = `${definition.scope}:${conflictIdentity(variant, platform)}`
        const bucket = seen.get(chord)
        if (bucket) bucket.push(definition.id)
        else seen.set(chord, [definition.id])
      }
    }
  }
  return [...seen]
    .filter(([, ids]) => ids.length > 1)
    .map(([chord, ids]) => ({ chord, ids }))
}
