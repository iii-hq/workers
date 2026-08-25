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

  it('stands down inside a declared standdown surface', () => {
    const seen: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({ 'workspace.create': () => seen.push('create') }),
      'mac',
    )
    const editorChild = {
      tagName: 'DIV',
      dataset: {},
      closest: (selector: string) =>
        selector.includes('data-keybindings-standdown') ? {} : null,
    } as unknown as EventTarget
    dispatcher.onKeyDown(key('t', { target: editorChild }))
    expect(seen).toEqual([])
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

  describe('pane-scoped page commands', () => {
    const pane = (id: string) => {
      const root = { dataset: { workspacePaneId: id } }
      return {
        tagName: 'DIV',
        dataset: {},
        closest: (selector: string) =>
          selector.includes('workspace-pane-id') ? root : null,
      } as unknown as EventTarget
    }
    const entry = (
      paneId: string,
      binding: string,
      run: () => void,
      extra: Partial<{
        enabled: () => boolean
        firesWhileTyping: boolean
      }> = {},
    ) => ({
      key: `page.${binding}`,
      pageId: 'page',
      source: 'page' as const,
      paneId,
      bindings: [binding],
      command: { id: binding, title: binding, run, ...extra },
    })

    it('fires only while focus is inside the pane that registered it', () => {
      const seen: string[] = []
      const dispatcher = createKeyDispatcher(
        () => ({}),
        'mac',
        (paneId) =>
          paneId === 'pane-1'
            ? [entry('pane-1', 'P', () => seen.push('open'))]
            : [],
      )
      dispatcher.onKeyDown(key('p', { target: pane('pane-2') }))
      dispatcher.onKeyDown(key('p'))
      expect(seen).toEqual([])
      const inside = key('p', { target: pane('pane-1') })
      dispatcher.onKeyDown(inside)
      expect(seen).toEqual(['open'])
      expect(inside.defaultPrevented).toBe(true)
    })

    it('never shadows a console key and skips a disabled command', () => {
      const seen: string[] = []
      const dispatcher = createKeyDispatcher(
        () => ({ 'workspace.create': () => seen.push('create') }),
        'mac',
        () => [
          entry('pane-1', 'T', () => seen.push('page-t')),
          entry('pane-1', 'X', () => seen.push('page-x'), {
            enabled: () => false,
          }),
        ],
      )
      dispatcher.onKeyDown(key('t', { target: pane('pane-1') }))
      dispatcher.onKeyDown(key('x', { target: pane('pane-1') }))
      expect(seen).toEqual(['create'])
    })

    it('completes a page chord and stands down in a field unless asked', () => {
      const seen: string[] = []
      const dispatcher = createKeyDispatcher(
        () => ({}),
        'mac',
        () => [
          entry('pane-1', 'Q L', () => seen.push('line')),
          entry('pane-1', 'Escape', () => seen.push('stop'), {
            firesWhileTyping: true,
          }),
        ],
      )
      dispatcher.onKeyDown(key('q', { target: pane('pane-1') }))
      dispatcher.onKeyDown(key('l', { target: pane('pane-1') }))
      expect(seen).toEqual(['line'])

      const field = {
        tagName: 'TEXTAREA',
        dataset: {},
        closest: (selector: string) =>
          selector.includes('workspace-pane-id')
            ? { dataset: { workspacePaneId: 'pane-1' } }
            : null,
      } as unknown as EventTarget
      dispatcher.onKeyDown(key('q', { target: field }))
      dispatcher.onKeyDown(key('l', { target: field }))
      dispatcher.onKeyDown(key('Escape', { target: field }))
      expect(seen).toEqual(['line', 'stop'])
    })

    it('leaves a key a component already answered alone', () => {
      const seen: string[] = []
      const dispatcher = createKeyDispatcher(
        () => ({ 'workspace.create': () => seen.push('create') }),
        'mac',
      )
      dispatcher.onKeyDown(key('t', { defaultPrevented: true }))
      expect(seen).toEqual([])
    })
  })
})
