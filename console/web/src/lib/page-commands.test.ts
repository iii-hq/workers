import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getPageCommands,
  paneCommands,
  registerPageCommands,
  resetPageCommands,
} from './page-commands'

const noop = () => {}

describe('page commands', () => {
  beforeEach(() => {
    resetPageCommands()
    vi.spyOn(console, 'warn').mockImplementation(noop)
  })
  afterEach(() => vi.restoreAllMocks())

  it('namespaces ids by page and removes exactly what it registered', () => {
    const off = registerPageCommands(
      {
        pageId: 'shell',
        source: 'page',
        paneId: 'pane-1',
        commands: [
          { id: 'open', title: 'Open file', shortcut: 'P', run: noop },
          { id: 'find', title: 'Search', shortcut: 'F', run: noop },
        ],
      },
      'mac',
    )
    registerPageCommands(
      {
        pageId: 'database',
        source: 'worker',
        commands: [{ id: 'query', title: 'Run a query', run: noop }],
      },
      'mac',
    )
    expect(getPageCommands().map((entry) => entry.key)).toEqual([
      'shell.open',
      'shell.find',
      'database.query',
    ])
    off()
    off()
    expect(getPageCommands().map((entry) => entry.key)).toEqual([
      'database.query',
    ])
  })

  it('keeps keys for a mounted page only, never for a worker-level row', () => {
    registerPageCommands(
      {
        pageId: 'shell',
        source: 'worker',
        commands: [
          { id: 'open', title: 'Open file', shortcut: 'P', run: noop },
        ],
      },
      'mac',
    )
    registerPageCommands(
      {
        pageId: 'shell',
        source: 'page',
        paneId: 'pane-1',
        commands: [
          { id: 'open', title: 'Open file', shortcut: 'P', run: noop },
        ],
      },
      'mac',
    )
    const [worker, page] = getPageCommands()
    expect(worker.bindings).toEqual([])
    expect(page.bindings).toEqual(['P'])
    expect(paneCommands('pane-1').map((entry) => entry.key)).toEqual([
      'shell.open',
    ])
    expect(paneCommands('pane-2')).toEqual([])
  })

  it('refuses the palette chord and browser keys, keeps the freed keys', () => {
    registerPageCommands(
      {
        pageId: 'shell',
        source: 'page',
        paneId: 'pane-1',
        commands: [
          { id: 'a', title: 'a', shortcut: 't', run: noop },
          { id: 'b', title: 'b', shortcut: '5', run: noop },
          { id: 'c', title: 'c', shortcut: 'G L', run: noop },
          { id: 'd', title: 'd', shortcut: 'Mod+K', run: noop },
          { id: 'e', title: 'e', shortcut: 'Mod+W', run: noop },
          {
            id: 'f',
            title: 'f',
            shortcut: { mac: ['P'], other: ['Q'] },
            run: noop,
          },
        ],
      },
      'mac',
    )
    expect(
      getPageCommands().map((entry) => [entry.command.id, entry.bindings]),
    ).toEqual([
      ['a', ['t']],
      ['b', ['5']],
      ['c', ['G L']],
      ['d', []],
      ['e', []],
      ['f', ['P']],
    ])
    expect(console.warn).toHaveBeenCalledTimes(2)
  })

  it('lets a re-registration from the same pane replace the older one', () => {
    registerPageCommands(
      {
        pageId: 'chat',
        source: 'page',
        paneId: 'pane-1',
        commands: [
          { id: 'stop', title: 'Stop', enabled: () => false, run: noop },
        ],
      },
      'mac',
    )
    registerPageCommands(
      {
        pageId: 'chat',
        source: 'page',
        paneId: 'pane-1',
        commands: [
          { id: 'stop', title: 'Stop', enabled: () => true, run: noop },
        ],
      },
      'mac',
    )
    const entries = getPageCommands()
    expect(entries).toHaveLength(1)
    expect(entries[0].command.enabled?.()).toBe(true)
  })
})
