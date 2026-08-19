import { describe, expect, it } from 'vitest'
import { isBrowserReserved, type Platform, parseBinding } from './bindings'
import {
  bindingsFor,
  KEYBINDINGS,
  keybinding,
  keybindingConflicts,
  keybindingGroups,
  matchesKeybinding,
  resolveBindings,
} from './registry'

const PLATFORMS: Platform[] = ['mac', 'other']

describe('the registry itself', () => {
  it('has no two shortcuts fighting over a chord, on either platform', () => {
    for (const platform of PLATFORMS) {
      expect(keybindingConflicts(platform)).toEqual([])
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
        expect(bindings.length).toBeGreaterThan(0)
        for (const binding of bindings) {
          expect({ binding, parsed: parseBinding(binding) !== null }).toEqual({
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
    expect(bindingsFor('shortcuts.open', 'mac')).toEqual(['?'])
  })

  it('resolves a per-platform binding list', () => {
    expect(bindingsFor('palette.next', 'mac')).toEqual(['ArrowDown', 'Ctrl+N'])
    expect(bindingsFor('palette.next', 'other')).toEqual(['ArrowDown'])
  })

  it('lets the palette be reached from a field, and the overlay not', () => {
    expect(keybinding('palette.toggle').firesWhileTyping).toBe(true)
    expect(keybinding('shortcuts.open').firesWhileTyping).toBeUndefined()
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
