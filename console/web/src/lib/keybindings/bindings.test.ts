import { describe, expect, it } from 'vitest'
import {
  bindingMatchesEvent,
  conflictIdentity,
  formatBinding,
  isBrowserReserved,
  isSequence,
  type KeyEventLike,
  parseBinding,
  parseSequence,
} from './bindings'

function press(
  key: string,
  modifiers: Partial<KeyEventLike> = {},
): KeyEventLike {
  return {
    key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...modifiers,
  }
}

describe('parseBinding', () => {
  it('reads modifiers and normalizes the key', () => {
    expect(parseBinding('Mod+Shift+k')).toMatchObject({
      mod: true,
      shift: true,
      key: 'K',
    })
  })

  it('keeps a named key as written', () => {
    expect(parseBinding('ArrowDown')?.key).toBe('ArrowDown')
  })

  it('refuses an unknown modifier', () => {
    expect(parseBinding('Hyper+K')).toBeNull()
  })

  it('refuses a binding with no key of its own', () => {
    expect(parseBinding('Mod+Shift')).toBeNull()
    expect(parseBinding('')).toBeNull()
  })
})

describe('bindingMatchesEvent', () => {
  it('resolves Mod per platform', () => {
    expect(
      bindingMatchesEvent('Mod+K', press('k', { metaKey: true }), 'mac'),
    ).toBe(true)
    expect(
      bindingMatchesEvent('Mod+K', press('k', { ctrlKey: true }), 'mac'),
    ).toBe(false)
    expect(
      bindingMatchesEvent('Mod+K', press('k', { ctrlKey: true }), 'other'),
    ).toBe(true)
    expect(
      bindingMatchesEvent('Mod+K', press('k', { metaKey: true }), 'other'),
    ).toBe(false)
  })

  it('ignores the case of a letter', () => {
    expect(
      bindingMatchesEvent('Mod+K', press('K', { metaKey: true }), 'mac'),
    ).toBe(true)
  })

  it('requires every modifier to match exactly', () => {
    expect(
      bindingMatchesEvent(
        'Mod+K',
        press('k', { metaKey: true, shiftKey: true }),
        'mac',
      ),
    ).toBe(false)
    expect(
      bindingMatchesEvent(
        'Mod+Shift+K',
        press('k', { metaKey: true, shiftKey: true }),
        'mac',
      ),
    ).toBe(true)
  })

  it('matches a bare key with no modifiers held', () => {
    expect(bindingMatchesEvent('Escape', press('Escape'), 'mac')).toBe(true)
    expect(
      bindingMatchesEvent('Escape', press('Escape', { metaKey: true }), 'mac'),
    ).toBe(false)
  })

  it('maps Space to the space key value', () => {
    expect(bindingMatchesEvent('Space', press(' '), 'mac')).toBe(true)
  })

  it('lets punctuation carry its own shift', () => {
    // `?` is shift and slash on a US layout and unshifted on others; both are
    // the same shortcut to the person pressing it.
    expect(
      bindingMatchesEvent('?', press('?', { shiftKey: true }), 'mac'),
    ).toBe(true)
    expect(bindingMatchesEvent('?', press('?'), 'mac')).toBe(true)
    // Other modifiers still have to be absent.
    expect(bindingMatchesEvent('?', press('?', { metaKey: true }), 'mac')).toBe(
      false,
    )
  })

  it('honours Shift when the binding names it', () => {
    // `?` is layout-independent; `Shift+/` is a physical chord and means it.
    expect(
      bindingMatchesEvent('Shift+/', press('/', { shiftKey: true }), 'mac'),
    ).toBe(true)
    expect(bindingMatchesEvent('Shift+/', press('/'), 'mac')).toBe(false)
  })

  it('does not match a different key', () => {
    expect(
      bindingMatchesEvent('Mod+K', press('j', { metaKey: true }), 'mac'),
    ).toBe(false)
  })
})

describe('formatBinding', () => {
  it('spells the chord for the platform', () => {
    expect(formatBinding('Mod+K', 'mac')).toEqual(['⌘', 'K'])
    expect(formatBinding('Mod+K', 'other')).toEqual(['ctrl', 'K'])
    expect(formatBinding('Mod+Shift+P', 'mac')).toEqual(['⌘', '⇧', 'P'])
  })

  it('prints a sequence chord by chord with "then" between', () => {
    expect(formatBinding('G C', 'mac')).toEqual(['G', 'then', 'C'])
    expect(formatBinding('G Shift+C', 'other')).toEqual([
      'G',
      'then',
      'shift',
      'C',
    ])
  })

  it('labels named keys', () => {
    expect(formatBinding('Escape', 'mac')).toEqual(['esc'])
    expect(formatBinding('ArrowUp', 'mac')).toEqual(['↑'])
  })

  it('returns the input when it cannot parse it', () => {
    expect(formatBinding('Hyper+K', 'mac')).toEqual(['Hyper+K'])
  })
})

