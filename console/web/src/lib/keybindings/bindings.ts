/**
 * Binding strings, and the three things the console does with them: format one
 * for the reader's keyboard, test a key event against one, and decide whether
 * two of them collide.
 *
 * A binding is modifiers and a key joined by `+`: `Mod+K`, `Mod+Shift+P`,
 * `Escape`, `?`. `Mod` is the platform's primary modifier, so a binding is
 * written once and reads as ⌘ on a Mac and Ctrl everywhere else. Nothing here
 * knows what any binding DOES; that is the registry's job.
 */

export type Platform = 'mac' | 'other'

export type ParsedBinding = {
  mod: boolean
  meta: boolean
  ctrl: boolean
  alt: boolean
  shift: boolean
  key: string
}

const MODIFIERS = ['Mod', 'Cmd', 'Ctrl', 'Alt', 'Shift'] as const

/** Named keys, canonical token to the `KeyboardEvent.key` value it matches. */
const NAMED_KEYS: Record<string, string> = {
  Escape: 'Escape',
  Enter: 'Enter',
  Tab: 'Tab',
  Space: ' ',
  Backspace: 'Backspace',
  Delete: 'Delete',
  ArrowUp: 'ArrowUp',
  ArrowDown: 'ArrowDown',
  ArrowLeft: 'ArrowLeft',
  ArrowRight: 'ArrowRight',
}

/** How each token prints. Modifiers differ per platform; keys do not. */
const KEY_LABELS: Record<string, string> = {
  Escape: 'esc',
  Enter: '↵',
  Tab: 'tab',
  Space: 'space',
  Backspace: '⌫',
  Delete: 'del',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
}

export function shortcutPlatform(): Platform {
  if (typeof navigator === 'undefined') return 'other'
  return /Mac|iPhone|iPad|iPod/.test(navigator.userAgent) ? 'mac' : 'other'
}

/** A letter or digit is compared case-insensitively; anything else is a key
 *  value in its own right (`?`, `,`, `/`). */
function isAlphanumericToken(key: string): boolean {
  return /^[A-Za-z0-9]$/.test(key)
}

function isNamedToken(key: string): boolean {
  return key in NAMED_KEYS
}

/**
 * Punctuation carries its own shift state: `?` IS shift and slash on a US
 * layout and an unshifted key elsewhere, so comparing the shift flag would
 * make the binding layout-dependent. `Shift+/` stays available for anyone who
 * means the physical chord.
 */
function isPunctuationToken(key: string): boolean {
  return !isAlphanumericToken(key) && !isNamedToken(key)
}

export function parseBinding(binding: string): ParsedBinding | null {
  const tokens = binding.split('+').filter((token) => token !== '')
  const key = tokens.pop()
  if (!key) return null
  const parsed: ParsedBinding = {
    mod: false,
    meta: false,
    ctrl: false,
    alt: false,
    shift: false,
    key: isAlphanumericToken(key) ? key.toUpperCase() : key,
  }
  for (const token of tokens) {
    if (!(MODIFIERS as readonly string[]).includes(token)) return null
    if (token === 'Mod') parsed.mod = true
    if (token === 'Cmd') parsed.meta = true
    if (token === 'Ctrl') parsed.ctrl = true
    if (token === 'Alt') parsed.alt = true
    if (token === 'Shift') parsed.shift = true
  }
  // A modifier token in the key position (`Mod+Shift`) parses as a key named
  // `Shift`, which no event will ever carry. Refuse it outright.
  if ((MODIFIERS as readonly string[]).includes(key)) return null
  return parsed
}

/** `Mod` resolved: the modifier each platform's own menus use. */
function resolveModifiers(
  parsed: ParsedBinding,
  platform: Platform,
): { meta: boolean; ctrl: boolean; alt: boolean; shift: boolean } {
  const mac = platform === 'mac'
  return {
    meta: parsed.meta || (parsed.mod && mac),
    ctrl: parsed.ctrl || (parsed.mod && !mac),
    alt: parsed.alt,
    shift: parsed.shift,
  }
}

export type KeyEventLike = Pick<
  KeyboardEvent,
  'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'
>

