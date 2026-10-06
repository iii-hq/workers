import type { Host } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@iii-workers/terminal-font', () => ({
  useTerminalFontSize: () => [13, () => undefined],
}))

// The ide/xterm.js console:module, as far as the mount effect uses it.
const built: Array<{ focused: boolean }> = []
const disposable = () => ({ dispose: () => undefined })
class Terminal {
  options: unknown
  focused = false
  buffer = { active: { viewportY: 0, baseY: 0 } }
  onData = disposable
  onBinary = disposable
  onResize = disposable
  onScroll = disposable
  constructor(options: unknown) {
    this.options = options
    built.push(this)
  }
  loadAddon() {}
  open() {}
  write() {}
  focus() {
    this.focused = true
  }
  dispose() {}
}
const xterm = {
  Terminal,
  FitAddon: class {
    fit() {}
  },
}

// The emulator is imported once for the page's life: each test is a page of
// its own, importing these afresh. A mock outlives vi.resetModules, so react's
// is made again over the fresh bare-hooks that `mount` comes from.
let mount: typeof import('./bare-hooks').mount
let useTerminalSession: typeof import('../terminal-session').useTerminalSession
let createTerminalOutputRouter: typeof import('../terminal-output-router').createTerminalOutputRouter
let createTerminalConnectionCoordinator: typeof import('../terminal-session-state').createTerminalConnectionCoordinator

const container = {
  appendChild: () => undefined,
  getBoundingClientRect: () => ({ width: 0, height: 0 }),
} as never

beforeEach(async () => {
  vi.resetModules()
  vi.doMock('react', async (original) => {
    const { hooks } = await import('./bare-hooks')
    return {
      ...(await original<typeof import('react')>()),
      ...hooks,
      // bare-hooks has no reducer: the session state's, over its useState.
      useReducer<S, A>(reduce: (last: S, action: A) => S, arg: unknown, init: (arg: unknown) => S) {
        const [state, set] = hooks.useState(() => init(arg)) as [S, (next: (last: S) => S) => void]
        return [state, (action: A) => set((last: S) => reduce(last, action))]
      },
    }
  })
  ;({ mount } = await import('./bare-hooks'))
  ;({ useTerminalSession } = await import('../terminal-session'))
  ;({ createTerminalOutputRouter } = await import('../terminal-output-router'))
  ;({ createTerminalConnectionCoordinator } = await import('../terminal-session-state'))
  vi.useFakeTimers()
  built.length = 0
  vi.stubGlobal('window', {
    setTimeout: (run: () => void, ms: number) => setTimeout(run, ms),
    clearTimeout: (id: number) => clearTimeout(id),
    setInterval: () => 0,
    clearInterval: () => undefined,
    requestAnimationFrame: (run: () => void) => {
      run()
      return 1
    },
    cancelAnimationFrame: () => undefined,
    getComputedStyle: () => ({ getPropertyValue: () => '' }),
  })
  vi.stubGlobal('document', {
    activeElement: null,
    body: {},
    documentElement: {},
    createElement: () => ({}),
  })
  class Observer {
    observe() {}
    disconnect() {}
  }
  vi.stubGlobal('ResizeObserver', Observer)
  vi.stubGlobal('MutationObserver', Observer)
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

/** A pane with its container attached; closing, so it opens no shell. */
function mountPane(importModule?: () => Promise<unknown>) {
  const host = {
    iii: {
      browserId: 'console-test',
      on: () => () => undefined,
      addConnectionStateListener: () => () => undefined,
      trigger: async () => undefined,
    },
    importModule,
  } as unknown as Host
  const coordinator = createTerminalConnectionCoordinator(() => 'request-1')
  coordinator.closing = true
  const pane = mount(useTerminalSession, {
    paneId: 'pane-1',
    root: '/repo',
    visible: true,
    router: createTerminalOutputRouter(host),
    leaseStore: null,
    storageKey: 'xterm-load',
    connectionCoordinator: coordinator,
  })
  pane.result.setContainer(container)
  return pane
}

describe('the lazily loaded terminal emulator', () => {
  it('shows a failed import and mounts xterm when the retry lands', async () => {
    const importModule = vi.fn().mockRejectedValueOnce(new Error('404')).mockResolvedValueOnce(xterm)
    const pane = mountPane(importModule)

    await vi.advanceTimersByTimeAsync(0)
    expect(pane.result.error).toBe('Terminal failed to load: 404')
    expect(built).toHaveLength(0)

    await vi.advanceTimersByTimeAsync(1_000)
    expect(importModule).toHaveBeenCalledTimes(2)
    expect(pane.result.error).toBeNull()
    expect(built).toHaveLength(1)
    expect(built[0].focused).toBe(true)
    pane.unmount()
  })

  it('builds no terminal for a pane gone before the import lands', async () => {
    const pane = mountPane(async () => xterm)
    pane.unmount()

    await vi.advanceTimersByTimeAsync(0)
    expect(built).toHaveLength(0)
  })

  it('names a console too old to load it', async () => {
    const pane = mountPane()

    await vi.advanceTimersByTimeAsync(60_000)
    expect(pane.result.error).toMatch(/predates lazy modules/)
    expect(built).toHaveLength(0)
    pane.unmount()
  })
})
