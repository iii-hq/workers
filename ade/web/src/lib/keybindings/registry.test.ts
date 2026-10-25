import { describe, expect, it } from 'vitest'
import {
  isBareKey,
  isBrowserReserved,
  type Platform,
  parseSequence,
} from './bindings'
import {
  bindingsFor,
  hoverTitle,
  KEYBINDINGS,
  keybinding,
  keybindingConflicts,
  keybindingGroups,
  matchDigitIndex,
  matchesKeybinding,
  resolveBindings,
  sequencesFor,
  shortcutClaimReason,
} from './registry'

const PLATFORMS: Platform[] = ['mac', 'other']

describe('the registry itself', () => {
  it('has no two shortcuts fighting over a chord, on either platform', () => {
    for (const platform of PLATFORMS) {
      expect(keybindingConflicts(platform)).toEqual([])
    }
  })

  it('binds nothing to a bare character key, on either platform', () => {
    for (const platform of PLATFORMS) {
      for (const definition of KEYBINDINGS) {
        for (const binding of resolveBindings(definition.bindings, platform)) {
          expect({
            id: definition.id,
            binding,
            bare: isBareKey(binding),
          }).toEqual({ id: definition.id, binding, bare: false })
        }
      }
    }
  })

  it('claims no chord the browser owns, on either platform', () => {
    for (const platform of PLATFORMS) {
      for (const definition of KEYBINDINGS) {
        for (const binding of resolveBindings(definition.bindings, platform)) {
          expect({
            id: definition.id,
            platform,
            binding,
            reserved: isBrowserReserved(binding, platform),
          }).toEqual({ id: definition.id, platform, binding, reserved: false })
        }
      }
    }
  })

  it('parses every binding it declares', () => {
    for (const platform of PLATFORMS) {
      for (const definition of KEYBINDINGS) {
        const bindings = resolveBindings(definition.bindings, platform)
        for (const binding of bindings) {
          expect({ binding, parsed: parseSequence(binding) !== null }).toEqual({
            binding,
            parsed: true,
          })
        }
      }
    }
  })

  it('gives every entry a title and a group to be listed under', () => {
    for (const definition of KEYBINDINGS) {
      expect(definition.title.length).toBeGreaterThan(0)
      expect(definition.group.length).toBeGreaterThan(0)
    }
  })

  it('uses each id once', () => {
    const ids = KEYBINDINGS.map((definition) => definition.id)
    expect(new Set(ids).size).toBe(ids.length)
  })
})