describe('conflictIdentity', () => {
  it('collapses Mod and Cmd on a Mac, not elsewhere', () => {
    expect(conflictIdentity('Mod+K', 'mac')).toBe(
      conflictIdentity('Cmd+K', 'mac'),
    )
    expect(conflictIdentity('Mod+K', 'other')).not.toBe(
      conflictIdentity('Cmd+K', 'other'),
    )
  })

  it('treats punctuation as one chord however it is shifted', () => {
    // Not the matching rule, on purpose: `?` matches shifted or not, so it
    // overlaps `Shift+?`. Registering both would fire both on one keystroke,
    // which is exactly what the conflict check exists to refuse.
    expect(conflictIdentity('?', 'mac')).toBe(
      conflictIdentity('Shift+?', 'mac'),
    )
  })

  it('keeps distinct chords distinct', () => {
    expect(conflictIdentity('Mod+K', 'mac')).not.toBe(
      conflictIdentity('Mod+Shift+K', 'mac'),
    )
  })
})

describe('isBrowserReserved', () => {
  it('flags the chords a page cannot count on receiving', () => {
    expect(isBrowserReserved('Mod+W', 'mac')).toBe(true)
    expect(isBrowserReserved('Mod+T', 'other')).toBe(true)
    expect(isBrowserReserved('Mod+1', 'mac')).toBe(true)
    // Print: delivered to the page, but not ours to take.
    expect(isBrowserReserved('Mod+P', 'mac')).toBe(true)
    expect(isBrowserReserved('Mod+P', 'other')).toBe(true)
    // Same chord written the platform-specific way.
    expect(isBrowserReserved('Cmd+W', 'mac')).toBe(true)
    expect(isBrowserReserved('Ctrl+W', 'other')).toBe(true)
  })

  it('reserves per platform, because the chord is per platform', () => {
    // ctrl+N opens a window on Windows and Linux; on a Mac that menu item is
    // ⌘N and ctrl+N is free for an app to take.
    expect(isBrowserReserved('Ctrl+N', 'other')).toBe(true)
    expect(isBrowserReserved('Ctrl+N', 'mac')).toBe(false)
  })

  it('reserves the Mac-only menu chords', () => {
    // The obvious chord for a settings screen opens Chrome's preferences.
    expect(isBrowserReserved('Mod+,', 'mac')).toBe(true)
    expect(isBrowserReserved('Mod+,', 'other')).toBe(false)
  })

  it('leaves bare keys alone, which is why the console uses them', () => {
    for (const binding of ['1', 't', ',', '\\', '?']) {
      expect({ binding, mac: isBrowserReserved(binding, 'mac') }).toEqual({
        binding,
        mac: false,
      })
      expect({ binding, other: isBrowserReserved(binding, 'other') }).toEqual({
        binding,
        other: false,
      })
    }
  })

  it('leaves ordinary chords alone', () => {
    expect(isBrowserReserved('Mod+K', 'mac')).toBe(false)
    expect(isBrowserReserved('?', 'mac')).toBe(false)
    expect(isBrowserReserved('Mod+Shift+K', 'other')).toBe(false)
  })
})

describe('sequences', () => {
  it('splits and parses every chord', () => {
    expect(isSequence('G C')).toBe(true)
    expect(isSequence('Mod+K')).toBe(false)
    expect(parseSequence('G C')?.map((chord) => chord.key)).toEqual(['G', 'C'])
    expect(parseSequence('G Hyper+C')).toBeNull()
    expect(parseBinding('G C')).toBeNull()
  })

  it('never matches a whole sequence against one keystroke', () => {
    expect(bindingMatchesEvent('G C', press('c'), 'mac')).toBe(false)
  })

  it('is reserved when any chord of it is', () => {
    expect(isBrowserReserved('G Mod+W', 'mac')).toBe(true)
    expect(isBrowserReserved('G C', 'mac')).toBe(false)
  })

  it('has an identity per chord', () => {
    expect(conflictIdentity('G C', 'mac')).toBe('++++G ++++C')
  })
})
