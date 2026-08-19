import { describe, expect, it } from 'vitest'
import { allowsWhileTyping } from './use-keybindings'

/** The guard reads one dataset entry, so a field stands in for the element. */
function field(allow?: string): EventTarget {
  return {
    dataset: allow === undefined ? {} : { keybindingsAllow: allow },
  } as unknown as EventTarget
}

describe('allowsWhileTyping', () => {
  it('hands back only the actions the field names', () => {
    const input = field('workspace.selectByIndex panel.split')

    expect(allowsWhileTyping(input, 'workspace.selectByIndex')).toBe(true)
    expect(allowsWhileTyping(input, 'panel.split')).toBe(true)
    // The workspace key is a letter, so this box still spells its own query.
    expect(allowsWhileTyping(input, 'workspace.create')).toBe(false)
  })

  it('accepts a comma-separated list as readily as a spaced one', () => {
    expect(
      allowsWhileTyping(field('panel.split,app.settings'), 'app.settings'),
    ).toBe(true)
  })

  it('keeps every key for a field that opts into nothing', () => {
    expect(allowsWhileTyping(field(), 'panel.split')).toBe(false)
    expect(allowsWhileTyping(field(''), 'panel.split')).toBe(false)
    expect(allowsWhileTyping(null, 'panel.split')).toBe(false)
  })

  it('does not match an action whose id merely starts the same way', () => {
    expect(allowsWhileTyping(field('panel.splitter'), 'panel.split')).toBe(
      false,
    )
  })
})