describe('lookup', () => {
  it('finds a definition by id', () => {
    expect(keybinding('palette.toggle').bindings).toEqual(['Mod+K'])
    expect(bindingsFor('workspace.close', 'mac')).toEqual(['Ctrl+W'])
    expect(keybinding('panel.split').title).toBe('Split panel right')
  })

  it('resolves a per-platform binding list', () => {
    expect(bindingsFor('palette.next', 'mac')).toEqual(['ArrowDown', 'Ctrl+N'])
    expect(bindingsFor('palette.next', 'other')).toEqual(['ArrowDown'])
  })

  it('lets the palette be reached from a field, and a workspace key not', () => {
    expect(keybinding('palette.toggle').firesWhileTyping).toBe(true)
    expect(keybinding('workspace.create').firesWhileTyping).toBeUndefined()
  })

  it('spells a hover title with the key, or plain text for an unbound action', () => {
    expect(hoverTitle('New workspace', 'workspace.create', 'mac')).toBe(
      'New workspace (⌃ T)',
    )
    expect(hoverTitle('Close', 'workspace.close', 'other')).toBe(
      'Close (alt W)',
    )
    expect(hoverTitle('Search', 'palette.toggle', 'other')).toBe(
      'Search (ctrl K)',
    )
    expect(hoverTitle('Chat', 'page.chat', 'mac')).toBe('Chat (⌃ ⌘ C)')
    expect(hoverTitle('Chat', 'page.chat', 'other')).toBe('Chat (alt shift C)')
    expect(hoverTitle('Panels', 'panel.next', 'mac')).toBe('Panels (⌃ })')
  })

  it('says why a page may not take a key, or lets it', () => {
    expect(shortcutClaimReason('Mod+K', 'other')).toMatch(
      /command palette|Search|palette/i,
    )
    expect(shortcutClaimReason('Mod+W', 'mac')).toBe('the browser owns it')
    expect(shortcutClaimReason('Hyper+Q', 'mac')).toBe('it does not parse')
    expect(shortcutClaimReason('Ctrl+T', 'mac')).toMatch(/New workspace/)
    expect(shortcutClaimReason('Ctrl+Cmd+C', 'mac')).toMatch(/Go to chat/)
    expect(shortcutClaimReason('Alt+C', 'mac')).toBeNull()
    expect(shortcutClaimReason('Ctrl+J', 'mac')).toBeNull()
    expect(shortcutClaimReason('Ctrl+`', 'other')).toBeNull()
    expect(shortcutClaimReason('Mod+Shift+P', 'mac')).toBeNull()
    expect(shortcutClaimReason('Ctrl+Q L', 'mac')).toBeNull()
    expect(shortcutClaimReason('Escape', 'mac')).toBeNull()
    expect(shortcutClaimReason('F2', 'other')).toBeNull()
  })

  it('refuses a key that types a character, on either platform', () => {
    for (const platform of PLATFORMS) {
      for (const binding of [
        't',
        '7',
        'P',
        '/',
        '`',
        'Shift+W',
        'G X',
        'Q L',
      ]) {
        expect([binding, shortcutClaimReason(binding, platform)]).toEqual([
          binding,
          'it has no modifier',
        ])
      }
    }
  })

  it('steps panel focus with the modifier braces', () => {
    expect(bindingsFor('panel.next', 'mac')).toEqual(['Ctrl+}'])
    expect(bindingsFor('panel.previous', 'other')).toEqual(['Alt+{'])
  })

  it('splits with the brackets, unshifted, and steps workspaces with the arrows', () => {
    expect(bindingsFor('panel.split', 'mac')).toEqual(['Ctrl+]'])
    expect(bindingsFor('panel.splitLeft', 'mac')).toEqual(['Ctrl+['])
    expect(bindingsFor('panel.splitLeft', 'other')).toEqual(['Alt+['])
    expect(hoverTitle('Split left', 'panel.splitLeft', 'mac')).toBe(
      'Split left (⌃ [)',
    )
    expect(bindingsFor('workspace.next', 'mac')).toEqual([
      'Ctrl+Shift+ArrowRight',
    ])
    expect(bindingsFor('workspace.previous', 'other')).toEqual([
      'Alt+Shift+ArrowLeft',
    ])
    expect(hoverTitle('Next', 'workspace.next', 'mac')).toBe('Next (⌃ ⇧ →)')
    // The bracket splits; its shifted twin, the brace, moves focus instead.
    const bracket = {
      key: ']',
      metaKey: false,
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
    }
    expect(matchesKeybinding('panel.split', bracket, 'mac')).toBe(true)
    expect(matchesKeybinding('panel.next', bracket, 'mac')).toBe(false)
    const brace = { ...bracket, key: '}', shiftKey: true }
    expect(matchesKeybinding('panel.next', brace, 'mac')).toBe(true)
    expect(matchesKeybinding('panel.split', brace, 'mac')).toBe(false)
    // An arrow keeps its name under Shift; without Shift it is not a chord.
    const arrow = { ...bracket, key: 'ArrowRight', shiftKey: true }
    expect(matchesKeybinding('workspace.next', arrow, 'mac')).toBe(true)
    expect(
      matchesKeybinding('workspace.next', { ...arrow, shiftKey: false }, 'mac'),
    ).toBe(false)
  })

  it('keeps every console shortcut to one press: no global sequences', () => {
    for (const platform of PLATFORMS) {
      for (const definition of KEYBINDINGS) {
        expect(sequencesFor(definition.id, platform)).toEqual([])
      }
    }
  })
})

