import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  allowsWhileTyping,
  CHORD_TIMEOUT_MS,
  createKeyDispatcher,
  type DispatchEvent,
} from './use-keybindings'

function key(
  k: string,
  extra: Partial<DispatchEvent> = {},
): DispatchEvent & { defaultPrevented: boolean } {
  const event = {
    key: k,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    repeat: false,
    target: null,
    defaultPrevented: false,
    preventDefault() {
      event.defaultPrevented = true
    },
    ...extra,
  }
  return event
}

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

describe('createKeyDispatcher', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('fires a single chord and a digit index', () => {
    const seen: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({
        'workspace.create': () => seen.push('create'),
        'workspace.selectByIndex': (index) => seen.push(`select ${index}`),
      }),
      'mac',
    )
    dispatcher.onKeyDown(key('t'))
    dispatcher.onKeyDown(key('3'))
    expect(seen).toEqual(['create', 'select 2'])
  })

  it('completes a go-to chord and swallows its prefix', () => {
    const seen: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({
        'page.chat': () => seen.push('chat'),
        'workspace.create': () => seen.push('create'),
      }),
      'mac',
    )
    const prefix = key('g')
    dispatcher.onKeyDown(prefix)
    expect(prefix.defaultPrevented).toBe(true)
    expect(seen).toEqual([])
    dispatcher.onKeyDown(key('c'))
    expect(seen).toEqual(['chat'])
  })

  it('lets a pending prefix take a letter that is also a single key', () => {
    const seen: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({
        'page.traces': () => seen.push('traces'),
        'workspace.create': () => seen.push('create'),
      }),
      'mac',
    )
    dispatcher.onKeyDown(key('g'))
    dispatcher.onKeyDown(key('t'))
    expect(seen).toEqual(['traces'])
    dispatcher.onKeyDown(key('t'))
    expect(seen).toEqual(['traces', 'create'])
  })

  it('forgets the prefix after the timeout or an unrelated key', () => {
    const seen: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({
        'page.chat': () => seen.push('chat'),
        'workspace.create': () => seen.push('create'),
      }),
      'mac',
    )
    dispatcher.onKeyDown(key('g'))
    vi.advanceTimersByTime(CHORD_TIMEOUT_MS + 1)
    dispatcher.onKeyDown(key('c'))
    expect(seen).toEqual([])

    dispatcher.onKeyDown(key('g'))
    dispatcher.onKeyDown(key('t'))
    expect(seen).toEqual(['create'])
    dispatcher.onKeyDown(key('c'))
    expect(seen).toEqual(['create'])
  })

  it('never arms a chord from a field or a dialog', () => {
    const seen: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({ 'page.chat': () => seen.push('chat') }),
      'mac',
    )
    const input = { tagName: 'INPUT', dataset: {} } as unknown as EventTarget
    const prefix = key('g', { target: input })
    dispatcher.onKeyDown(prefix)
    expect(prefix.defaultPrevented).toBe(false)
    dispatcher.onKeyDown(key('c'))
    expect(seen).toEqual([])
  })
})