export function bindingMatchesEvent(
  binding: string,
  event: KeyEventLike,
  platform: Platform = shortcutPlatform(),
): boolean {
  const parsed = parseBinding(binding)
  if (!parsed) return false
  const wanted = resolveModifiers(parsed, platform)
  if (wanted.meta !== event.metaKey) return false
  if (wanted.ctrl !== event.ctrlKey) return false
  if (wanted.alt !== event.altKey) return false
  // See isPunctuationToken: the key value already encodes the shift.
  if (!isPunctuationToken(parsed.key) && wanted.shift !== event.shiftKey) {
    return false
  }
  if (isAlphanumericToken(parsed.key)) {
    return event.key.toUpperCase() === parsed.key
  }
  if (isNamedToken(parsed.key)) return event.key === NAMED_KEYS[parsed.key]
  return event.key === parsed.key
}

/** One binding as the caps to print, outermost modifier first. */
export function formatBinding(
  binding: string,
  platform: Platform = shortcutPlatform(),
): string[] {
  const parsed = parseBinding(binding)
  if (!parsed) return [binding]
  const mac = platform === 'mac'
  const caps: string[] = []
  if (parsed.mod) caps.push(mac ? '⌘' : 'ctrl')
  if (parsed.meta) caps.push(mac ? '⌘' : 'meta')
  if (parsed.ctrl) caps.push(mac ? '⌃' : 'ctrl')
  if (parsed.alt) caps.push(mac ? '⌥' : 'alt')
  if (parsed.shift) caps.push(mac ? '⇧' : 'shift')
  caps.push(KEY_LABELS[parsed.key] ?? parsed.key.toLowerCase())
  return caps
}

/**
 * The chord two bindings collide on, with `Mod` resolved for the platform:
 * `Cmd+K` and `Mod+K` are the same chord on a Mac and different chords
 * everywhere else, so a conflict check has to be asked about a platform.
 */
export function conflictIdentity(binding: string, platform: Platform): string {
  const parsed = parseBinding(binding)
  if (!parsed) return binding
  const resolved = resolveModifiers(parsed, platform)
  return [
    resolved.meta ? 'meta' : '',
    resolved.ctrl ? 'ctrl' : '',
    resolved.alt ? 'alt' : '',
    // Punctuation ignores shift when matching, so it must ignore it here too,
    // or `?` and `Shift+?` would read as two free chords and both fire.
    resolved.shift && !isPunctuationToken(parsed.key) ? 'shift' : '',
    parsed.key,
  ].join('+')
}

/**
 * Chords the browser keeps for itself, as window and tab management.
 *
 * This is where a desktop app's shortcut table stops porting to a page: an
 * Electron shell can claim ⌘W or ⌘T before the web contents see them, and a
 * tab cannot. A binding listed here would show up in the shortcut overlay and
 * then do nothing, which is worse than having no shortcut at all, so the
 * registry's own test refuses one.
 *
 * Reservation is per-platform, because the chord is: `Ctrl+N` opens a window
 * on Windows and Linux and is an ordinary free chord on a Mac, where the same
 * menu item is ⌘N. Written with `Mod` so each entry covers whichever modifier
 * the platform's own menus use.
 */
export const BROWSER_RESERVED: readonly string[] = [
  'Mod+W',
  'Mod+T',
  'Mod+N',
  'Mod+Q',
  'Mod+Shift+W',
  'Mod+Shift+N',
  'Mod+Shift+T',
  // Tab selection by number, which is the shortcut a tabbed app reaches for
  // first and the one a page is least likely to receive.
  ...Array.from({ length: 9 }, (_, index) => `Mod+${index + 1}`),
]

/**
 * Reserved on macOS only, where the browser's own menus sit on Command and
 * the equivalent chord is free on Windows and Linux. `Mod+,` is the trap
 * worth naming: it is the obvious chord for a settings screen and it opens
 * Chrome's own preferences.
 */
export const MAC_RESERVED: readonly string[] = ['Mod+,', 'Mod+[', 'Mod+]']

export function isBrowserReserved(
  binding: string,
  platform: Platform,
): boolean {
  const reserved = [
    ...BROWSER_RESERVED,
    ...(platform === 'mac' ? MAC_RESERVED : []),
  ]
  const identities = new Set(
    reserved.map((entry) => conflictIdentity(entry, platform)),
  )
  return identities.has(conflictIdentity(binding, platform))
}

/** The digit a keystroke carries, for shortcuts that select by position. */
export function digitFromEvent(event: KeyEventLike): string | null {
  return /^[1-9]$/.test(event.key) ? event.key : null
}