describe('matchesKeybinding', () => {
  it('matches any of an action’s bindings', () => {
    const press = {
      key: 'n',
      metaKey: false,
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
    }
    expect(matchesKeybinding('palette.next', press, 'mac')).toBe(true)
    expect(
      matchesKeybinding(
        'palette.next',
        {
          key: 'ArrowDown',
          metaKey: false,
          ctrlKey: false,
          altKey: false,
          shiftKey: false,
        },
        'mac',
      ),
    ).toBe(true)
  })

  it('does not match a binding this platform does not carry', () => {
    const ctrlN = {
      key: 'n',
      metaKey: false,
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
    }
    expect(matchesKeybinding('palette.next', ctrlN, 'other')).toBe(false)
  })

  it('does not match an unrelated chord', () => {
    expect(
      matchesKeybinding(
        'palette.toggle',
        {
          key: 'k',
          metaKey: false,
          ctrlKey: false,
          altKey: false,
          shiftKey: false,
        },
        'mac',
      ),
    ).toBe(false)
  })
})

describe('matchDigitIndex', () => {
  function digit(key: string) {
    return {
      key,
      metaKey: false,
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
    }
  }

  it('selects by position through the modifier, never a bare digit', () => {
    expect(bindingsFor('workspace.selectByIndex', 'mac')).toEqual(['Ctrl+1'])
    for (let value = 1; value <= 9; value++) {
      expect(
        matchDigitIndex('workspace.selectByIndex', digit(String(value)), 'mac'),
      ).toBeNull()
      expect(
        matchDigitIndex(
          'workspace.selectByIndex',
          { ...digit(String(value)), ctrlKey: true },
          'mac',
        ),
      ).toBe(value - 1)
    }
  })

  it('ignores a keystroke that is not a digit, and zero', () => {
    expect(
      matchDigitIndex('workspace.selectByIndex', digit('0'), 'mac'),
    ).toBeNull()
    expect(
      matchDigitIndex('workspace.selectByIndex', digit('t'), 'mac'),
    ).toBeNull()
  })

  it('does not fire when a modifier is held', () => {
    expect(
      matchDigitIndex(
        'workspace.selectByIndex',
        { ...digit('1'), metaKey: true },
        'mac',
      ),
    ).toBeNull()
  })

  it('returns null for a shortcut that does not select by position', () => {
    expect(matchDigitIndex('palette.toggle', digit('1'), 'mac')).toBeNull()
  })

  it('reserves all nine digits, so nothing else may take one', () => {
    // Proven by the conflict check: a second shortcut on `5` would collide
    // with the row that stores `1`.
    const identities = keybindingConflicts('mac')
    expect(identities).toEqual([])
  })
})

describe('keybindingGroups', () => {
  it('groups without losing or duplicating an entry', () => {
    const grouped = keybindingGroups()
    expect(grouped.flatMap(([, entries]) => entries)).toHaveLength(
      KEYBINDINGS.length,
    )
  })

  it('keeps registry order inside a group', () => {
    const [, first] = keybindingGroups()[0]
    expect(first[0].id).toBe(KEYBINDINGS[0].id)
  })
})

describe('palette quick open', () => {
  it('opens the Nth row on ⌃ plus the digit on a Mac', () => {
    const controlThree = {
      key: '3',
      code: 'Digit3',
      metaKey: false,
      ctrlKey: true,
      altKey: false,
      shiftKey: false,
    }
    expect(matchDigitIndex('palette.pick', controlThree, 'mac')).toBe(2)
    expect(
      matchDigitIndex(
        'palette.pick',
        { ...controlThree, ctrlKey: false },
        'mac',
      ),
    ).toBeNull()
  })

  it('reads Alt+1–9 from the digit row elsewhere even when the layout turns it into a symbol', () => {
    const altThree = {
      key: '£',
      code: 'Digit3',
      metaKey: false,
      ctrlKey: false,
      altKey: true,
      shiftKey: false,
    }
    expect(matchDigitIndex('palette.pick', altThree, 'other')).toBe(2)
    expect(
      matchDigitIndex('palette.pick', { ...altThree, altKey: false }, 'other'),
    ).toBeNull()
  })
})
